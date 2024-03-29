use std::collections::hash_map;

use rustc_hir::{def::DefKind, def_id::DefId};

use rustc_middle::{
    mir::{BasicBlock, Mutability, Terminator, TerminatorKind},
    query::Key,
    ty::{
        GenericArg, GenericArgKind, GenericArgs, GenericParamDefKind, Instance, List, ParamEnv, Ty,
        TyCtxt, TyKind,
    },
};
use rustc_span::{sym::TyKind, Span, Symbol};

use crate::{
    mir::{TokenKey, TokenMirBuilder},
    util::{
        feeder::{
            feed,
            feeders::{MirBuiltFeeder, MirBuiltStasher},
            read_feed,
        },
        hash::{FxHashMap, FxHashSet},
        mir::{
            does_have_instance_mir, find_region_with_name, for_each_unsized_func,
            iter_all_local_def_ids, safeishly_grab_local_def_id_mir,
        },
        ty::{get_fn_sig_maybe_closure, is_annotated_ty},
    },
};

use petgraph::stable_graph::{NodeIndex, StableGraph};

// === Engine === //

#[derive(Debug, Default)]
pub struct AnalysisDriver<'tcx> {
    generic_map: FxHashMap<DefId, NodeIndex>,
    generic_graph: StableGraph<FunctionFacts<'tcx>, &'tcx List<GenericArg<'tcx>>>,
}

#[derive(Debug, Default)]
struct FunctionFacts<'tcx> {
    borrows: FxHashMap<Ty<'tcx>, (Mutability, FxHashSet<Symbol>)>,
    sets: FxHashMap<Ty<'tcx>, Mutability>,
}

impl<'tcx> AnalysisDriver<'tcx> {
    pub fn analyze(&mut self, tcx: TyCtxt<'tcx>) {
        // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
        for did in iter_all_local_def_ids(tcx) {
            if safeishly_grab_local_def_id_mir(tcx, did).is_some() {
                assert!(read_feed::<MirBuiltStasher>(tcx, did).is_some());
            }
        }

        // Compute isolated function facts
        assert!(!tcx.untracked().definitions.is_frozen());

        for did in iter_all_local_def_ids(tcx) {
            let did = did.to_def_id();
            if does_have_instance_mir(tcx, did) && !Self::is_tie_func(tcx, did) {
                self.discover_isolated_func_facts(tcx, did);
            }
        }

        // Compute propagated function facts
        assert!(!tcx.untracked().definitions.is_frozen());
        // dbg!(&self.generic_graph);

        // TODO
    }

    fn discover_isolated_func_facts(&mut self, tcx: TyCtxt<'tcx>, src_did: DefId) -> NodeIndex {
        assert!(does_have_instance_mir(tcx, src_did));
        assert!(!Self::is_tie_func(tcx, src_did));

        // See if we have a node already
        let entry = match self.generic_map.entry(src_did) {
            hash_map::Entry::Vacant(entry) => entry,
            hash_map::Entry::Occupied(entry) => {
                return *entry.into_mut();
            }
        };

        let src_id = self.generic_graph.add_node(FunctionFacts::default());
        entry.insert(src_id);

        let src_body = tcx.optimized_mir(src_did);

        for src_bb in src_body.basic_blocks.iter() {
            let Some(Terminator {
                kind:
                    TerminatorKind::Call {
                        func: dest_func, ..
                    },
                ..
            }) = &src_bb.terminator
            else {
                continue;
            };

            let dest_func = dest_func.ty(&src_body.local_decls, tcx);

            let (dest_did, dest_args) = match dest_func.kind() {
                TyKind::FnPtr(_) => {
                    // (ignored: this is a dynamic call)
                    continue;
                }
                TyKind::FnDef(did, args) => (*did, *args),
                TyKind::Closure(did, args) => (*did, args.as_closure().args),
                _ => unreachable!(),
            };

            let Ok(dest_args) =
                tcx.try_normalize_erasing_regions(ParamEnv::reveal_all(), dest_args)
            else {
                continue;
            };

            let Ok(Some(dest_instance)) = tcx.resolve_instance(
                tcx.erase_regions(ParamEnv::reveal_all().and((dest_did, dest_args))),
            ) else {
                continue;
            };

            let dest_did = dest_instance.def_id();

            if Self::is_tie_func(tcx, dest_did) {
                let facts = &mut self.generic_graph[src_id];

                let tied = dest_args[0].expect_ty();
                let tied = if tied.is_unit() {
                    None
                } else {
                    let first_field = tied.ty_adt_def().unwrap().all_fields().next().unwrap();
                    let first_field = tcx.type_of(first_field.did).skip_binder();
                    let TyKind::Ref(first_field, _pointee, _mut) = first_field.kind() else {
                        unreachable!();
                    };

                    Some(first_field.get_name().unwrap())
                };

                Self::instantiate_set_proc(
                    tcx,
                    src_body.span,
                    tcx.erase_regions_ty(dest_args[1].expect_ty()),
                    &mut |key, mutability| {
                        let (curr_mutability, curr_ties) = facts
                            .borrows
                            .entry(key)
                            .or_insert((mutability, FxHashSet::default()));

                        if let Some(tied) = tied {
                            curr_ties.insert(tied);
                        }

                        *curr_mutability = (*curr_mutability).max(mutability);
                    },
                    Some(&mut |set, mutability| {
                        let curr_mut = facts.sets.entry(set).or_insert(mutability);
                        *curr_mut = (*curr_mut).max(mutability);
                    }),
                );

                continue;
            }

            if !does_have_instance_mir(tcx, dest_did) {
                continue;
            }

            let dst_id = self.discover_isolated_func_facts(tcx, dest_did);

            self.generic_graph.add_edge(src_id, dst_id, dest_args);
        }

        src_id
    }

    fn is_tie_func(tcx: TyCtxt<'tcx>, def_id: DefId) -> bool {
        tcx.opt_item_name(def_id) == Some(sym::__autoken_declare_tied.get())
    }

    fn instantiate_set(
        tcx: TyCtxt<'tcx>,
        span: Span,
        ty: Ty<'tcx>,
        add_generic_union_set: Option<&mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)>,
    ) -> FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)> {
        let mut set = FxHashMap::<Ty<'tcx>, (Mutability, Option<Symbol>)>::default();

        Self::instantiate_set_proc(
            tcx,
            span,
            ty,
            &mut |ty, mutability| match set.entry(ty) {
                hash_map::Entry::Occupied(entry) => {
                    if mutability.is_mut() {
                        entry.into_mut().0 = Mutability::Mut;
                    }
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert((Mutability::Mut, None));
                }
            },
            add_generic_union_set,
        );

        set
    }

    fn instantiate_set_proc(
        tcx: TyCtxt<'tcx>,
        span: Span,
        ty: Ty<'tcx>,
        add_ty: &mut dyn FnMut(Ty<'tcx>, Mutability),
        mut add_generic_union_set: Option<&mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)>,
    ) {
        match ty.kind() {
            // Union
            TyKind::Tuple(fields) => {
                for field in fields.iter() {
                    Self::instantiate_set_proc(
                        tcx,
                        span,
                        field,
                        add_ty,
                        add_generic_union_set.as_deref_mut(),
                    );
                }
            }
            TyKind::Adt(def, generics)
                if is_annotated_ty(def, sym::__autoken_ref_ty_marker.get()) =>
            {
                add_ty(generics[0].as_type().unwrap(), Mutability::Not);
            }
            TyKind::Adt(def, generics)
                if is_annotated_ty(def, sym::__autoken_mut_ty_marker.get()) =>
            {
                add_ty(generics[0].as_type().unwrap(), Mutability::Mut);
            }
            TyKind::Adt(def, generics)
                if is_annotated_ty(def, sym::__autoken_downgrade_ty_marker.get()) =>
            {
                let mut set = Self::instantiate_set(
                    tcx,
                    span,
                    generics[0].as_type().unwrap(),
                    add_generic_union_set
                        .as_deref_mut()
                        .map(|add_generic_union_set| {
                            |set: Ty<'tcx>, _mut: Mutability| {
                                add_generic_union_set(set, Mutability::Not)
                            }
                        })
                        .as_mut()
                        .map(|v| v as &mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)),
                );

                for (mutability, _) in set.values_mut() {
                    *mutability = Mutability::Not;
                }

                for (ty, (mutability, _)) in set {
                    add_ty(ty, mutability);
                }
            }
            TyKind::Adt(def, generics)
                if is_annotated_ty(def, sym::__autoken_diff_ty_marker.get()) =>
            {
                let mut set =
                    Self::instantiate_set(tcx, span, generics[0].as_type().unwrap(), None);

                Self::instantiate_set_proc(
                    tcx,
                    span,
                    generics[1].as_type().unwrap(),
                    &mut |ty, mutability| match set.entry(ty) {
                        hash_map::Entry::Occupied(entry) => {
                            if mutability.is_mut() {
                                entry.remove();
                            } else {
                                entry.into_mut().0 = Mutability::Not;
                            }
                        }
                        hash_map::Entry::Vacant(_) => {}
                    },
                    None,
                );

                for (ty, (mutability, _)) in set {
                    add_ty(ty, mutability);
                }
            }

            TyKind::Param(_) | TyKind::Alias(_, _) => {
                if let Some(add_generic_union_set) = &mut add_generic_union_set {
                    add_generic_union_set(ty, Mutability::Mut);
                } else {
                    tcx.dcx()
                        .span_err(span, "generic sets can only appear in top-level unions");
                }
            }
            _ => unreachable!(),
        }
    }
}

#[allow(non_upper_case_globals)]
mod sym {
    use crate::util::mir::CachedSymbol;

    pub static __autoken_declare_tied: CachedSymbol = CachedSymbol::new("__autoken_declare_tied");

    pub static __autoken_absorb_only: CachedSymbol = CachedSymbol::new("__autoken_absorb_only");

    pub static __autoken_mut_ty_marker: CachedSymbol = CachedSymbol::new("__autoken_mut_ty_marker");

    pub static __autoken_ref_ty_marker: CachedSymbol = CachedSymbol::new("__autoken_ref_ty_marker");

    pub static __autoken_downgrade_ty_marker: CachedSymbol =
        CachedSymbol::new("__autoken_downgrade_ty_marker");

    pub static __autoken_diff_ty_marker: CachedSymbol =
        CachedSymbol::new("__autoken_diff_ty_marker");

    pub static unnamed: CachedSymbol = CachedSymbol::new("unnamed");
}
