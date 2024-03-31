use std::collections::hash_map;

use rustc_hir::{def::DefKind, def_id::DefId};

use rustc_middle::{
    mir::{BasicBlock, Mutability},
    ty::{EarlyBinder, GenericArg, Instance, List, ParamEnv, Ty, TyCtxt, TyKind},
};
use rustc_span::{Span, Symbol};

use crate::{
    mir::{TokenKey, TokenMirBuilder},
    util::{
        feeder::{
            feed,
            feeders::{MirBuiltFeeder, MirBuiltStasher},
            read_feed,
        },
        graph::propagate_graph,
        hash::{FxHashMap, FxHashSet},
        mir::{
            does_have_instance_mir, find_region_with_name, get_static_callee_from_terminator,
            iter_all_local_def_ids, resolve_instance, safeishly_grab_local_def_id_mir,
            TerminalCallKind,
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
    id_gen: u64,
}

#[derive(Debug, Clone)]
struct FunctionFacts<'tcx> {
    def_id: DefId,
    borrows: FxHashMap<Ty<'tcx>, (Mutability, FxHashSet<Symbol>)>,
    sets: FxHashMap<Ty<'tcx>, Mutability>,
    generic_calls: FxHashSet<(DefId, &'tcx List<GenericArg<'tcx>>)>,
}

impl FunctionFacts<'_> {
    pub fn new(def_id: DefId) -> Self {
        Self {
            def_id,
            borrows: FxHashMap::default(),
            sets: FxHashMap::default(),
            generic_calls: FxHashSet::default(),
        }
    }
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

        propagate_graph(
            &mut self.generic_graph,
            // merge_into
            |graph, edge, caller, called| {
                let caller_instance = Instance::new(graph[caller].def_id, graph[edge]);

                let (caller_data, called_data) = graph.index_twice_mut(caller, called);

                for (ty, (mutability, _)) in &called_data.borrows {
                    let ty = caller_instance.instantiate_mir_and_normalize_erasing_regions(
                        tcx,
                        ParamEnv::reveal_all(),
                        EarlyBinder::bind(*ty),
                    );

                    let (other_mutability, _) = caller_data
                        .borrows
                        .entry(ty)
                        .or_insert((*mutability, FxHashSet::default()));

                    *other_mutability = (*other_mutability).max(*mutability);
                }

                for (called_did, called_args) in &called_data.generic_calls {
                    // TODO: Handle this:
                    // let called_args = tcx.mk_args_from_iter(called_args.iter().map(|arg| {
                    //     caller_instance.instantiate_mir_and_normalize_erasing_regions(
                    //         tcx,
                    //         ParamEnv::reveal_all(),
                    //         EarlyBinder::bind(arg),
                    //     )
                    // }));
                    //
                    // match resolve_instance(tcx, *called_did, called_args) {
                    //     Ok(Some(called)) => {
                    //         let called_did = called.def_id();
                    //         let called_args = called.args;
                    //
                    //         edges_to_add.push((caller, self.generic_map[&called_did], called_args));
                    //     }
                    //     Ok(None) => {
                    //         caller_data.generic_calls.insert((*called_did, called_args));
                    //     }
                    //     Err(_) => {
                    //         // (ignored)
                    //     }
                    // }
                }

                // TODO: Handle borrow sets and constraints
            },
            // replicate
            |graph, into, from| {
                graph[into] = graph[from].clone();
            },
        );

        // Generate shadow functions for each locally-visited function.
        assert!(!tcx.untracked().definitions.is_frozen());

        let mut shadows = Vec::new();

        for src_facts in self.generic_graph.node_weights() {
            let Some(orig_id) = src_facts.def_id.as_local() else {
                continue;
            };

            // Modify body
            let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
                // Some `DefIds` with facts are just shims—not functions with actual MIR.
                continue;
            };

            let mut body_mutator = TokenMirBuilder::new(tcx, &mut body);

            for (key, (_, tied)) in &src_facts.borrows {
                for tied in tied {
                    body_mutator.tie_token_to_my_return(TokenKey::Ty(*key), *tied);
                }
            }

            let bb_count = body_mutator.body().basic_blocks.len();
            for bb in 0..bb_count {
                let bb = BasicBlock::from_usize(bb);

                // Fetch static callee
                let Some(TerminalCallKind::Static(target_did, target_args)) =
                    get_static_callee_from_terminator(
                        tcx,
                        &body_mutator.body().basic_blocks[bb].terminator,
                        &body_mutator.body().local_decls,
                    )
                else {
                    continue;
                };

                let Some(&target_id) = self.generic_map.get(&target_did) else {
                    continue;
                };

                // Determine what it borrows
                let target_facts = &self.generic_graph[target_id];

                // Determine the set of tokens borrowed by this function.
                let mut ensure_not_borrowed = Vec::new();
                let callee_instance = Instance::new(target_did, target_args);

                for (ty, (mutbl, tie)) in &target_facts.borrows {
                    let ty = callee_instance.instantiate_mir_and_normalize_erasing_regions(
                        tcx,
                        ParamEnv::reveal_all(),
                        EarlyBinder::bind(*ty),
                    );
                    ensure_not_borrowed.push((ty, *mutbl, tie));
                }

                for (ty, mutability, tied) in ensure_not_borrowed.iter().copied() {
                    body_mutator.ensure_not_borrowed_at(bb, TokenKey::Ty(ty), mutability);

                    for &tied in tied {
                        // Compute the type as which the function result is going to be bound.
                        let mapped_region = find_region_with_name(
                            tcx,
                            // N.B. we need to use the monomorphized ID since the non-monomorphized
                            //  ID could just be the parent trait function def, which won't have the
                            //  user's regions.
                            get_fn_sig_maybe_closure(tcx, target_did)
                                .skip_binder()
                                .skip_binder()
                                .output(),
                            tied,
                        )
                        .unwrap();

                        body_mutator.tie_token_to_its_return(
                            bb,
                            TokenKey::Ty(ty),
                            mutability,
                            |region| region == mapped_region,
                        );
                    }
                }
            }

            drop(body_mutator);

            // Feed the query system the shadow function's properties.
            let shadow_kind = tcx.def_kind(orig_id);
            let shadow_def = tcx.at(body.span).create_def(
                tcx.local_parent(orig_id),
                Symbol::intern(&format!(
                    "{}_autoken_shadow_{}",
                    tcx.opt_item_name(orig_id.to_def_id())
                        .unwrap_or_else(|| sym::unnamed.get()),
                    self.id_gen,
                )),
                shadow_kind,
            );
            self.id_gen += 1;

            feed::<MirBuiltFeeder>(tcx, shadow_def.def_id(), tcx.alloc_steal_mir(body));
            shadow_def.opt_local_def_id_to_hir_id(Some(tcx.local_def_id_to_hir_id(orig_id)));
            shadow_def.visibility(tcx.visibility(orig_id));

            if shadow_kind == DefKind::AssocFn {
                shadow_def.associated_item(tcx.associated_item(orig_id));
            }

            // ...and queue it up for borrow checking!
            shadows.push(shadow_def);
        }

        // Finally, borrow check everything in a single go to avoid issues with stolen values.
        for shadow in shadows {
            // dbg!(shadow.def_id(), tcx.mir_built(shadow.def_id()));
            let _ = tcx.mir_borrowck(shadow.def_id());
        }
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

        let src_id = self.generic_graph.add_node(FunctionFacts::new(src_did));
        entry.insert(src_id);

        let src_body = tcx.optimized_mir(src_did);

        for src_bb in src_body.basic_blocks.iter() {
            match get_static_callee_from_terminator(tcx, &src_bb.terminator, &src_body.local_decls)
            {
                Some(TerminalCallKind::Static(dest_did, dest_args)) => {
                    if Self::is_tie_func(tcx, dest_did) {
                        let facts = &mut self.generic_graph[src_id];

                        let tied = dest_args[0].expect_ty();
                        let tied = if tied.is_unit() {
                            None
                        } else {
                            let first_field =
                                tied.ty_adt_def().unwrap().all_fields().next().unwrap();
                            let first_field = tcx.type_of(first_field.did).skip_binder();
                            let TyKind::Ref(first_field, _pointee, _mut) = first_field.kind()
                            else {
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
                Some(TerminalCallKind::Generic(dest_did, dest_args)) => {
                    self.generic_graph[src_id]
                        .generic_calls
                        .insert((dest_did, dest_args));
                }
                Some(TerminalCallKind::Dynamic) => {
                    // (ignored)
                }
                None => {
                    // (ignored)
                }
            }
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
