use std::collections::hash_map;

use rustc_hash::FxHashMap;
use rustc_hir::def_id::DefId;
use rustc_middle::ty::{
    EarlyBinder, GenericArg, Instance, List, Mutability, ParamEnv, Ty, TyCtxt, TypeFoldable,
};
use rustc_span::Symbol;

use crate::{
    analyzer::sets::{get_tie, instantiate_set_proc, is_tie_func},
    util::{
        hash::FxHashSet,
        mir::{
            get_static_callee_from_terminator, has_instance_mir, resolve_instance, TerminalCallKind,
        },
    },
};

// === FunctionFacts === //

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
            assert!(!is_tie_func(tcx, def_id));
            assert!(has_instance_mir(tcx, def_id));

            // Traverse the function
            let body = tcx.optimized_mir(def_id);

            for bb in body.basic_blocks.iter() {
                match get_static_callee_from_terminator(tcx, &bb.terminator, &body.local_decls) {
                    Some(TerminalCallKind::Static(span, called_did, args)) => {
                        if is_tie_func(tcx, called_did) {
                            let tied_to = get_tie(tcx, args[0].expect_ty());

                            instantiate_set_proc(
                                tcx,
                                span,
                                args[1].expect_ty(),
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
                        } else if has_instance_mir(tcx, called_did) {
                            entry.calls.insert((called_did, args));
                            self.collection_queue.push(called_did);
                        }
                    }
                    Some(TerminalCallKind::Generic(_span, called_did, args)) => {
                        entry.calls.insert((called_did, args));
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

    pub fn lookup(&self, def_id: DefId) -> &FunctionFacts<'tcx> {
        &self.facts[&def_id]
    }
}

#[derive(Debug)]
pub struct FunctionFacts<'tcx> {
    pub def_id: DefId,
    pub calls: FxHashSet<(DefId, &'tcx List<GenericArg<'tcx>>)>,
    pub found_borrows: FxHashMap<Ty<'tcx>, BorrowInfo>,
    pub generic_borrow_sets: FxHashMap<Ty<'tcx>, BorrowInfo>,
    pub alias_classes: FxHashMap<Ty<'tcx>, AliasClass>,
}

impl<'tcx> FunctionFacts<'tcx> {
    pub fn new(def_id: DefId) -> Self {
        Self {
            def_id,
            calls: FxHashSet::default(),
            found_borrows: FxHashMap::default(),
            generic_borrow_sets: FxHashMap::default(),
            alias_classes: FxHashMap::default(),
        }
    }

    pub fn instantiate_arg<T>(
        &self,
        tcx: TyCtxt<'tcx>,
        ty: T,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        Instance::new(self.def_id, args).instantiate_mir_and_normalize_erasing_regions(
            tcx,
            ParamEnv::reveal_all(),
            EarlyBinder::bind(ty),
        )
    }

    pub fn instantiate_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (CallConcretizationResult, &'tcx List<GenericArg<'tcx>>)> + '_ {
        self.calls.iter().filter_map(move |(def_id, called_args)| {
            let args = tcx.mk_args_from_iter(
                called_args
                    .iter()
                    .map(|arg| self.instantiate_arg(tcx, arg, args)),
            );

            match resolve_instance(tcx, *def_id, args) {
                Ok(Some(instance)) => Some((
                    CallConcretizationResult::Concrete(instance.def_id()),
                    instance.args,
                )),
                Ok(None) => Some((CallConcretizationResult::Generic(*def_id), args)),
                Err(_) => None,
            }
        })
    }

    pub fn instantiate_found_borrows(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, &BorrowInfo)> + '_ {
        self.found_borrows
            .iter()
            .map(move |(&ty, borrow_info)| (ty, self.instantiate_arg(tcx, ty, args), borrow_info))
    }

    pub fn instantiate_borrow_sets(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (Ty<'tcx>, &BorrowInfo)> + '_ {
        self.generic_borrow_sets
            .iter()
            .map(move |(&ty, borrow_info)| (self.instantiate_arg(tcx, ty, args), borrow_info))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum CallConcretizationResult {
    Concrete(DefId),
    Generic(DefId),
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
        self.mutability = self.mutability.max(mutability);

        if let Some(tied_to) = tied_to {
            self.tied_to.insert(tied_to);
        }
    }
}

rustc_index::newtype_index! {
    pub struct AliasClass {}
}

// === FunctionFactExplorer === //

#[derive(Debug, Default)]
pub struct FunctionFactExplorer<'tcx> {
    visit_set: FxHashSet<(DefId, &'tcx List<GenericArg<'tcx>>)>,
}

impl<'tcx> FunctionFactExplorer<'tcx> {
    pub fn iter_reachable(
        &mut self,
        tcx: TyCtxt<'tcx>,
        facts: &FunctionFactStore<'tcx>,
        def_id: DefId,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (DefId, &'tcx List<GenericArg<'tcx>>)> + '_ {
        self.visit_set.clear();
        self.iter_reachable_inner(tcx, facts, def_id, args);
        self.visit_set.iter().copied()
    }

    fn iter_reachable_inner(
        &mut self,
        tcx: TyCtxt<'tcx>,
        facts: &FunctionFactStore<'tcx>,
        src_did: DefId,
        src_args: &'tcx List<GenericArg<'tcx>>,
    ) {
        if !self.visit_set.insert((src_did, src_args)) {
            return;
        }

        for (dest_did, dest_args) in facts.lookup(src_did).instantiate_calls(tcx, src_args) {
            let CallConcretizationResult::Concrete(dest_did) = dest_did else {
                continue;
            };

            self.iter_reachable_inner(tcx, facts, dest_did, dest_args);
        }
    }
}
