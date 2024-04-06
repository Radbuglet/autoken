use std::collections::hash_map;

use rustc_hash::FxHashMap;
use rustc_hir::def::DefKind;
use rustc_middle::{
    mir::BasicBlock,
    ty::{Mutability, Ty, TyCtxt},
};
use rustc_span::Symbol;

use crate::{
    analyzer::facts::{AliasClass, IterBorrowsResult, EMPTY_TIED_SET},
    util::{
        feeder::{
            feed,
            feeders::{MirBuiltFeeder, MirBuiltStasher},
            read_feed,
        },
        mir::{
            get_static_callee_from_terminator, iter_all_local_def_ids,
            safeishly_grab_local_def_id_mir, TerminalCallKind,
        },
        ty::{find_region_with_name, get_fn_sig_maybe_closure, MaybeConcretizedFunc},
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
    let mut facts = FunctionFactStore::new(tcx);

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();

        if has_facts(tcx, did) {
            facts.collect(did);
        }
    }

    facts.optimize();

    // Validate generic assumptions
    let mut class_check_map = FxHashMap::<Ty<'_>, (AliasClass, Ty<'_>)>::default();

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();

        if !has_facts(tcx, did) {
            continue;
        }

        // Ensure that types have no alias.
        // TODO: Give better spans at the exact violation site.
        for called in facts
            .iter_reachable(MaybeConcretizedFunc(did, None))
            .iter_concrete()
        {
            class_check_map.clear();

            for (generic, concrete, class) in facts
                .lookup(called.def_id())
                .unwrap()
                .instantiate_alias_classes(tcx, called.args())
            {
                match class_check_map.entry(concrete) {
                    hash_map::Entry::Occupied(entry) => {
                        let entry = entry.into_mut();
                        if entry.0 != class {
                            tcx.dcx().span_err(
                                tcx.def_span(called.def_id()),
                                format!(
                                    "call instantiates generic types {} and {generic} to {concrete}, \
                                     but the called function assumes that they don't alias",
                                    entry.1,
                                ),
                            );
                        }
                    }
                    hash_map::Entry::Vacant(entry) => {
                        entry.insert((class, generic));
                    }
                }
            }
        }

        // Ensure that provided functions don't borrow illegal objects.
        // TODO
    }

    // Create shadow MIR
    // Generate shadow functions for each locally-visited function.
    assert!(!tcx.untracked().definitions.is_frozen());

    let mut shadows = Vec::new();
    let mut ensure_not_borrowed = Vec::new();

    for orig_did in iter_all_local_def_ids(tcx) {
        if !has_facts(tcx, orig_did.to_def_id()) {
            continue;
        }

        // Modify body
        let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_did).cloned() else {
            // Some `DefIds` with facts are just shims—not functions with actual MIR.
            continue;
        };

        let mut body_mutator = TokenMirBuilder::new(tcx, &mut body);

        // Define tokens for each key
        let tokens_getting_tied =
            facts.iter_used_with_ties(MaybeConcretizedFunc(orig_did.to_def_id(), None));

        for (key, info) in &facts.lookup(orig_did.to_def_id()).unwrap().found_borrows {
            for &tied in &info.tied_to {
                body_mutator.tie_token_to_my_return(TokenKey(*key), tied);
            }
        }

        // Instantiate basic block borrow stubs
        let bb_count = body_mutator.body().basic_blocks.len();
        for bb in 0..bb_count {
            let bb = BasicBlock::from_usize(bb);

            ensure_not_borrowed.clear();

            // Determine the set of tokens borrowed by this function.
            let target_func = match get_static_callee_from_terminator(
                tcx,
                &body_mutator.body().basic_blocks[bb].terminator,
                &body_mutator.body().local_decls,
            ) {
                Some(TerminalCallKind::Static(_target_span, target_func)) => {
                    match facts.iter_borrows(target_func.into()) {
                        IterBorrowsResult::Only(borrows) => {
                            for (&ty, (mutability, tied_to)) in &*borrows {
                                ensure_not_borrowed.push((ty, *mutability, *tied_to));
                            }
                        }
                        IterBorrowsResult::Exclude(exceptions) => {
                            // TODO: Handle tied sets
                            for &key in &*tokens_getting_tied {
                                if let Some(mutability) = exceptions.get(&key) {
                                    if mutability.is_not() {
                                        ensure_not_borrowed.push((
                                            key,
                                            Mutability::Not,
                                            EMPTY_TIED_SET,
                                        ));
                                    }
                                } else {
                                    ensure_not_borrowed.push((
                                        key,
                                        Mutability::Mut,
                                        EMPTY_TIED_SET,
                                    ));
                                }
                            }
                        }
                    }

                    target_func
                }
                Some(TerminalCallKind::Generic(_target_span, target_func)) => {
                    let exceptions = facts.iter_generic_exclusion(
                        facts.lookup(orig_did.to_def_id()).unwrap(),
                        target_func,
                    );

                    // TODO: Handle tied sets
                    for &key in &*tokens_getting_tied {
                        if let Some(mutability) = exceptions.get(&key) {
                            if mutability.is_not() {
                                ensure_not_borrowed.push((key, Mutability::Not, EMPTY_TIED_SET));
                            }
                        } else {
                            ensure_not_borrowed.push((key, Mutability::Mut, EMPTY_TIED_SET));
                        }
                    }

                    target_func
                }
                _ => continue,
            };

            for (ty, mutability, tied) in ensure_not_borrowed.iter().copied() {
                body_mutator.ensure_not_borrowed_at(bb, TokenKey(ty), mutability);

                for &tied in tied {
                    // Compute the type as which the function result is going to be bound.
                    let mapped_region = find_region_with_name(
                        tcx,
                        // N.B. we need to use the monomorphized ID since the non-monomorphized
                        //  ID could just be the parent trait function def, which won't have the
                        //  user's regions.
                        get_fn_sig_maybe_closure(tcx, target_func.def_id())
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
        let shadow_kind = tcx.def_kind(orig_did);
        let shadow_def = tcx.at(body.span).create_def(
            tcx.local_parent(orig_did),
            Symbol::intern(&format!(
                "{}_autoken_shadow_{}",
                tcx.opt_item_name(orig_did.to_def_id())
                    .unwrap_or_else(|| sym::unnamed.get()),
                shadows.len(),
            )),
            shadow_kind,
        );

        feed::<MirBuiltFeeder>(tcx, shadow_def.def_id(), tcx.alloc_steal_mir(body));
        shadow_def.opt_local_def_id_to_hir_id(Some(tcx.local_def_id_to_hir_id(orig_did)));
        shadow_def.visibility(tcx.visibility(orig_did));

        if shadow_kind == DefKind::AssocFn {
            shadow_def.associated_item(tcx.associated_item(orig_did));
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
