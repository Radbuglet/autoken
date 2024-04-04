use rustc_hir::def::DefKind;
use rustc_middle::{mir::BasicBlock, ty::TyCtxt};
use rustc_span::Symbol;

use crate::{
    analyzer::facts::FactExplorer,
    util::{
        feeder::{
            feed,
            feeders::{MirBuiltFeeder, MirBuiltStasher},
            read_feed,
        },
        mir::{
            find_region_with_name, get_static_callee_from_terminator, iter_all_local_def_ids,
            safeishly_grab_local_def_id_mir, TerminalCallKind,
        },
        ty::get_fn_sig_maybe_closure,
    },
};

use self::{
    facts::{has_facts, FunctionFactStore},
    mir::{TokenKey, TokenMirBuilder},
};

mod facts;
mod mir;
mod sets;
mod sym;

pub fn analyze(tcx: TyCtxt<'_>) {
    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    for did in iter_all_local_def_ids(tcx) {
        if safeishly_grab_local_def_id_mir(tcx, did).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, did).is_some());
        }
    }

    // Collect facts for every function
    let mut facts = FunctionFactStore::default();

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();

        if has_facts(tcx, did) {
            facts.collect(tcx, did);
        }
    }

    facts.optimize();

    // Validate generic assumptions
    let mut explorer = FactExplorer::new(tcx, &facts);

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();

        if !has_facts(tcx, did) {
            continue;
        }

        // Ensure that types have no alias.
        // TODO

        // Ensure that provided functions don't borrow illegal objects.
        // TODO
    }

    // Create shadow MIR
    // Generate shadow functions for each locally-visited function.
    assert!(!tcx.untracked().definitions.is_frozen());

    let mut shadows = Vec::new();

    for orig_id in iter_all_local_def_ids(tcx) {
        if !has_facts(tcx, orig_id.to_def_id()) {
            continue;
        }

        // Modify body
        let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
            // Some `DefIds` with facts are just shims—not functions with actual MIR.
            continue;
        };

        let mut body_mutator = TokenMirBuilder::new(tcx, &mut body);

        for (key, info) in &facts.lookup(orig_id.to_def_id()).found_borrows {
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

            // Determine the set of tokens borrowed by this function.
            let mut ensure_not_borrowed = Vec::new();

            for (&ty, (mutability, tied_to)) in explorer.iter_borrows(target_did, Some(target_args))
            {
                ensure_not_borrowed.push((ty, *mutability, *tied_to));
            }

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
