use std::{collections::hash_map, sync::OnceLock};

use smallvec::{smallvec, SmallVec};

use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_interface::interface::Compiler;
use rustc_middle::{
    mir::{BasicBlock, Body, CastKind, Rvalue, StatementKind, TerminatorKind, START_BLOCK},
    traits::util::supertraits,
    ty::{
        adjustment::PointerCoercion, EarlyBinder, GenericArg, Instance, InstanceDef, List,
        ParamEnv, Ty, TyCtxt, TyKind, TypeAndMut, VtblEntry,
    },
};
use rustc_span::Symbol;

// === Driver === //

pub struct AnalyzerConfig {}

impl AnalyzerConfig {
    pub fn analyze(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) {
        // Only run our analysis if this binary has an entry point.
        let Some((main_fn, _)) = tcx.entry_fn(()) else {
            return;
        };

        // Run the analysis and let the compiler take over when we're done.
        let mut collect_analyzer = CollectAnalyzer::new(tcx);
        collect_analyzer.collect_dyn(Instance::mono(tcx, main_fn));

        let mut fact_analyzer = FactAnalyzer::new(&collect_analyzer);
        fact_analyzer.analyze(main_fn);
    }
}

// === CollectAnalyzer === //

pub struct CollectAnalyzer<'tcx> {
    tcx: TyCtxt<'tcx>,

    // The set of instances visited during collection.
    visited_fns_during_collection: FxHashSet<Instance<'tcx>>,

    // Maps from function pointers to specific function instances.
    fn_impls: FxHashMap<Ty<'tcx>, FxHashSet<Instance<'tcx>>>,

    // Maps from trait methods to specific implementations.
    trait_impls: FxHashMap<(DefId, &'tcx List<GenericArg<'tcx>>), FxHashSet<Instance<'tcx>>>,
}

impl<'tcx> CollectAnalyzer<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            visited_fns_during_collection: Default::default(),
            fn_impls: Default::default(),
            trait_impls: Default::default(),
        }
    }

    /// Collects dynamic dispatch targets reachable from `my_body_id`.
    pub fn collect_dyn(&mut self, my_body_id: Instance<'tcx>) {
        // Ensure that we haven't analyzed this function yet.
        if !self.visited_fns_during_collection.insert(my_body_id) {
            return;
        }

        // Ignore dispatches into `__autoken_assume_black_box`.
        if self
            .tcx
            .opt_item_name(my_body_id.def_id())
            .is_some_and(|name| name == sym::__autoken_assume_black_box.get())
        {
            return;
        }

        // Grab the MIR for this instance. If it's an unavailable dynamic dispatch shim, just ignore
        // it because we handle dynamic dispatch ourselves.
        let MirGrabResult::Found(my_body) = safeishly_grab_instance_mir(self.tcx, my_body_id.def)
        else {
            return;
        };

        for my_block in my_body.basic_blocks.iter() {
            // Look for coercions which can introduce new dynamic callees.
            for stmt in &my_block.statements {
                let StatementKind::Assign(stmt) = &stmt.kind else {
                    continue;
                };
                let (_place, rvalue) = &**stmt;

                let Rvalue::Cast(CastKind::PointerCoercion(kind), from_op, to_ty) = rvalue else {
                    continue;
                };

                let from_ty = my_body_id.subst_mir_and_normalize_erasing_regions(
                    self.tcx,
                    ParamEnv::reveal_all(),
                    EarlyBinder::bind(from_op.ty(&my_body.local_decls, self.tcx)),
                );

                let to_ty = my_body_id.subst_mir_and_normalize_erasing_regions(
                    self.tcx,
                    ParamEnv::reveal_all(),
                    EarlyBinder::bind(*to_ty),
                );

                match kind {
                    PointerCoercion::ReifyFnPointer => {
                        let TyKind::FnDef(def, generics) = from_ty.kind() else {
                            unreachable!()
                        };

                        let instance = Instance::expect_resolve(
                            self.tcx,
                            ParamEnv::reveal_all(),
                            *def,
                            generics,
                        );

                        self.fn_impls.entry(to_ty).or_default().insert(instance);

                        self.collect_dyn(instance);
                    }
                    PointerCoercion::ClosureFnPointer(_) => {
                        let TyKind::Closure(def, generics) = from_ty.kind() else {
                            unreachable!()
                        };

                        let instance = Instance::expect_resolve(
                            self.tcx,
                            ParamEnv::reveal_all(),
                            *def,
                            generics,
                        );

                        self.fn_impls.entry(to_ty).or_default().insert(instance);

                        self.collect_dyn(instance);
                    }
                    PointerCoercion::Unsize => {
                        // Finds the type the coercion actually changed.
                        let (from_ty, to_ty) = get_unsized_ty(self.tcx, from_ty, to_ty);

                        // Ensures that we're analyzing a dynamic type unsizing coercion.
                        let TyKind::Dynamic(binders, ..) = to_ty.kind() else {
                            continue;
                        };

                        // Extract the principal non-auto-type from the dynamic type.
                        let Some(binder) = binders.principal() else {
                            continue;
                        };

                        // Do some magic with binders... I guess.
                        let base_binder =
                            self.tcx.erase_regions(binder.with_self_ty(self.tcx, to_ty));

                        for binder in supertraits(self.tcx, base_binder) {
                            let trait_id = self.tcx.erase_late_bound_regions(binder);

                            // Get the actual methods which make up the trait's vtable since those are
                            // the things we can actually call.
                            let vtable_entries = self.tcx.vtable_entries(binder);

                            for vtable_entry in vtable_entries {
                                let VtblEntry::Method(vtbl_method) = vtable_entry else {
                                    continue;
                                };

                                // Now, get the concrete implementation of this method.
                                let concrete = Instance::expect_resolve(
                                    self.tcx,
                                    ParamEnv::reveal_all(),
                                    vtbl_method.def_id(),
                                    self.tcx.mk_args(
                                        [GenericArg::from(from_ty)]
                                            .into_iter()
                                            .chain(trait_id.args.iter().skip(1))
                                            .collect::<Vec<_>>()
                                            .as_slice(),
                                    ),
                                );

                                // Add it to the set and recurse into it to ensure its latent dynamic
                                // coercions are also captured.
                                self.trait_impls
                                    .entry((vtbl_method.def_id(), vtbl_method.args))
                                    .or_default()
                                    .insert(concrete);

                                self.collect_dyn(concrete);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Recurse into the things this block could call.
            match &my_block.terminator.as_ref().unwrap().kind {
                TerminatorKind::Call { func, .. } => {
                    let func = func.ty(&my_body.local_decls, self.tcx);
                    let func = my_body_id.subst_mir_and_normalize_erasing_regions(
                        self.tcx,
                        ParamEnv::reveal_all(),
                        EarlyBinder::bind(func),
                    );

                    // We can ignore function pointers since they are visited in a different way.
                    let TyKind::FnDef(callee_id, generics) = func.kind() else {
                        continue;
                    };

                    let callee_id = Instance::expect_resolve(
                        self.tcx,
                        ParamEnv::reveal_all(),
                        *callee_id,
                        generics,
                    );

                    self.collect_dyn(callee_id);
                }
                TerminatorKind::Drop { place, .. } => {
                    let place = place.ty(&my_body.local_decls, self.tcx).ty;
                    let place = my_body_id.subst_mir_and_normalize_erasing_regions(
                        self.tcx,
                        ParamEnv::reveal_all(),
                        EarlyBinder::bind(place),
                    );

                    let Some(dtor) = place
                        .needs_drop(self.tcx, ParamEnv::reveal_all())
                        .then(|| Instance::resolve_drop_in_place(self.tcx, place))
                    else {
                        continue;
                    };

                    self.collect_dyn(dtor);
                }
                _ => {}
            }
        }
    }
}

// === FactAnalyzer === //

pub struct FactAnalyzer<'cl, 'tcx> {
    tcx: TyCtxt<'tcx>,

    // Reference to an identically-named field in `CollectAnalyzer`
    fn_impls: &'cl FxHashMap<Ty<'tcx>, FxHashSet<Instance<'tcx>>>,

    // Reference to an identically-named field in `CollectAnalyzer`
    trait_impls: &'cl FxHashMap<(DefId, &'tcx List<GenericArg<'tcx>>), FxHashSet<Instance<'tcx>>>,

    // Stores analysis facts about each analyzed function monomorphization.
    fn_facts: FxHashMap<AnalysisSubject<'tcx>, MaybeFunctionFacts<'tcx>>,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
enum AnalysisSubject<'tcx> {
    Instance(Instance<'tcx>),
    FnPtr(Ty<'tcx>),
}

#[derive(Debug)]
enum MaybeFunctionFacts<'tcx> {
    Pending { my_depth: u32 },
    Done(FactMap<'tcx, FunctionFacts>),
}

type FactMap<'tcx, T> = FxHashMap<Ty<'tcx>, T>;

type FunctionFactsMap<'tcx> = FactMap<'tcx, FunctionFacts>;

type LeakFactsMap<'tcx> = FactMap<'tcx, LeakFacts>;

#[derive(Debug, Copy, Clone)]
struct FunctionFacts {
    max_enter_ref: i32,
    max_enter_mut: i32,
    mutably_borrows: bool,
    leaks: LeakFacts,
}

impl Default for FunctionFacts {
    fn default() -> Self {
        Self {
            max_enter_ref: i32::MAX,
            max_enter_mut: i32::MAX,
            mutably_borrows: false,
            leaks: LeakFacts::default(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
struct LeakFacts {
    leaked_muts: i32,
    leaked_refs: i32,
}

impl<'cl, 'tcx> FactAnalyzer<'cl, 'tcx> {
    pub fn new(collect_analyzer: &'cl CollectAnalyzer<'tcx>) -> Self {
        Self {
            tcx: collect_analyzer.tcx,
            fn_impls: &collect_analyzer.fn_impls,
            trait_impls: &collect_analyzer.trait_impls,
            fn_facts: FxHashMap::default(),
        }
    }

    /// Analyzes every function which is reachable from `body_id`.
    pub fn analyze(&mut self, body_id: DefId) {
        let _ = self.analyze_single(
            0,
            AnalysisSubject::Instance(Instance::mono(self.tcx, body_id)),
        );
    }

    /// Attempts to discover the facts about the provided function.
    ///
    /// Returns the inclusive depth of the lowest function on the stack we were able able to cycle
    /// back into or `u32::MAX` if the target never called a function which was already being analyzed.
    #[must_use]
    fn analyze_single(&mut self, my_depth: u32, my_body_id: AnalysisSubject<'tcx>) -> u32 {
        // Create a blank fact entry for us. If a facts entry already exists, handle it as either a
        // cycle or a memoized result.
        match self.fn_facts.entry(my_body_id) {
            hash_map::Entry::Occupied(entry) => {
                return match entry.get() {
                    // This may not actually be the true depth of the lowest function we could cycle
                    // back into but that will be discovered during the actual call to `analyze_inner`
                    // on our given `body_id`.
                    MaybeFunctionFacts::Pending { my_depth } => *my_depth,

                    // Because this function has had its facts fully determined, we know that it
                    // couldn't have possibly called into a function which is currently being analyzed
                    // because those functions would also be marked as done and would therefore not
                    // be analyzed ever again.
                    MaybeFunctionFacts::Done(_) => u32::MAX,
                };
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(MaybeFunctionFacts::Pending { my_depth });
            }
        }

        // If this is a function pointer, analyze its multiple dispatch.
        let my_body_id = match my_body_id {
            AnalysisSubject::Instance(instance) => instance,
            AnalysisSubject::FnPtr(fn_ty) => {
                let (min_recursion_into, my_facts) = self.analyze_multi(
                    my_depth,
                    self.fn_impls
                        .get(&fn_ty)
                        .unwrap_or(&FxHashSet::default())
                        .iter()
                        .copied(),
                );

                self.fn_facts.insert(
                    AnalysisSubject::FnPtr(fn_ty),
                    MaybeFunctionFacts::Done(my_facts),
                );

                return min_recursion_into;
            }
        };

        // If this function has a hardcoded fact set, use it.
        if let Some(facts) = self.get_hardcoded_facts(my_body_id) {
            self.fn_facts.insert(
                AnalysisSubject::Instance(my_body_id),
                MaybeFunctionFacts::Done(facts),
            );

            return u32::MAX;
        }

        // Acquire the function body or, if it's a dynamic dispatch, analyze its potential targets.
        let my_body = match safeishly_grab_instance_mir(self.tcx, my_body_id.def) {
            MirGrabResult::Found(body) => body,
            MirGrabResult::Dynamic => {
                let (min_recursion_into, my_facts) = self.analyze_multi(
                    my_depth,
                    self.trait_impls
                        .get(&(my_body_id.def_id(), my_body_id.args))
                        .unwrap_or(&FxHashSet::default())
                        .iter()
                        .copied(),
                );

                self.fn_facts.insert(
                    AnalysisSubject::Instance(my_body_id),
                    MaybeFunctionFacts::Done(my_facts),
                );

                return min_recursion_into;
            }
            MirGrabResult::BottomsOut => {
                self.fn_facts.insert(
                    AnalysisSubject::Instance(my_body_id),
                    MaybeFunctionFacts::Done(FunctionFactsMap::default()),
                );

                return u32::MAX;
            }
        };

        // Keep track of the minimum recursion depth.
        let mut min_recurse_into = u32::MAX;
        let mut my_facts = FunctionFactsMap::default();

        // Also keep track of whether we are allowed to borrow things mutably in this function.
        let mut cannot_have_mutables_of = FxHashSet::<Ty>::default();

        // Now, we have to analyze the basic blocks' calling in some arbitrary order to determine both
        // which components are being borrowed and the function's leaked effects.
        let mut process_stack = vec![START_BLOCK];
        let mut bb_facts = (0..my_body.basic_blocks.len())
            .map(|_| None::<LeakFactsMap>)
            .collect::<Vec<_>>();

        bb_facts[START_BLOCK.as_usize()] = Some(LeakFactsMap::default());
        bb_facts.push(None); // We use the last bb as a fake bb for all returns.

        while let Some(curr_id) = process_stack.pop() {
            let curr = &my_body.basic_blocks[curr_id];
            let curr_terminator = curr.terminator.as_ref().unwrap();
            let curr_facts = bb_facts[curr_id.as_usize()].as_ref().unwrap();

            let mut span = curr_terminator.source_info.span;

            // Determine whether this block could possibly call another function and collect the
            // list of basic-block targets.
            //
            // N.B. we intentionally ignore panics because they complicate analysis a lot and the
            // program is already broken by that point so we probably shouldn't bother ensuring that
            // those are safe.
            let (calls, targets): (_, SmallVec<[_; 2]>) = match &curr_terminator.kind {
                //> The following terminators have no effects and are just connectors to other blocks.
                TerminatorKind::Goto { target } | TerminatorKind::Assert { target, .. } => {
                    (None, smallvec![*target])
                }
                TerminatorKind::SwitchInt { targets, .. } => {
                    (None, targets.all_targets().iter().copied().collect())
                }

                // Inline assembly is already quite inherently dangerous so it's probably fine to
                // not bother trying to determine who it calls. I mean, how would we even do that
                // analysis?
                TerminatorKind::InlineAsm { destination, .. } => {
                    (None, destination.iter().copied().collect())
                }

                //> The following terminators have no effects or blocks to call to.
                TerminatorKind::UnwindResume
                | TerminatorKind::UnwindTerminate(_)
                | TerminatorKind::Unreachable => (None, smallvec![]),

                //> The following terminator is special in that it is the only way to safely
                //> return. We treat this as branching to the last bb, which we reserve as
                //> the terminator branch
                TerminatorKind::Return => (None, smallvec![BasicBlock::from(bb_facts.len() - 1)]),

                //> The following terminators may call into other functions and, therefore, may
                //> have effects.
                TerminatorKind::Call {
                    func,
                    target,
                    fn_span,
                    ..
                } => {
                    span = *fn_span;

                    let func = func.ty(&my_body.local_decls, self.tcx);
                    let func = my_body_id.subst_mir_and_normalize_erasing_regions(
                        self.tcx,
                        ParamEnv::reveal_all(),
                        EarlyBinder::bind(func),
                    );
                    match func.kind() {
                        TyKind::FnDef(callee_id, generics) => {
                            let callee_id = Instance::expect_resolve(
                                self.tcx,
                                ParamEnv::reveal_all(),
                                *callee_id,
                                generics,
                            );

                            (
                                Some(AnalysisSubject::Instance(callee_id)),
                                (*target).into_iter().collect(),
                            )
                        }
                        TyKind::FnPtr(_) => (
                            Some(AnalysisSubject::FnPtr(func)),
                            (*target).into_iter().collect(),
                        ),
                        _ => unreachable!(),
                    }
                }
                TerminatorKind::Drop { place, target, .. } => {
                    let place = place.ty(&my_body.local_decls, self.tcx).ty;
                    let place = my_body_id.subst_mir_and_normalize_erasing_regions(
                        self.tcx,
                        ParamEnv::reveal_all(),
                        EarlyBinder::bind(place),
                    );

                    let dtor = place.needs_drop(self.tcx, ParamEnv::reveal_all()).then(|| {
                        AnalysisSubject::Instance(Instance::resolve_drop_in_place(self.tcx, place))
                    });

                    (dtor, smallvec![*target])
                }

                //> The following terminators never happen:

                // Yield is not permitted after generator lowering, which we force before our
                // analysis.
                TerminatorKind::Yield { .. } | TerminatorKind::GeneratorDrop { .. } => {
                    unreachable!("generators should have been lowered by this point")
                }

                // We have already completed drop elaboration so this won't occur either.
                TerminatorKind::FalseEdge { .. } | TerminatorKind::FalseUnwind { .. } => {
                    unreachable!("drops should have been elaborated by this point")
                }
            };

            // If we call a function, analyze and propagate their leaked borrows.
            let empty_fact_map = FunctionFactsMap::default();
            let call_facts = if let Some(callee_id) = calls {
                // Analyze the callees and determine the `min_recurse_into` depth.
                let this_min_recurse_level = self.analyze_single(my_depth + 1, callee_id);

                min_recurse_into = min_recurse_into.min(this_min_recurse_level);

                // For self-recursion, we do actually have to ensure that we don't have any
                // ongoing mutable borrows and that, if we do have ongoing immutable borrows,
                // then we don't be doing any mutable borrowing.
                //
                // N.B. we only care about the counts of borrows relative to this current
                // function. If the outside scope already has some active borrows, it will
                // yield the error using regular maximum allowable borrow semantics.
                if this_min_recurse_level <= my_depth {
                    for (&comp_ty, curr_facts) in curr_facts {
                        if curr_facts.leaked_muts > 0 {
                            let leaked_muts = curr_facts.leaked_muts;

                            self.tcx.sess.span_warn(
                                span,
                                format!(
                                        "this function calls itself recursively while holding at least \
                                         {leaked_muts} mutable borrow{} of {comp_ty:?} meaning that, if \
                                         it does reach this same call again, it may mutably borrow the \
                                         same component more than once",
                                        s_pluralize(leaked_muts),
                                    ),
                            );
                        }

                        if curr_facts.leaked_refs > 0 {
                            cannot_have_mutables_of.insert(comp_ty);
                        }
                    }
                }

                // Determine the facts of this callee.
                match &self.fn_facts[&callee_id] {
                    // If the function was pending, we know that it calls itself recursively. We can
                    // assume that the only valid choice for a recursively called function is to not
                    // leak anything because, if it did leak a borrow, one could construct an MIR
                    // trace with an unbounded number of leaks. We'll verify this assumption at the
                    // end for every function which detects itself to be self-recursive using the
                    // `min_recurse_into` value.
                    //
                    // We also pretend as if the function did not borrow anything because the fact
                    // that it borrowed something can come from a different directly-observed call.
                    MaybeFunctionFacts::Pending { .. } => &empty_fact_map,
                    MaybeFunctionFacts::Done(facts) => facts,
                }
            } else {
                &empty_fact_map
            };

            // Validate the facts.
            for (comp_ty, &call_facts) in call_facts.iter() {
                let my_facts = my_facts.entry(*comp_ty).or_default();
                let curr_facts = curr_facts.get(comp_ty).copied().unwrap_or_default();

                // Adjust the max enter borrow counters appropriately.
                //
                // my_facts.max_enter_(im)mutable_borrows + curr_facts.leaked_(im)mutables
                //    <= call_facts.max_enter_(im)mutable_borrows
                //
                // So:
                //
                // my_facts.max_enter_(im)mutable_borrows
                //    <= call_facts.max_enter_(im)mutable_borrows - curr_facts.leaked_(im)mutables
                //

                let constrict_max_enter_mut = call_facts
                    .max_enter_mut
                    .saturating_sub(curr_facts.leaked_muts);

                if constrict_max_enter_mut >= 0 {
                    my_facts.max_enter_mut = my_facts.max_enter_mut.min(constrict_max_enter_mut);
                } else {
                    let max_enter_mut = call_facts.max_enter_mut;
                    let leaked_muts = curr_facts.leaked_muts;

                    self.tcx.sess.span_warn(
                        span,
                        format!(
                            "called a function expecting at most {max_enter_mut} mutable borrow{} of \
                            type {comp_ty:?} but was called in a scope with at least {leaked_muts}",
                            s_pluralize(max_enter_mut),
                        ),
                    );
                }

                let constrict_max_enter_ref = call_facts
                    .max_enter_ref
                    .saturating_sub(curr_facts.leaked_refs);

                if constrict_max_enter_ref >= 0 {
                    my_facts.max_enter_ref = my_facts.max_enter_ref.min(constrict_max_enter_ref);
                } else {
                    let max_enter_ref = call_facts.max_enter_ref;
                    let leaked_refs = curr_facts.leaked_refs;

                    self.tcx.sess.span_warn(
                        span,
                        format!(
                                "called a function expecting at most {max_enter_ref} immutable borrow{} of \
                                type {comp_ty:?} but was called in a scope with at least {leaked_refs}",
                                s_pluralize(max_enter_ref),
                            ),
                    );
                }

                my_facts.mutably_borrows |= call_facts.mutably_borrows;
            }

            // Propagate the leak facts to the target basic blocks and determine which targets we
            // still need to process. We make sure to strip our `leak_expectation` map of empty
            // entries to ensure that there's only one valid encoding of it.
            let mut leak_expectation = LeakFactsMap::default();

            for (comp_ty, call_facts) in call_facts {
                if call_facts.leaks.leaked_refs != 0 || call_facts.leaks.leaked_muts != 0 {
                    let leak_facts = leak_expectation.entry(*comp_ty).or_default();
                    leak_facts.leaked_refs = call_facts.leaks.leaked_refs;
                    leak_facts.leaked_muts = call_facts.leaks.leaked_muts;
                }
            }

            for (comp_ty, curr_facts) in curr_facts {
                match leak_expectation.entry(*comp_ty) {
                    hash_map::Entry::Occupied(mut leak_facts) => {
                        let new_facts = LeakFacts {
                            leaked_refs: leak_facts.get().leaked_refs + curr_facts.leaked_refs,
                            leaked_muts: leak_facts.get().leaked_muts + curr_facts.leaked_muts,
                        };

                        if new_facts == LeakFacts::default() {
                            leak_facts.remove();
                        } else {
                            *leak_facts.get_mut() = new_facts;
                        }
                    }
                    hash_map::Entry::Vacant(leak_facts) => {
                        if curr_facts.leaked_refs != 0 || curr_facts.leaked_muts != 0 {
                            leak_facts.insert(*curr_facts);
                        }
                    }
                }
            }

            for &target in &targets {
                let bb_target = &mut bb_facts[target.as_usize()];
                match bb_target {
                    Some(target_facts) => {
                        // If not all paths result in the same number of leaks, there will always be
                        // at least one theoretically taken path which could cause a borrow error or
                        // invalid leak.

                        if target_facts != &leak_expectation {
                            // Report the error and proceed with analysis using one of the assumptions
                            // made since, even though the analysis may be incomplete, we'll still
                            // produce useful diagnostics.
                            self.tcx.sess.span_warn(
                                    span,
                                    "not all control-flow paths to this statement are guaranteed to borrow \
                                     the same number of components",
                                );
                        }
                    }
                    None => {
                        *bb_target = Some(leak_expectation.clone());

                        // It doesn't make sense to push the return basic block.
                        if target.as_usize() < bb_facts.len() - 1 {
                            process_stack.push(target);
                        }
                    }
                }
            }
        }

        // Gather the functions leaks from the leaks of the terminator block.
        for (comp_ty, bb_facts) in bb_facts
            .last()
            .unwrap()
            .as_ref()
            .unwrap_or(&LeakFactsMap::default())
        {
            let my_facts = my_facts.entry(*comp_ty).or_default();

            my_facts.leaks = *bb_facts;

            // If we are self-recursive, we know that we mustn't have leaked anything. See above for
            // an explanation of why.
            if min_recurse_into <= my_depth && my_facts.leaks != LeakFacts::default() {
                self.tcx.sess.span_warn(
                    my_body.span,
                    format!(
                            "this function self-recurses yet has the ability to leak borrows of {comp_ty:?}, \
                             meaning that it could theoretically leak an arbitrary number of borrows",
                        ),
                );
            }
        }

        // Ensure that, if we deemed that this function is disallowed from borrowing mutably, then the
        // rule is actually enforced.
        for forbidden in cannot_have_mutables_of {
            if my_facts
                .get(&forbidden)
                .is_some_and(|fact| fact.mutably_borrows)
            {
                self.tcx.sess.span_warn(
                    my_body.span,
                    format!(
                            "this function self-recurses while holding an immutable borrow to {forbidden:?} \
                             but holds the potential of borrowing that same component mutably somewhere \
                             in the function body",
                        ),
                );
            }
        }

        // We're almost done but we have to apply the special `__autoken_assume_no_alias` rules, which
        // preserve leaks but allow the incoming maximum borrow counts to be unbounded.
        if self
            .tcx
            .opt_item_name(my_body_id.def_id())
            .is_some_and(|name| name == sym::__autoken_assume_no_alias_in.get())
        {
            let ignored_ty = self.tcx.erase_regions_ty(my_body_id.args[0].expect_ty());
            let singleton_arr: [Ty<'tcx>; 1];

            let ignored_tys = if let TyKind::Tuple(list) = ignored_ty.kind() {
                list.as_slice().iter()
            } else {
                singleton_arr = [ignored_ty];
                singleton_arr.iter()
            };

            for ignored_ty in ignored_tys {
                if let Some(my_facts) = my_facts.get_mut(ignored_ty) {
                    my_facts.max_enter_mut = i32::MAX;
                    my_facts.max_enter_ref = i32::MAX;
                    my_facts.mutably_borrows = false;
                }
            }
        }

        if self
            .tcx
            .opt_item_name(my_body_id.def_id())
            .is_some_and(|name| name == sym::__autoken_assume_no_alias.get())
        {
            for my_facts in my_facts.values_mut() {
                my_facts.max_enter_mut = i32::MAX;
                my_facts.max_enter_ref = i32::MAX;
                my_facts.mutably_borrows = false;
            }
        }

        // Finally, save our resolved facts.
        *self
            .fn_facts
            .get_mut(&AnalysisSubject::Instance(my_body_id))
            .unwrap() = MaybeFunctionFacts::Done(my_facts);

        min_recurse_into
    }

    fn analyze_multi(
        &mut self,
        my_depth: u32,
        callees: impl IntoIterator<Item = Instance<'tcx>>,
    ) -> (u32, FunctionFactsMap<'tcx>) {
        let mut min_recurse_into = u32::MAX;
        let mut my_facts = FunctionFactsMap::default();

        for callee in callees {
            min_recurse_into = min_recurse_into
                .min(self.analyze_single(my_depth, AnalysisSubject::Instance(callee)));

            // This analysis is largely copied from what happens on below.
            let empty_fact_map = FunctionFactsMap::default();
            let call_facts = match &self.fn_facts[&AnalysisSubject::Instance(callee)] {
                MaybeFunctionFacts::Pending { .. } => &empty_fact_map,
                MaybeFunctionFacts::Done(facts) => facts,
            };

            for (comp_ty, &call_facts) in call_facts.iter() {
                let my_facts = my_facts.entry(*comp_ty).or_default();

                // TODO: Handle disparities in leak counts.
                my_facts.leaks.leaked_muts =
                    my_facts.leaks.leaked_muts.max(call_facts.leaks.leaked_muts);
                my_facts.leaks.leaked_refs =
                    my_facts.leaks.leaked_refs.max(call_facts.leaks.leaked_refs);

                my_facts.max_enter_mut = my_facts.max_enter_mut.min(call_facts.max_enter_mut);
                my_facts.max_enter_ref = my_facts.max_enter_ref.min(call_facts.max_enter_ref);
                my_facts.mutably_borrows |= call_facts.mutably_borrows;
            }
        }

        (min_recurse_into, my_facts)
    }

    fn get_hardcoded_facts(&self, my_body_id: Instance<'tcx>) -> Option<FunctionFactsMap<'tcx>> {
        let item_name = self.tcx.opt_item_name(my_body_id.def_id())?;

        // If this is a black box, pretend as if it is entirely uninteresting without visiting the
        // closure it calls.
        if item_name == sym::__autoken_assume_black_box.get() {
            return Some(FunctionFactsMap::default());
        }

        // Try to hardcode the remaining primitive borrow methods.
        let facts = if item_name == sym::__autoken_borrow_mutably.get() {
            FunctionFacts {
                max_enter_mut: 0,
                max_enter_ref: 0,
                mutably_borrows: true,
                leaks: LeakFacts {
                    leaked_muts: 1,
                    leaked_refs: 0,
                },
            }
        } else if item_name == sym::__autoken_unborrow_mutably.get() {
            FunctionFacts {
                max_enter_mut: i32::MAX,
                max_enter_ref: i32::MAX,
                mutably_borrows: false,
                leaks: LeakFacts {
                    leaked_muts: -1,
                    leaked_refs: 0,
                },
            }
        } else if item_name == sym::__autoken_borrow_immutably.get() {
            FunctionFacts {
                max_enter_mut: 0,
                max_enter_ref: i32::MAX,
                mutably_borrows: false,
                leaks: LeakFacts {
                    leaked_muts: 0,
                    leaked_refs: 1,
                },
            }
        } else if item_name == sym::__autoken_unborrow_immutably.get() {
            FunctionFacts {
                max_enter_mut: i32::MAX,
                max_enter_ref: i32::MAX,
                mutably_borrows: false,
                leaks: LeakFacts {
                    leaked_muts: 0,
                    leaked_refs: -1,
                },
            }
        } else {
            return None;
        };

        // If they borrow nothing, fall back to an empty map.
        let ty = self.tcx.erase_regions_ty(my_body_id.args[0].expect_ty());

        if Self::is_nothing_type(ty) {
            Some(FunctionFactsMap::default())
        } else {
            Some(FunctionFactsMap::from_iter([(ty, facts)]))
        }
    }

    fn is_nothing_type(ty: Ty<'tcx>) -> bool {
        let Some(adt) = ty.ty_adt_def() else {
            return false;
        };

        let Some(field) = adt.all_fields().next() else {
            return false;
        };

        field.name == sym::__autoken_nothing_type_field_indicator.get()
    }
}

// === Helpers === //

fn s_pluralize(v: i32) -> &'static str {
    if v == 1 {
        ""
    } else {
        "s"
    }
}

enum MirGrabResult<'tcx> {
    Found(&'tcx Body<'tcx>),
    Dynamic,
    BottomsOut,
}

fn safeishly_grab_instance_mir<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: InstanceDef<'tcx>,
) -> MirGrabResult<'tcx> {
    match instance {
        // Items are defined by users and thus have MIR... even if they're from an external crate.
        InstanceDef::Item(item) => {
            // However, foreign items and lang-items don't have MIR
            if !tcx.is_foreign_item(item) {
                MirGrabResult::Found(tcx.instance_mir(instance))
            } else {
                MirGrabResult::BottomsOut
            }
        }

        // All the remaining things here require shims. We referenced...
        //
        // https://github.com/rust-lang/rust/blob/9c20ddd956426d577d77cb3f57a7db2227a3c6e9/compiler/rustc_mir_transform/src/shim.rs#L29
        //
        // ...to figure out which instance def types support this operation.

        // These are always supported.
        InstanceDef::ThreadLocalShim(_)
        | InstanceDef::DropGlue(_, _)
        | InstanceDef::ClosureOnceShim { .. }
        | InstanceDef::CloneShim(_, _)
        | InstanceDef::FnPtrAddrShim(_, _) => MirGrabResult::Found(tcx.instance_mir(instance)),

        // These are never supported and will never return to the user.
        InstanceDef::Intrinsic(_) => MirGrabResult::BottomsOut,

        // These are dynamic dispatches and should not be analyzed since we analyze them in a
        // different way.
        InstanceDef::VTableShim(_)
        | InstanceDef::ReifyShim(_)
        | InstanceDef::FnPtrShim(_, _)
        | InstanceDef::Virtual(_, _) => MirGrabResult::Dynamic,
    }
}

// Referenced from https://github.com/rust-lang/rust/blob/4b85902b438f791c5bfcb6b1c5b476d5b88e2bef/compiler/rustc_codegen_cranelift/src/unsize.rs#L62
fn get_unsized_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
) -> (Ty<'tcx>, Ty<'tcx>) {
    match (from_ty.kind(), to_ty.kind()) {
        // Reference unsizing
        (TyKind::Ref(_, a, _), TyKind::Ref(_, b, _))
        | (TyKind::Ref(_, a, _), TyKind::RawPtr(TypeAndMut { ty: b, mutbl: _ }))
        | (
            TyKind::RawPtr(TypeAndMut { ty: a, mutbl: _ }),
            TyKind::RawPtr(TypeAndMut { ty: b, mutbl: _ }),
        ) => get_unsized_ty(tcx, *a, *b),

        // Box unsizing
        (TyKind::Adt(def_a, _), TyKind::Adt(def_b, _)) if def_a.is_box() && def_b.is_box() => {
            get_unsized_ty(tcx, from_ty.boxed_ty(), to_ty.boxed_ty())
        }

        // Structural unsizing
        (TyKind::Adt(def_a, args_a), TyKind::Adt(def_b, args_b)) => {
            assert_eq!(def_a, def_b);

            for field in def_a.all_fields() {
                let from_ty = field.ty(tcx, args_a);
                let to_ty = field.ty(tcx, args_b);
                if from_ty != to_ty {
                    return get_unsized_ty(tcx, from_ty, to_ty);
                }
            }

            (from_ty, to_ty)
        }

        // Identity unsizing
        _ => (from_ty, to_ty),
    }
}

struct ReusedSymbol {
    raw: &'static str,
    sym: OnceLock<Symbol>,
}

impl ReusedSymbol {
    const fn new(raw: &'static str) -> Self {
        Self {
            raw,
            sym: OnceLock::new(),
        }
    }

    fn get(&self) -> Symbol {
        *self.sym.get_or_init(|| Symbol::intern(self.raw))
    }
}

// === Symbols === //

#[allow(non_upper_case_globals)]
mod sym {
    use super::ReusedSymbol;

    pub static __autoken_borrow_mutably: ReusedSymbol =
        ReusedSymbol::new("__autoken_borrow_mutably");

    pub static __autoken_unborrow_mutably: ReusedSymbol =
        ReusedSymbol::new("__autoken_unborrow_mutably");

    pub static __autoken_borrow_immutably: ReusedSymbol =
        ReusedSymbol::new("__autoken_borrow_immutably");

    pub static __autoken_unborrow_immutably: ReusedSymbol =
        ReusedSymbol::new("__autoken_unborrow_immutably");

    pub static __autoken_assume_no_alias_in: ReusedSymbol =
        ReusedSymbol::new("__autoken_assume_no_alias_in");

    pub static __autoken_assume_no_alias: ReusedSymbol =
        ReusedSymbol::new("__autoken_assume_no_alias");

    pub static __autoken_assume_black_box: ReusedSymbol =
        ReusedSymbol::new("__autoken_assume_black_box");

    pub static __autoken_nothing_type_field_indicator: ReusedSymbol =
        ReusedSymbol::new("__autoken_nothing_type_field_indicator");
}
