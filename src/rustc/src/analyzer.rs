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
        graph::{GraphPropagator, GraphPropagatorCx},
        hash::{FxHashMap, FxHashSet},
        mir::{
            does_have_instance_mir, find_region_with_name, get_static_callee_from_terminator,
            iter_all_local_def_ids, resolve_instance, safeishly_grab_local_def_id_mir,
            TerminalCallKind,
        },
        ty::{get_fn_sig_maybe_closure, is_annotated_ty},
    },
};

// === Engine === //

#[derive(Debug, Clone)]
struct FunctionFacts<'tcx> {
    def_id: DefId,
    borrows: FxHashMap<Ty<'tcx>, (Mutability, FxHashSet<Symbol>)>,
    sets: FxHashMap<Ty<'tcx>, Mutability>,
    generic_calls: FxHashSet<(DefId, &'tcx List<GenericArg<'tcx>>)>,
}

impl<'tcx> FunctionFacts<'tcx> {
    pub fn new(def_id: DefId) -> Self {
        Self {
            def_id,
            borrows: FxHashMap::default(),
            sets: FxHashMap::default(),
            generic_calls: FxHashSet::default(),
        }
    }

    pub fn instantiate_simple_borrows(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>, Mutability, &FxHashSet<Symbol>)> + '_ {
        let instance = Instance::new(self.def_id, args);

        self.borrows
            .iter()
            .map(move |(&generic_ty, (mutability, symbols))| {
                let concrete_ty = instance.instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    ParamEnv::reveal_all(),
                    EarlyBinder::bind(generic_ty),
                );
                (generic_ty, concrete_ty, *mutability, symbols)
            })
    }

    pub fn instantiate_generic_calls(
        &self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> impl Iterator<Item = (DefId, &'tcx List<GenericArg<'tcx>>)> + '_ {
        let instance = Instance::new(self.def_id, args);

        self.generic_calls.iter().map(move |(def_id, args)| {
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

pub fn analyze<'tcx>(tcx: TyCtxt<'tcx>) {
    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    for did in iter_all_local_def_ids(tcx) {
        if safeishly_grab_local_def_id_mir(tcx, did).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, did).is_some());
        }
    }

    // Compute propagated function facts
    assert!(!tcx.untracked().definitions.is_frozen());

    let propagator = |cx: &mut GraphPropagatorCx<(), DefId, FunctionFacts<'tcx>>, src_did| {
        assert!(does_have_instance_mir(tcx, src_did));
        assert!(!is_tie_func(tcx, src_did));

        // Trace call edges
        let src_body = tcx.optimized_mir(src_did);
        let mut src_facts = FunctionFacts::new(src_did);
        let mut analysis_queue = Vec::new();

        for src_bb in src_body.basic_blocks.iter() {
            match get_static_callee_from_terminator(tcx, &src_bb.terminator, &src_body.local_decls)
            {
                Some(TerminalCallKind::Static(dest_span, dest_did, dest_args)) => {
                    if is_tie_func(tcx, dest_did) {
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

                        instantiate_set_proc(
                            tcx,
                            src_body.span,
                            tcx.erase_regions_ty(dest_args[1].expect_ty()),
                            &mut |key, mutability| {
                                let (curr_mutability, curr_ties) = src_facts
                                    .borrows
                                    .entry(key)
                                    .or_insert((mutability, FxHashSet::default()));

                                if let Some(tied) = tied {
                                    curr_ties.insert(tied);
                                }

                                *curr_mutability = (*curr_mutability).max(mutability);
                            },
                            Some(&mut |set, mutability| {
                                let curr_mut = src_facts.sets.entry(set).or_insert(mutability);
                                *curr_mut = (*curr_mut).max(mutability);
                            }),
                        );

                        continue;
                    } else if does_have_instance_mir(tcx, dest_did) {
                        analysis_queue.push((dest_span, dest_did, dest_args));
                    }
                }
                Some(TerminalCallKind::Generic(_dest_span, dest_did, dest_args)) => {
                    src_facts.generic_calls.insert((dest_did, dest_args));
                }
                Some(TerminalCallKind::Dynamic) => {
                    // (ignored)
                }
                None => {
                    // (ignored)
                }
            }
        }

        // Analyze callees
        #[allow(clippy::mutable_key_type)]
        let mut already_analyzed = FxHashSet::default();

        while let Some((dest_span, dest_did, dest_args)) = analysis_queue.pop() {
            // Don't analyze the same callee more than once.
            if !already_analyzed.insert((dest_did, dest_args)) {
                continue;
            };

            // Fetch facts for the destination.
            let Some(dest_facts) = cx.analyze(dest_did) else {
                continue;
            };

            // Inherit borrows
            let mut concrete_to_generic = FxHashMap::default();
            for (generic_ty, concrete_ty, mutability, _ties) in
                dest_facts.instantiate_simple_borrows(tcx, dest_args)
            {
                match concrete_to_generic.entry(concrete_ty) {
                    hash_map::Entry::Occupied(replaced_generic) => {
                        let replaced_generic = replaced_generic.into_mut();
                        tcx.dcx().span_err(
                            dest_span,
                            format!("call unifies two presumed-distinct borrowed tokens {replaced_generic} and {generic_ty} to {concrete_ty}"),
                        );
                    }
                    hash_map::Entry::Vacant(entry) => {
                        entry.insert(generic_ty);
                    }
                }

                let (other_mutability, _) = src_facts
                    .borrows
                    .entry(concrete_ty)
                    .or_insert((mutability, FxHashSet::default()));

                *other_mutability = (*other_mutability).max(mutability);
            }

            // Inherit generic calls
            for (rec_called_did, rec_called_args) in
                dest_facts.instantiate_generic_calls(tcx, dest_args)
            {
                match resolve_instance(tcx, rec_called_did, rec_called_args) {
                    Ok(Some(rec_instance)) => {
                        if does_have_instance_mir(tcx, rec_instance.def_id()) {
                            analysis_queue.push((
                                dest_span,
                                rec_instance.def_id(),
                                rec_instance.args,
                            ));
                        }
                    }
                    Ok(None) => {
                        src_facts
                            .generic_calls
                            .insert((rec_called_did, rec_called_args));
                    }
                    Err(_) => {
                        // (ignored)
                    }
                }
            }
        }

        // TODO: Handle borrow sets and constraints

        src_facts
    };
    let mut propagator = GraphPropagator::new((), &propagator);

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();
        if does_have_instance_mir(tcx, did) && !is_tie_func(tcx, did) {
            propagator.analyze(did);
        }
    }

    let facts = propagator.into_fact_map();

    // Generate shadow functions for each locally-visited function.
    assert!(!tcx.untracked().definitions.is_frozen());

    let mut shadows = Vec::new();

    for (orig_id, src_facts) in &facts {
        let Some(orig_id) = orig_id.as_local() else {
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
            let Some(TerminalCallKind::Static(_target_span, target_did, target_args)) =
                get_static_callee_from_terminator(
                    tcx,
                    &body_mutator.body().basic_blocks[bb].terminator,
                    &body_mutator.body().local_decls,
                )
            else {
                continue;
            };

            // Determine what it borrows
            let Some(target_facts) = facts.get(&target_did) else {
                continue;
            };

            // Determine the set of tokens borrowed by this function.
            let mut ensure_not_borrowed = Vec::new();

            for (_, ty, mutbl, tie) in target_facts.instantiate_simple_borrows(tcx, target_args) {
                ensure_not_borrowed.push((ty, mutbl, tie));
            }

            // TODO: Instantiate generics

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
                shadows.len(),
            )),
            shadow_kind,
        );

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

fn is_tie_func(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    tcx.opt_item_name(def_id) == Some(sym::__autoken_declare_tied.get())
}

fn instantiate_set<'tcx>(
    tcx: TyCtxt<'tcx>,
    span: Span,
    ty: Ty<'tcx>,
    add_generic_union_set: Option<&mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)>,
) -> FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)> {
    let mut set = FxHashMap::<Ty<'tcx>, (Mutability, Option<Symbol>)>::default();

    instantiate_set_proc(
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

fn instantiate_set_proc<'tcx>(
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
                instantiate_set_proc(
                    tcx,
                    span,
                    field,
                    add_ty,
                    add_generic_union_set.as_deref_mut(),
                );
            }
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_ref_ty_marker.get()) => {
            add_ty(generics[0].as_type().unwrap(), Mutability::Not);
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_mut_ty_marker.get()) => {
            add_ty(generics[0].as_type().unwrap(), Mutability::Mut);
        }
        TyKind::Adt(def, generics)
            if is_annotated_ty(def, sym::__autoken_downgrade_ty_marker.get()) =>
        {
            let mut set = instantiate_set(
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
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_diff_ty_marker.get()) => {
            let mut set = instantiate_set(tcx, span, generics[0].as_type().unwrap(), None);

            instantiate_set_proc(
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
