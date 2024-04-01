use std::collections::hash_map;

use rustc_hash::FxHashMap;
use rustc_hir::def_id::DefId;
use rustc_index::IndexVec;
use rustc_middle::ty::{EarlyBinder, GenericArg, Instance, List, Mutability, ParamEnv, Ty, TyCtxt};
use rustc_span::{Span, Symbol};

use crate::util::{hash::FxHashSet, ty::is_generic_ty};

rustc_index::newtype_index! {
    pub struct FunctionFactsId {}
}

#[derive(Debug, Default)]
pub struct FunctionFactStore<'tcx> {
    pub facts: IndexVec<FunctionFactsId, FunctionFacts<'tcx>>,
}

impl<'tcx> FunctionFactStore<'tcx> {
    pub fn create(&mut self, def_id: DefId) -> FunctionFactsId {
        self.facts.push(FunctionFacts {
            def_id,
            generic_restrictions: FxHashMap::default(),
            borrows: FxHashMap::default(),
            borrow_sets: FxHashMap::default(),
            generic_calls: FxHashMap::default(),
        })
    }

    pub fn import(
        &mut self,
        tcx: TyCtxt<'tcx>,
        span: Span,
        src: FunctionFactsId,
        dest: FunctionFactsId,
        args: &'tcx List<GenericArg<'tcx>>,
        lookup_did: impl FnMut(&mut Self, DefId) -> FunctionFactsId,
    ) {
        if src == dest {
            todo!();
        }

        let (src_data, dest_data) = self.facts.pick2_mut(src, dest);

        // Import and validate generic restrictions
        {
            let mut concrete_to_generic = FxHashMap::default();

            for (generic_ty, concrete_ty, eq_class) in
                dest_data.instantiate_generic_restrictions(tcx, args)
            {
                match concrete_to_generic.entry(concrete_ty) {
                    hash_map::Entry::Occupied(entry) => {
                        let (other_generic_ty, other_eq_class) = entry.into_mut();

                        if eq_class != *other_eq_class {
                            tcx.dcx().span_err(
                                span,
                                format!(
                                    "this call's generic parameters cause {other_generic_ty} and \
                                     {generic_ty} to unify to {concrete_ty} but the function assumes \
                                     that these types are distinct
                                "),
                            );
                        }
                    }
                    hash_map::Entry::Vacant(entry) => {
                        entry.insert((generic_ty, eq_class));
                    }
                }

                src_data.generic_restrictions.insert(concrete_ty, eq_class);
            }
        }

        // Import basic borrows
        for (_generic_ty, concrete_ty, borrow_info) in
            dest_data.instantiate_simple_borrows(tcx, args)
        {
            src_data.push_borrow(
                concrete_ty,
                borrow_info.mutability,
                borrow_info.tied_to.iter().copied(),
            );
        }

        // Import set borrows
        for (set, borrow_info) in dest_data.instantiate_set_borrows(tcx, args) {
            // TODO
        }

        // Import generic calls
        // TODO
    }
}

#[derive(Debug)]
pub struct FunctionFacts<'tcx> {
    pub def_id: DefId,

    /// Types used in borrows and their permitted equivalence classes.
    pub generic_restrictions: FxHashMap<Ty<'tcx>, u32>,

    /// The tokens the function borrows and the lifetimes they're tied to.
    pub borrows: FxHashMap<Ty<'tcx>, BorrowInfo>,

    /// The sets of generic token set types to eventually union into the borrow set.
    pub borrow_sets: FxHashMap<Ty<'tcx>, BorrowInfo>,

    /// The set of generic functions the function calls and their restrictions on what they are not
    /// allowed to borrow.
    pub generic_calls: FxHashMap<(DefId, &'tcx List<GenericArg<'tcx>>), FxHashSet<Ty<'tcx>>>,
}

impl<'tcx> FunctionFacts<'tcx> {
    pub fn push_borrow(
        &mut self,
        ty: Ty<'tcx>,
        mutability: Mutability,
        tied_to: impl IntoIterator<Item = Symbol>,
    ) {
        match self.borrows.entry(ty) {
            hash_map::Entry::Occupied(entry) => {
                let entry = entry.into_mut();
                entry.mutability = entry.mutability.max(mutability);
                entry.tied_to.extend(tied_to);
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(BorrowInfo {
                    mutability,
                    tied_to: tied_to.into_iter().collect(),
                });
            }
        }
    }

    pub fn instantiate_ty(
        &self,
        tcx: TyCtxt<'tcx>,
        ty: Ty<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> Ty<'tcx> {
        Instance::new(self.def_id, args).instantiate_mir_and_normalize_erasing_regions(
            tcx,
            ParamEnv::reveal_all(),
            EarlyBinder::bind(ty),
        )
    }

    pub fn instantiate_generic_restrictions(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, u32)> + '_ {
        self.generic_restrictions
            .iter()
            .map(move |(&ty, eq_class)| (ty, self.instantiate_ty(tcx, ty, args), *eq_class))
    }

    pub fn instantiate_simple_borrows(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, &BorrowInfo)> + '_ {
        self.borrows
            .iter()
            .map(move |(&ty, borrow_info)| (ty, self.instantiate_ty(tcx, ty, args), borrow_info))
    }

    pub fn instantiate_set_borrows(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (GenericSetInstanceResult<'tcx>, &BorrowInfo)> + '_ {
        self.borrows.iter().map(move |(&ty, borrow_info)| {
            let ty = self.instantiate_ty(tcx, ty, args);

            (
                match is_generic_ty(ty) {
                    true => GenericSetInstanceResult::Concrete(ty),
                    false => GenericSetInstanceResult::Generic(ty),
                },
                borrow_info,
            )
        })
    }

    pub fn instantiate_generic_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (DefId, &'tcx List<GenericArg<'tcx>>)> + '_ {
        let instance = Instance::new(self.def_id, args);

        self.generic_calls.keys().map(move |(def_id, args)| {
            let args = tcx.mk_args_from_iter(args.iter().map(|arg| {
                instance.instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    ParamEnv::reveal_all(),
                    EarlyBinder::bind(arg),
                )
            }));

            (*def_id, args)
        })
    }
}

#[derive(Debug, Clone)]
pub struct BorrowInfo {
    /// The mutability of the borrow (or borrows if this is for a set).
    pub mutability: Mutability,

    /// The lifetimes of the fact's DefId to which this borrow is tied.
    pub tied_to: FxHashSet<Symbol>,
}

#[derive(Debug, Copy, Clone)]
pub enum GenericSetInstanceResult<'tcx> {
    Concrete(Ty<'tcx>),
    Generic(Ty<'tcx>),
}
