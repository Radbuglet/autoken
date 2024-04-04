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
        hash::{new_const_hash_set, FxHashSet},
        mir::{
            get_static_callee_from_terminator, has_instance_mir, resolve_instance, TerminalCallKind,
        },
    },
};

// === FunctionFacts === //

pub fn has_facts(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    !is_tie_func(tcx, def_id) && has_instance_mir(tcx, def_id)
}

pub type ConcretizationArgs<'tcx> = Option<&'tcx List<GenericArg<'tcx>>>;

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
                    Some(TerminalCallKind::Static(span, called_did, args)) => {
                        if is_tie_func(tcx, called_did) {
                            let tied_to = get_tie(tcx, args[0].expect_ty());

                            // TODO: Populate alias classes.

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
                            entry.known_calls.insert((called_did, args));
                            self.collection_queue.push(called_did);
                        }
                    }
                    Some(TerminalCallKind::Generic(_span, called_did, args)) => {
                        entry
                            .generic_calls
                            .entry((called_did, args))
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

    /// The statically-resolved non-generic functions this function can call.
    pub known_calls: FxHashSet<(DefId, &'tcx List<GenericArg<'tcx>>)>,

    /// The statically-resolved generic functions this function can call and their promises on what
    /// they won't borrow.
    pub generic_calls: FxHashMap<(DefId, &'tcx List<GenericArg<'tcx>>), GenericCallInfo<'tcx>>,

    /// The set of all concrete borrows for this function without accounting for deep calls.
    pub found_borrows: FxHashMap<Ty<'tcx>, BorrowInfo>,

    /// The set of all borrows of generic sets in this function without accounting for deep calls.
    pub generic_borrow_sets: FxHashMap<Ty<'tcx>, BorrowInfo>,

    /// The function's restrictions on types which it assumes not to alias.
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

    pub fn instantiate_arg<T>(&self, tcx: TyCtxt<'tcx>, ty: T, args: ConcretizationArgs<'tcx>) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        if let Some(args) = args {
            Instance::new(self.def_id, args).instantiate_mir_and_normalize_erasing_regions(
                tcx,
                ParamEnv::reveal_all(),
                EarlyBinder::bind(ty),
            )
        } else {
            ty
        }
    }

    pub fn instantiate_args(
        &self,
        tcx: TyCtxt<'tcx>,
        ty: &'tcx List<GenericArg<'tcx>>,
        args: ConcretizationArgs<'tcx>,
    ) -> &'tcx List<GenericArg<'tcx>> {
        tcx.mk_args_from_iter(ty.iter().map(|arg| self.instantiate_arg(tcx, arg, args)))
    }

    pub fn instantiate_known_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = (DefId, &'tcx List<GenericArg<'tcx>>)> + '_ {
        self.known_calls
            .iter()
            .map(move |(did, called_args)| (*did, self.instantiate_args(tcx, called_args, args)))
    }

    pub fn instantiate_generic_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = FactInstantiatedCall<'tcx, '_>> + '_ {
        self.generic_calls
            .iter()
            .filter_map(move |((did, called_args), info)| {
                let called_args = self.instantiate_args(tcx, called_args, args);

                match resolve_instance(tcx, *did, called_args) {
                    Ok(Some(instance)) => Some(FactInstantiatedCall::Concrete {
                        did: instance.def_id(),
                        args: instance.args,
                    }),
                    Ok(None) => Some(FactInstantiatedCall::Generic {
                        did: *did,
                        args: called_args,
                        info,
                    }),
                    Err(_) => None,
                }
            })
    }

    pub fn instantiate_all_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = FactInstantiatedCall<'tcx, '_>> + '_ {
        self.instantiate_known_calls(tcx, args)
            .map(|(did, args)| FactInstantiatedCall::Concrete { did, args })
            .chain(self.instantiate_generic_calls(tcx, args))
    }

    pub fn instantiate_found_borrows(
        &self,
        tcx: TyCtxt<'tcx>,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, &BorrowInfo)> + '_ {
        self.found_borrows
            .iter()
            .map(move |(&ty, borrow_info)| (ty, self.instantiate_arg(tcx, ty, args), borrow_info))
    }

    pub fn instantiate_borrow_sets(
        &self,
        tcx: TyCtxt<'tcx>,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, &BorrowInfo)> + '_ {
        self.generic_borrow_sets
            .iter()
            .map(move |(&ty, borrow_info)| (self.instantiate_arg(tcx, ty, args), borrow_info))
    }

    pub fn instantiate_alias_classes(
        &self,
        tcx: TyCtxt<'tcx>,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, AliasClass)> + '_ {
        self.alias_classes
            .iter()
            .map(move |(&ty, &class)| (ty, self.instantiate_arg(tcx, ty, args), class))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum FactInstantiatedCall<'tcx, 'a> {
    Concrete {
        did: DefId,
        args: &'tcx List<GenericArg<'tcx>>,
    },
    Generic {
        did: DefId,
        args: &'tcx List<GenericArg<'tcx>>,
        info: &'a GenericCallInfo<'tcx>,
    },
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

#[derive(Debug, Default)]
pub struct GenericCallInfo<'tcx> {
    pub does_not_borrow: FxHashMap<Ty<'tcx>, Mutability>,
}

rustc_index::newtype_index! {
    pub struct AliasClass {}
}

// === FactExplorer === //

static EMPTY_TIED_SET: &FxHashSet<Symbol> = &new_const_hash_set();

pub struct FactExplorer<'tcx, 'facts> {
    pub tcx: TyCtxt<'tcx>,
    pub facts: &'facts FunctionFactStore<'tcx>,
    pub reachable: ReachableFactExplorer<'tcx>,
    borrows: FxHashMap<Ty<'tcx>, (Mutability, &'facts FxHashSet<Symbol>)>,
}

impl<'tcx, 'facts> FactExplorer<'tcx, 'facts> {
    pub fn new(tcx: TyCtxt<'tcx>, facts: &'facts FunctionFactStore<'tcx>) -> Self {
        Self {
            tcx,
            facts,
            reachable: ReachableFactExplorer::default(),
            borrows: FxHashMap::default(),
        }
    }

    pub fn iter_borrows(
        &mut self,
        src_def_id: DefId,
        args: ConcretizationArgs<'tcx>,
    ) -> &FxHashMap<Ty<'tcx>, (Mutability, &'facts FxHashSet<Symbol>)> {
        self.borrows.clear();

        for (def_id, args) in self
            .reachable
            .iter_reachable(self.tcx, self.facts, src_def_id, args)
        {
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

                *mutability = (*mutability).max(info.mutability);

                if def_id == src_def_id && !info.tied_to.is_empty() {
                    *set = &info.tied_to;
                }
            }
        }

        &self.borrows
    }
}

#[derive(Default)]
pub struct ReachableFactExplorer<'tcx> {
    visit_set: FxHashSet<(DefId, ConcretizationArgs<'tcx>)>,
}

impl<'tcx> ReachableFactExplorer<'tcx> {
    pub fn iter_reachable(
        &mut self,
        tcx: TyCtxt<'tcx>,
        facts: &FunctionFactStore<'tcx>,
        def_id: DefId,
        args: ConcretizationArgs<'tcx>,
    ) -> impl Iterator<Item = (DefId, ConcretizationArgs<'tcx>)> + '_ {
        self.visit_set.clear();
        self.iter_reachable_inner(tcx, facts, def_id, args);
        self.visit_set.iter().copied()
    }

    fn iter_reachable_inner(
        &mut self,
        tcx: TyCtxt<'tcx>,
        facts: &FunctionFactStore<'tcx>,
        src_did: DefId,
        src_args: ConcretizationArgs<'tcx>,
    ) {
        let Some(src_facts) = facts.lookup(src_did) else {
            return;
        };

        if !self.visit_set.insert((src_did, src_args)) {
            return;
        }

        for dest in src_facts.instantiate_all_calls(tcx, src_args) {
            let FactInstantiatedCall::Concrete {
                did: dest_did,
                args: dest_args,
            } = dest
            else {
                continue;
            };

            self.iter_reachable_inner(tcx, facts, dest_did, Some(dest_args));
        }
    }
}
