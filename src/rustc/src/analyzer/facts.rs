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
        pool::{pool, Pooled},
        ty::{
            ConcretizedFunc, GenericTransformer, MaybeConcretizedArgs, MaybeConcretizedFunc,
            MutabilityExt,
        },
    },
};

// === Pools === //

pool! {
    pub def_id_vec => Vec<DefId>;
    pub ty_set<'tcx> => FxHashSet<Ty<'tcx>>;
    pub borrows_map<'tcx, 'facts> => FxHashMap<Ty<'tcx>, (Mutability, &'facts FxHashSet<Symbol>)>;
    pub concrete_visit_set<'tcx> => FxHashSet<MaybeConcretizedFunc<'tcx>>;
}

// === Function Fact Store === //

pub fn has_facts(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    !is_tie_func(tcx, def_id) && has_instance_mir(tcx, def_id)
}

pub struct FunctionFactStore<'tcx> {
    tcx: TyCtxt<'tcx>,
    facts: FxHashMap<DefId, FunctionFacts<'tcx>>,
}

impl<'tcx> FunctionFactStore<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            facts: FxHashMap::default(),
        }
    }

    pub fn collect(&mut self, def_id: DefId) {
        let mut collection_queue = def_id_vec();
        collection_queue.push(def_id);

        while let Some(def_id) = collection_queue.pop() {
            // Ensure that we're analyzing a new function
            let hash_map::Entry::Vacant(entry) = self.facts.entry(def_id) else {
                continue;
            };
            let entry = entry.insert(FunctionFacts::new(def_id));

            // Validate the function
            assert!(has_facts(self.tcx, def_id));

            // Traverse the function
            let body = self.tcx.optimized_mir(def_id);

            for bb in body.basic_blocks.iter() {
                match get_static_callee_from_terminator(self.tcx, &bb.terminator, &body.local_decls)
                {
                    Some(TerminalCallKind::Static(span, called)) => {
                        if is_tie_func(self.tcx, called.def_id()) {
                            let tied_to = get_tie(self.tcx, called.args()[0].expect_ty());

                            instantiate_set_proc(
                                self.tcx,
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
                        } else if has_instance_mir(self.tcx, called.def_id()) {
                            entry.known_calls.insert(called);
                            collection_queue.push(called.def_id());
                        }
                    }
                    Some(TerminalCallKind::Generic(_span, _called)) => {
                        // (ignored)
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

        // Populate alias classes
        // TODO: Clean this up
        for func in self.facts.keys().copied().collect::<Vec<_>>() {
            let used_in_ties = self.iter_used_with_ties(MaybeConcretizedFunc(func, None));

            let alias_classes = &mut self.facts.get_mut(&func).unwrap().alias_classes;

            for &ty in &*used_in_ties {
                let new_class = AliasClass::from_usize(alias_classes.len());
                alias_classes.entry(ty).or_insert(new_class);
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

rustc_index::newtype_index! {
    pub struct AliasClass {}
}

// === Function Fact Instantiation === //

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

// === Function Fact Exploration === //

static EMPTY_TIED_SET: &FxHashSet<Symbol> = &new_const_hash_set();

impl<'tcx> FunctionFactStore<'tcx> {
    pub fn iter_reachable<'facts>(
        &'facts self,
        func: MaybeConcretizedFunc<'tcx>,
    ) -> Pooled<FxHashSet<MaybeConcretizedFunc<'tcx>>> {
        let mut concrete_visit_set = concrete_visit_set();

        self.iter_reachable_inner(&mut concrete_visit_set, func);

        concrete_visit_set
    }

    fn iter_reachable_inner<'facts>(
        &'facts self,
        concrete_visit_set: &mut FxHashSet<MaybeConcretizedFunc<'tcx>>,
        src_func: MaybeConcretizedFunc<'tcx>,
    ) {
        let Some(src_facts) = self.lookup(src_func.def_id()) else {
            return;
        };

        if !concrete_visit_set.insert(src_func) {
            return;
        }

        for dest in src_facts.instantiate_known_calls(self.tcx, src_func.args()) {
            self.iter_reachable_inner(concrete_visit_set, dest.into());
        }
    }

    pub fn iter_used_with_ties(
        &self,
        func: MaybeConcretizedFunc<'tcx>,
    ) -> Pooled<FxHashSet<Ty<'tcx>>> {
        let mut used_with_ties = ty_set();

        for &callee in &self.lookup(func.def_id()).unwrap().known_calls {
            let facts = self.lookup(callee.def_id()).unwrap();

            for (_ty, ty, info) in facts.instantiate_found_borrows(self.tcx, Some(callee.args())) {
                if !info.tied_to.is_empty() {
                    used_with_ties.insert(ty);
                }
            }
        }

        used_with_ties
    }

    pub fn iter_borrows(
        &self,
        src_func: MaybeConcretizedFunc<'tcx>,
    ) -> Pooled<FxHashMap<Ty<'tcx>, (Mutability, &'_ FxHashSet<Symbol>)>> {
        let mut borrows = borrows_map();

        for MaybeConcretizedFunc(def_id, args) in &*self.iter_reachable(src_func) {
            for (_, ty, info) in self
                .lookup(*def_id)
                .unwrap() // iter_reachable only yields functions with facts
                .instantiate_found_borrows(self.tcx, *args)
            {
                let (mutability, set) = borrows
                    .entry(ty)
                    .or_insert((Mutability::Not, EMPTY_TIED_SET));

                mutability.upgrade(info.mutability);

                if *def_id == src_func.def_id() && !info.tied_to.is_empty() {
                    *set = &info.tied_to;
                }
            }
        }

        borrows
    }
}
