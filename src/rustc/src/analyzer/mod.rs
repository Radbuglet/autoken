use rustc_hir::{def::DefKind, def_id::DefId};

use rustc_middle::{
    mir::BasicBlock,
    ty::{TyCtxt, TyKind},
};
use rustc_span::Symbol;

use crate::{
    analyzer::{
        facts::{FunctionFactStore, FunctionFactsId},
        sets::{instantiate_set_proc, is_tie_func},
    },
    util::{
        feeder::{
            feed,
            feeders::{MirBuiltFeeder, MirBuiltStasher},
            read_feed,
        },
        graph::{GraphPropagator, GraphPropagatorCx},
        mir::{
            does_have_instance_mir, find_region_with_name, get_static_callee_from_terminator,
            iter_all_local_def_ids, safeishly_grab_local_def_id_mir, TerminalCallKind,
        },
        ty::get_fn_sig_maybe_closure,
    },
};

use self::mir::{TokenKey, TokenMirBuilder};

// === Modules === //

mod facts;
mod mir;
mod sets;
mod sym;

// === Engine === //

pub fn analyze<'tcx>(tcx: TyCtxt<'tcx>) {
    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    for did in iter_all_local_def_ids(tcx) {
        if safeishly_grab_local_def_id_mir(tcx, did).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, did).is_some());
        }
    }

    // Compute propagated function facts
    assert!(!tcx.untracked().definitions.is_frozen());

    // TODO: Clean this up too!
    type Gah<'a, 'b, 'c, 'tcx> =
        GraphPropagatorCx<'a, 'b, &'c mut FunctionFactStore<'tcx>, DefId, FunctionFactsId>;

    let propagator = |cx: &mut Gah<'_, '_, '_, 'tcx>, src_did: DefId| {
        assert!(does_have_instance_mir(tcx, src_did));
        assert!(!is_tie_func(tcx, src_did));

        let src_body = tcx.optimized_mir(src_did);
        let src_facts = cx.context_mut().create(src_did);

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
                                // TODO
                            },
                            Some(&mut |set, mutability| {
                                // TODO
                            }),
                        );

                        continue;
                    } else if does_have_instance_mir(tcx, dest_did) {
                        // TODO
                    }
                }
                Some(TerminalCallKind::Generic(_dest_span, dest_did, dest_args)) => {
                    // TODO
                }
                Some(TerminalCallKind::Dynamic) => {
                    // (ignored)
                }
                None => {
                    // (ignored)
                }
            }
        }

        src_facts
    };

    let mut facts = FunctionFactStore::default();
    let mut propagator = GraphPropagator::new(&mut facts, &propagator);

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();
        if does_have_instance_mir(tcx, did) && !is_tie_func(tcx, did) {
            propagator.analyze(did);
        }
    }

    let fact_map = propagator.into_fact_map();

    // Generate shadow functions for each locally-visited function.
    assert!(!tcx.untracked().definitions.is_frozen());

    let mut shadows = Vec::new();

    for (orig_id, src_facts) in &fact_map {
        let Some(orig_id) = orig_id.as_local() else {
            continue;
        };

        // Modify body
        let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
            // Some `DefIds` with facts are just shims—not functions with actual MIR.
            continue;
        };

        let mut body_mutator = TokenMirBuilder::new(tcx, &mut body);

        for (key, info) in &facts.facts[*src_facts].borrows {
            for tied in &info.tied_to {
                body_mutator.tie_token_to_my_return(TokenKey(*key), *tied);
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
            let Some(target_facts) = fact_map.get(&target_did) else {
                continue;
            };

            // Determine the set of tokens borrowed by this function.
            let mut ensure_not_borrowed = Vec::new();

            for (_ty, ty, info) in
                facts.facts[*src_facts].instantiate_simple_borrows(tcx, target_args)
            {
                ensure_not_borrowed.push((ty, info.mutability, &info.tied_to));
            }

            // TODO: Instantiate generics

            for (ty, mutability, tied) in ensure_not_borrowed.iter().copied() {
                body_mutator.ensure_not_borrowed_at(bb, TokenKey(ty), mutability);

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

                    body_mutator.tie_token_to_its_return(bb, TokenKey(ty), mutability, |region| {
                        region == mapped_region
                    });
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
