use std::collections::hash_map;

use rustc_hash::FxHashMap;
use rustc_hir::def_id::DefId;
use rustc_middle::ty::{Mutability, Ty, TyCtxt};
use rustc_span::Symbol;

use crate::{
    analyzer::sets::{get_tie, instantiate_set_proc, is_tie_func},
    util::{
        hash::{new_const_hash_set, FxHashSet},
        mir::{get_static_callee_from_terminator, has_instance_mir, TerminalCallKind},
        ty::{
            ConcretizedFunc, GenericTransformer, MaybeConcretizedArgs, MaybeConcretizedFunc,
            MutabilityExt,
        },
    },
};

// === Function Fact Store === //

pub fn has_facts(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    !is_tie_func(tcx, def_id) && has_instance_mir(tcx, def_id)
}

#[derive(Debug, Default)]
pub struct FunctionFactStore<'tcx> {
    facts: FxHashMap<DefId, FunctionFacts<'tcx>>,
    collection_queue: Vec<DefId>,
}

impl<'tcx> FunctionFactStore<'tcx> {
    pub fn collect(&mut self, tcx: TyCtxt<'tcx>, def_id: DefId) {
        self.collection_queue.push(def_id);

        while let Some(def_id) = self.collection_queue.pop() {
            // Ensure that we're analyzing a new function
            let hash_map::Entry::Vacant(entry) = self.facts.entry(def_id) else {
                continue;
            };
            let entry = entry.insert(FunctionFacts::new(def_id));

            // Validate the function
            assert!(has_facts(tcx, def_id));

            // Traverse the function
            let body = tcx.optimized_mir(def_id);

            for bb in body.basic_blocks.iter() {
                match get_static_callee_from_terminator(tcx, &bb.terminator, &body.local_decls) {
                    Some(TerminalCallKind::Static(span, called)) => {
                        if is_tie_func(tcx, called.def_id()) {
                            let tied_to = get_tie(tcx, called.args()[0].expect_ty());

                            // TODO: Populate alias classes.

                            instantiate_set_proc(
                                tcx,
                                span,
                                called.args()[1].expect_ty(),
                                &mut |ty, mutability| {
                                    entry
                                        .found_borrows
                                        .entry(ty)
                                        .or_insert_with(Default::default)
                                        .upgrade(mutability, tied_to);
                                },
                                Some(&mut |ty, mutability| {
                                    entry
                                        .generic_borrow_sets
                                        .entry(ty)
                                        .or_insert_with(Default::default)
                                        .upgrade(mutability, tied_to);
                                }),
                            );
                        } else if has_instance_mir(tcx, called.def_id()) {
                            entry.known_calls.insert(called);
                            self.collection_queue.push(called.def_id());
                        }
                    }
                    Some(TerminalCallKind::Generic(_span, called)) => {
                        entry
                            .generic_calls
                            .entry(called)
                            .or_insert_with(GenericCallInfo::default);
                    }
                    Some(TerminalCallKind::Dynamic) => {
                        // (ignored)
                    }
                    None => {
                        // (ignored)
                    }
                }
            }
        }
    }

    pub fn optimize(&mut self) {
        // TODO: Optimize this graph to reduce redundant searching.
    }

    pub fn lookup(&self, def_id: DefId) -> Option<&FunctionFacts<'tcx>> {
        self.facts.get(&def_id)
    }
}

#[derive(Debug)]
pub struct FunctionFacts<'tcx> {
    pub def_id: DefId,

    /// The statically-resolved non-generic functions this function can call without accounting for
    /// deep calls.
    pub known_calls: FxHashSet<ConcretizedFunc<'tcx>>,

    /// The statically-resolved generic functions this function can call and their promises on what
    /// they won't borrow without accounting for deep calls.
    pub generic_calls: FxHashMap<ConcretizedFunc<'tcx>, GenericCallInfo<'tcx>>,

    /// The set of all concrete borrows for this function without accounting for deep calls.
    pub found_borrows: FxHashMap<Ty<'tcx>, BorrowInfo>,

    /// The set of all borrows of generic sets in this function without accounting for deep calls.
    pub generic_borrow_sets: FxHashMap<Ty<'tcx>, BorrowInfo>,

    /// The function's restrictions on types which it assumes not to alias without accounting for
    /// deep calls.
    pub alias_classes: FxHashMap<Ty<'tcx>, AliasClass>,
}

impl<'tcx> FunctionFacts<'tcx> {
    pub fn new(def_id: DefId) -> Self {
        Self {
            def_id,
            known_calls: FxHashSet::default(),
            generic_calls: FxHashMap::default(),
            found_borrows: FxHashMap::default(),
            generic_borrow_sets: FxHashMap::default(),
            alias_classes: FxHashMap::default(),
        }
    }
}

#[derive(Debug)]
pub struct BorrowInfo {
    pub mutability: Mutability,
    pub tied_to: FxHashSet<Symbol>,
}

impl Default for BorrowInfo {
    fn default() -> Self {
        Self {
            mutability: Mutability::Not,
            tied_to: FxHashSet::default(),
        }
    }
}

impl BorrowInfo {
    pub fn upgrade(&mut self, mutability: Mutability, tied_to: Option<Symbol>) {
        self.mutability.upgrade(mutability);

        if let Some(tied_to) = tied_to {
            self.tied_to.insert(tied_to);
        }
    }
}

#[derive(Debug, Default)]
pub struct GenericCallInfo<'tcx> {
    pub does_not_borrow: FxHashMap<Ty<'tcx>, Mutability>,
}

rustc_index::newtype_index! {
    pub struct AliasClass {}
}

// === Function Fact Instantiation === //

#[derive(Debug, Copy, Clone)]
pub enum FactInstantiatedCall<'tcx, 'a> {
    Concrete(ConcretizedFunc<'tcx>),
    Generic(ConcretizedFunc<'tcx>, &'a GenericCallInfo<'tcx>),
}

impl<'tcx> FunctionFacts<'tcx> {
    pub fn func(&self, args: MaybeConcretizedArgs<'tcx>) -> MaybeConcretizedFunc<'tcx> {
        MaybeConcretizedFunc(self.def_id, args)
    }

    pub fn instantiate_known_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
    ) -> impl Iterator<Item = ConcretizedFunc<'tcx>> + '_ {
        self.known_calls
            .iter()
            .map(move |&called| self.func(args).instantiate_func(tcx, called))
    }

    pub fn instantiate_generic_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
    ) -> impl Iterator<Item = FactInstantiatedCall<'tcx, '_>> + '_ {
        self.generic_calls
            .iter()
            .filter_map(move |(&called, info)| {
                match self
                    .func(args)
                    .instantiate_func(tcx, called)
                    .resolve_instance(tcx)
                {
                    Ok(Some(instance)) => Some(FactInstantiatedCall::Concrete(instance.into())),
                    Ok(None) => Some(FactInstantiatedCall::Generic(called, info)),
                    Err(_) => None,
                }
            })
    }

    pub fn instantiate_all_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
    ) -> impl Iterator<Item = FactInstantiatedCall<'tcx, '_>> + '_ {
        self.instantiate_known_calls(tcx, args)
            .map(FactInstantiatedCall::Concrete)
            .chain(self.instantiate_generic_calls(tcx, args))
    }

    pub fn instantiate_found_borrows(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, &BorrowInfo)> + '_ {
        self.found_borrows.iter().map(move |(&ty, borrow_info)| {
            (ty, self.func(args).instantiate_arg(tcx, ty), borrow_info)
        })
    }

    pub fn instantiate_borrow_sets(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, &BorrowInfo)> + '_ {
        self.generic_borrow_sets
            .iter()
            .map(move |(&ty, borrow_info)| (self.func(args).instantiate_arg(tcx, ty), borrow_info))
    }

    pub fn instantiate_alias_classes(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, AliasClass)> + '_ {
        self.alias_classes
            .iter()
            .map(move |(&ty, &class)| (ty, self.func(args).instantiate_arg(tcx, ty), class))
    }
}

impl<'tcx> GenericCallInfo<'tcx> {
    pub fn instantiate_does_not_borrow(
        &self,
        tcx: TyCtxt<'tcx>,
        func: MaybeConcretizedFunc<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Mutability)> + '_ {
        self.does_not_borrow
            .iter()
            .map(move |(ty, mutbl)| (func.instantiate_arg(tcx, *ty), *mutbl))
    }
}

// === Function Fact Exploration === //

pub static EMPTY_TIED_SET: &FxHashSet<Symbol> = &new_const_hash_set();

#[derive(Debug, Copy, Clone)]
pub enum IterBorrowsResult<'a, 'tcx, 'facts> {
    Only(&'a FxHashMap<Ty<'tcx>, (Mutability, &'facts FxHashSet<Symbol>)>),
    Exclude(&'a FxHashMap<Ty<'tcx>, Mutability>),
}

pub struct FactExplorer<'tcx, 'facts> {
    pub tcx: TyCtxt<'tcx>,
    pub facts: &'facts FunctionFactStore<'tcx>,
    pub reachable: ReachableFactExplorer<'tcx, 'facts>,

    borrows: FxHashMap<Ty<'tcx>, (Mutability, &'facts FxHashSet<Symbol>)>,
    negative_borrows: FxHashMap<Ty<'tcx>, Mutability>,
    used_with_ties: FxHashSet<Ty<'tcx>>,
}

impl<'tcx, 'facts> FactExplorer<'tcx, 'facts> {
    pub fn new(tcx: TyCtxt<'tcx>, facts: &'facts FunctionFactStore<'tcx>) -> Self {
        Self {
            tcx,
            facts,
            reachable: ReachableFactExplorer::default(),
            borrows: FxHashMap::default(),
            negative_borrows: FxHashMap::default(),
            used_with_ties: FxHashSet::default(),
        }
    }

    pub fn iter_used_with_ties(
        &mut self,
        tcx: TyCtxt<'tcx>,
        func: MaybeConcretizedFunc<'tcx>,
    ) -> &FxHashSet<Ty<'tcx>> {
        self.used_with_ties.clear();

        for &callee in &self.facts.lookup(func.def_id()).unwrap().known_calls {
            let facts = self.facts.lookup(callee.def_id()).unwrap();

            for (_ty, ty, info) in facts.instantiate_found_borrows(tcx, Some(callee.args())) {
                if !info.tied_to.is_empty() {
                    self.used_with_ties.insert(ty);
                }
            }
        }

        &self.used_with_ties
    }

    pub fn iter_borrows(
        &mut self,
        src_func: MaybeConcretizedFunc<'tcx>,
    ) -> IterBorrowsResult<'_, 'tcx, 'facts> {
        self.borrows.clear();
        self.negative_borrows.clear();

        let reachable = self
            .reachable
            .iter_reachable(self.tcx, self.facts, src_func);

        if reachable.has_generic_visits() {
            // Collect negative borrows
            for (func, info) in reachable.iter_generic() {
                for (do_not_borrow, mutability) in info.instantiate_does_not_borrow(self.tcx, func)
                {
                    self.negative_borrows
                        .entry(do_not_borrow)
                        .or_insert(mutability)
                        .upgrade(mutability);
                }
            }

            // Remove positive borrows
            for MaybeConcretizedFunc(def_id, args) in reachable.iter_concrete() {
                for (_, ty, info) in self
                    .facts
                    .lookup(def_id)
                    .unwrap() // iter_reachable only yields functions with facts
                    .instantiate_found_borrows(self.tcx, args)
                {
                    if info.mutability == Mutability::Mut {
                        self.negative_borrows.remove(&ty);
                    } else {
                        self.negative_borrows
                            .entry(ty)
                            .and_modify(|v| *v = Mutability::Not);
                    }
                }
            }

            IterBorrowsResult::Exclude(&self.negative_borrows)
        } else {
            for MaybeConcretizedFunc(def_id, args) in reachable.iter_concrete() {
                for (_, ty, info) in self
                    .facts
                    .lookup(def_id)
                    .unwrap() // iter_reachable only yields functions with facts
                    .instantiate_found_borrows(self.tcx, args)
                {
                    let (mutability, set) = self
                        .borrows
                        .entry(ty)
                        .or_insert((Mutability::Not, EMPTY_TIED_SET));

                    mutability.upgrade(info.mutability);

                    if def_id == src_func.def_id() && !info.tied_to.is_empty() {
                        *set = &info.tied_to;
                    }
                }
            }

            IterBorrowsResult::Only(&self.borrows)
        }
    }
}

// Reachability
#[derive(Default)]
pub struct ReachableFactExplorer<'tcx, 'facts> {
    concrete_visit_set: FxHashSet<MaybeConcretizedFunc<'tcx>>,
    generic_visit_set: FxHashMap<MaybeConcretizedFunc<'tcx>, &'facts GenericCallInfo<'tcx>>,
}

impl<'tcx, 'facts> ReachableFactExplorer<'tcx, 'facts> {
    pub fn iter_reachable(
        &mut self,
        tcx: TyCtxt<'tcx>,
        facts: &'facts FunctionFactStore<'tcx>,
        func: MaybeConcretizedFunc<'tcx>,
    ) -> ReachableFuncs<'_, 'tcx, 'facts> {
        self.concrete_visit_set.clear();
        self.generic_visit_set.clear();
        self.iter_reachable_inner(tcx, facts, func);

        ReachableFuncs {
            concrete_visit_set: &self.concrete_visit_set,
            generic_visit_set: &self.generic_visit_set,
        }
    }

    fn iter_reachable_inner(
        &mut self,
        tcx: TyCtxt<'tcx>,
        facts: &'facts FunctionFactStore<'tcx>,
        src_func: MaybeConcretizedFunc<'tcx>,
    ) {
        let Some(src_facts) = facts.lookup(src_func.def_id()) else {
            return;
        };

        if !self.concrete_visit_set.insert(src_func) {
            return;
        }

        for dest in src_facts.instantiate_all_calls(tcx, src_func.args()) {
            match dest {
                FactInstantiatedCall::Concrete(dest) => {
                    self.iter_reachable_inner(tcx, facts, dest.into());
                }
                FactInstantiatedCall::Generic(dest, info) => {
                    self.generic_visit_set.insert(dest.into(), info);
                }
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ReachableFuncs<'a, 'tcx, 'facts> {
    pub concrete_visit_set: &'a FxHashSet<MaybeConcretizedFunc<'tcx>>,
    pub generic_visit_set: &'a FxHashMap<MaybeConcretizedFunc<'tcx>, &'facts GenericCallInfo<'tcx>>,
}

impl<'a, 'tcx, 'facts> ReachableFuncs<'a, 'tcx, 'facts> {
    pub fn has_generic_visits(self) -> bool {
        !self.generic_visit_set.is_empty()
    }

    pub fn iter_concrete(self) -> impl Iterator<Item = MaybeConcretizedFunc<'tcx>> + 'a {
        self.concrete_visit_set.iter().copied()
    }

    pub fn iter_generic(
        self,
    ) -> impl Iterator<Item = (MaybeConcretizedFunc<'tcx>, &'facts GenericCallInfo<'tcx>)> + 'a
    {
        self.generic_visit_set.iter().map(|(&k, &v)| (k, v))
    }
}
