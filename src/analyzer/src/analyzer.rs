use std::{cell::RefCell, collections::hash_map};

use rustc_data_structures::stable_hasher::Hash128;
use smallvec::{smallvec, SmallVec};

use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_interface::interface::Compiler;
use rustc_middle::{
    mir::{BasicBlock, TerminatorKind, START_BLOCK},
    ty::{EarlyBinder, Instance, ParamEnv, TyCtxt, TyKind},
};
use rustc_span::Symbol;

// === Driver === //

pub struct AnalyzerConfig {}

impl AnalyzerConfig {
    pub fn analyze(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) {
        let Some((main_fn, _)) = tcx.entry_fn(()) else {
            return;
        };
        let analyzer = Analyzer::new(tcx);

        analyzer.analyze(main_fn);

        // FIXME: This should not be necessary but currently is because `cc` doesn't work properly.
        std::process::exit(0);
    }
}

// === Analyzer === //

pub struct Analyzer<'tcx> {
    tcx: TyCtxt<'tcx>,
    fn_facts: RefCell<FxHashMap<Instance<'tcx>, MaybeFunctionFacts>>,
}

enum MaybeFunctionFacts {
    Pending { my_depth: u32 },
    Done(FactMap<FunctionFacts>),
}

type FactMap<T> = FxHashMap<Hash128, T>;

type FunctionFactsMap = FactMap<FunctionFacts>;

type LeakFactsMap = FactMap<LeakFacts>;

#[derive(Copy, Clone, Default)]
struct FunctionFacts {
    borrows_immutably: bool,
    borrows_mutably: bool,
    leaks: LeakFacts,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
struct LeakFacts {
    leaked_muts: i32,
    leaked_refs: i32,
}

impl<'tcx> Analyzer<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            fn_facts: Default::default(),
        }
    }

    /// Analyzes every function which is reachable from `body_id`.
    pub fn analyze(&self, body_id: DefId) {
        let _ = self.analyze_inner(0, Instance::mono(self.tcx, body_id));
    }

    /// Attempts to discover the facts about the provided function.
    ///
    /// Returns the inclusive depth of the lowest function on the stack we were able able to cycle
    /// back into or `u32::MAX` if the target never called a function which was already being analyzed.
    #[must_use]
    fn analyze_inner(&self, my_depth: u32, my_body_id: Instance<'tcx>) -> u32 {
        // If `my_body_id` corresponds to an autoken primitive, just hardcode its value.
        {
            let item_name = self.tcx.item_name(my_body_id.def_id());
            let facts = if item_name == Symbol::intern("__autoken_borrow_mutably") {
                Some(FunctionFacts {
                    borrows_immutably: true,
                    borrows_mutably: true,
                    leaks: LeakFacts {
                        leaked_muts: 1,
                        leaked_refs: 0,
                    },
                })
            } else if item_name == Symbol::intern("__autoken_unborrow_mutably") {
                Some(FunctionFacts {
                    borrows_immutably: false,
                    borrows_mutably: false,
                    leaks: LeakFacts {
                        leaked_muts: -1,
                        leaked_refs: 0,
                    },
                })
            } else if item_name == Symbol::intern("__autoken_borrow_immutably") {
                Some(FunctionFacts {
                    borrows_immutably: true,
                    borrows_mutably: false,
                    leaks: LeakFacts {
                        leaked_muts: 0,
                        leaked_refs: 1,
                    },
                })
            } else if item_name == Symbol::intern("__autoken_unborrow_immutably") {
                Some(FunctionFacts {
                    borrows_immutably: false,
                    borrows_mutably: false,
                    leaks: LeakFacts {
                        leaked_muts: 0,
                        leaked_refs: -1,
                    },
                })
            } else {
                None
            };

            if let Some(facts) = facts {
                self.fn_facts.borrow_mut().insert(
                    my_body_id,
                    MaybeFunctionFacts::Done(FunctionFactsMap::from_iter([(
                        self.tcx.type_id_hash(my_body_id.args[0].expect_ty()),
                        facts,
                    )])),
                );

                return u32::MAX;
            }
        }

        // Keep track of the minimum recursion depth.
        let mut min_recurse_into = u32::MAX;

        // Also keep track of whether we are allowed to borrow things mutably in this function.
        let mut cannot_have_mutables_of = FxHashSet::<Hash128>::default();

        // Create a blank fact entry for us. If a facts entry already exists, handle it as either a
        // cycle or a memoized result.
        match self.fn_facts.borrow_mut().entry(my_body_id) {
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

        let mut my_facts = FunctionFactsMap::default();

        // Acquire the function body
        let my_body = self.tcx.instance_mir(my_body_id.def);

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
            let curr_facts = bb_facts[curr_id.as_usize()].as_ref().unwrap();

            // Determine whether this block could possibly call another function and collect the
            // list of basic-block targets.
            //
            // N.B. we intentionally ignore panics because they complicate analysis a lot and the
            // program is already broken by that point so we probably shouldn't bother ensuring that
            // those are safe.
            let (calls, targets): (_, SmallVec<[_; 2]>) =
                match &curr.terminator.as_ref().unwrap().kind {
                    //> The following terminators have no effects and are just connectors to other blocks.
                    TerminatorKind::Goto { target } | TerminatorKind::Assert { target, .. } => {
                        (None, smallvec![*target])
                    }
                    TerminatorKind::SwitchInt { targets, .. } => {
                        (None, targets.iter().map(|(_, bb)| bb).collect())
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
                    | TerminatorKind::Unreachable => continue,

                    //> The following terminator is special in that it is the only way to safely
                    //> return. We treat this as branching to the last bb, which we reserve as
                    //> the terminator branch
                    TerminatorKind::Return => {
                        (None, smallvec![BasicBlock::from(bb_facts.len() - 1)])
                    }

                    //> The following terminators may call into other functions and, therefore, may
                    //> have effects.
                    TerminatorKind::Call { func, target, .. } => {
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

                                (Some(callee_id), (*target).into_iter().collect())
                            }
                            TyKind::FnPtr(_) => todo!(),
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

                        let dtor = place
                            .needs_drop(self.tcx, ParamEnv::reveal_all())
                            .then(|| Instance::resolve_drop_in_place(self.tcx, place));

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
            let call_facts = if let Some(callee_id) = calls {
                // Analyze the callees and determine the `min_recurse_into` depth.
                let this_min_recurse_level = self.analyze_inner(my_depth + 1, callee_id);

                min_recurse_into = min_recurse_into.min(this_min_recurse_level);

                // For self-recursion, we do actually have to ensure that we don't have any
                // ongoing mutable borrows and that, if we do have ongoing immutable borrows,
                // then we don't be doing any mutable borrowing.
                if this_min_recurse_level <= my_depth {
                    for (&comp_ty, curr_facts) in curr_facts {
                        assert_eq!(curr_facts.leaked_muts, 0);

                        if curr_facts.leaked_refs > 0 {
                            cannot_have_mutables_of.insert(comp_ty);
                        }
                    }
                }

                // Determine the facts of this callee.
                match &self.fn_facts.borrow()[&callee_id] {
                    // If the function was pending, we know that it calls itself recursively. We can
                    // assume that the only valid choice for a recursively called function is to not
                    // leak anything because, if it did leak a borrow, one could construct an MIR
                    // trace with an unbounded number of leaks. We'll verify this assumption at the
                    // end for every function which detects itself to be self-recursive using the
                    // `min_recurse_into` value.
                    //
                    // We also pretend as if the function did not borrow anything because the fact
                    // that it borrowed something can come from a different directly-observed call.
                    MaybeFunctionFacts::Pending { .. } => FunctionFactsMap::default(),
                    MaybeFunctionFacts::Done(facts) => facts.clone(),
                }
            } else {
                FunctionFactsMap::default()
            };

            // Validate the facts.
            for (comp_ty, &call_facts) in call_facts.iter() {
                let my_facts = my_facts.entry(*comp_ty).or_default();

                // Ensure that our borrow state is consistent with what is needed by our callee.
                if call_facts.borrows_immutably {
                    assert_eq!(curr_facts.get(comp_ty).map_or(0, |v| v.leaked_muts), 0);
                }

                if call_facts.borrows_mutably {
                    let curr_facts = curr_facts
                        .get(comp_ty)
                        .copied()
                        .unwrap_or(LeakFacts::default());

                    assert_eq!(curr_facts.leaked_refs, 0);
                    assert_eq!(curr_facts.leaked_muts, 0);
                }

                // Propagate this access fact to the current function.
                my_facts.borrows_immutably |= call_facts.borrows_immutably;
                my_facts.borrows_mutably |= call_facts.borrows_mutably;
            }

            // Propagate the leak facts to the target basic blocks and determine which targets we
            // still need to process. We make sure to strip our `leak_expectation` map of empty
            // entries to ensure that there's only one valid encoding of it.
            let mut leak_expectation = LeakFactsMap::default();

            for (comp_ty, call_facts) in &call_facts {
                if call_facts.leaks.leaked_refs > 0 || call_facts.leaks.leaked_muts > 0 {
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
                        if curr_facts.leaked_refs > 0 || curr_facts.leaked_muts > 0 {
                            leak_facts.insert(*curr_facts);
                        }
                    }
                }
            }

            for &target in &targets {
                let bb_target = &mut bb_facts[target.as_usize()];
                match bb_target {
                    Some(target_facts) => {
                        // If not all paths result in the same number of leaks, there's nothing we
                        // can do to save this program.
                        assert_eq!(target_facts, &leak_expectation);
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
            if min_recurse_into <= my_depth {
                assert_eq!(my_facts.leaks, LeakFacts::default());
            }
        }

        // Ensure that, if we deemed that this function is disallowed from borrowing mutably, then the
        // rule is actually enforced.
        for forbidden in cannot_have_mutables_of {
            assert!(!my_facts
                .get(&forbidden)
                .is_some_and(|fact| fact.borrows_mutably));
        }

        // Finally, save our resolved facts.
        *self.fn_facts.borrow_mut().get_mut(&my_body_id).unwrap() =
            MaybeFunctionFacts::Done(my_facts);

        min_recurse_into
    }
}
