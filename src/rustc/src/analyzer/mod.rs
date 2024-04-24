use std::collections::hash_map;

use rustc_hir::{def::DefKind, LangItem};

use rustc_middle::{
    mir::{BasicBlock, Mutability},
    ty::{InstanceDef, Ty, TyCtxt, TyKind},
};
use rustc_span::Symbol;

use crate::util::{
    feeder::{
        feed,
        feeders::{
            AssociatedItemFeeder, DefKindFeeder, MirBuiltFeeder, MirBuiltStasher,
            OptLocalDefIdToHirIdFeeder, VisibilityFeeder,
        },
        read_feed,
    },
    graph::{GraphPropagator, GraphPropagatorCx},
    hash::FxHashMap,
    mir::{
        for_each_concrete_unsized_func, get_callee_from_terminator, iter_all_local_def_ids,
        try_grab_base_mir_of_def_id, try_grab_optimized_mir_of_instance, TerminalCallKind,
    },
    ty::{
        find_region_with_name, get_fn_sig_maybe_closure, try_resolve_mono_args_for_func,
        MaybeConcretizedFunc,
    },
};

use self::{
    mir::{TokenKey, TokenMirBuilder},
    sets::{instantiate_set, instantiate_set_proc, is_absorb_func, is_tie_func},
};

mod mir;
mod sets;
mod sym;

pub fn analyze(tcx: TyCtxt<'_>) {
    let mut id_gen = 0;

    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    for local_def in iter_all_local_def_ids(tcx) {
        if try_grab_base_mir_of_def_id(tcx, local_def).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, local_def).is_some());
        }
    }

    // Get the token use sets of each function.
    let mut func_facts = GraphPropagator::new(
        FnFactAnalysisCx {
            tcx,
            analysis_queue: Vec::new(),
        },
        &analyze_fn_facts,
    );
    assert!(!tcx.untracked().definitions.is_frozen());

    for did in iter_all_local_def_ids(tcx) {
        // Ensure that we're analyzing a function...
        if !matches!(tcx.def_kind(did), DefKind::Fn | DefKind::AssocFn) {
            continue;
        }

        // If it is a function, analyze it.
        let func = MaybeConcretizedFunc {
            def: InstanceDef::Item(did.to_def_id()),
            args: None,
        };

        if try_resolve_mono_args_for_func(tcx, did.to_def_id()).is_some()
            && should_analyze(tcx, func)
        {
            func_facts.context_mut().analysis_queue.push(func);
        }
    }

    while let Some(instance) = func_facts.context_mut().analysis_queue.pop() {
        func_facts.analyze(instance);
    }

    let func_facts = func_facts.into_fact_map();

    // Check for undeclared unsizing.
    for instance in func_facts.keys().copied() {
        let body = try_grab_optimized_mir_of_instance(tcx, instance.def).unwrap();

        if tcx.entry_fn(()).map(|(did, _)| did) == Some(instance.def_id()) {
            ensure_no_borrow(tcx, &func_facts, instance, "use this main function");
        }

        if tcx.def_kind(instance.def_id()) == DefKind::AssocFn
            && tcx
                .associated_item(instance.def_id())
                .trait_item_def_id
                .map(|method_did| tcx.parent(method_did))
                == Some(tcx.require_lang_item(LangItem::Drop, None))
        {
            ensure_no_borrow(
                tcx,
                &func_facts,
                instance,
                "use this method as a destructor",
            );
        }

        for_each_concrete_unsized_func(tcx, instance, body, |instance| {
            ensure_no_borrow(tcx, &func_facts, instance.into(), "unsize this function")
        });
    }

    // Generate shadow functions for each locally-visited function.
    assert!(!tcx.untracked().definitions.is_frozen());

    let mut shadows = Vec::new();

    for (instance, facts) in &func_facts {
        let Some(orig_id) = instance.def_id().as_local() else {
            continue;
        };

        // Modify body
        let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
            // Some `DefIds` with facts are just shims—not functions with actual MIR.
            continue;
        };

        let mut body_mutator = TokenMirBuilder::new(tcx, &mut body);

        for (key, (_, tied)) in &facts.borrows {
            if let Some(tied) = tied {
                body_mutator.tie_token_to_my_return(TokenKey(*key), *tied);
            }
        }

        let bb_count = body_mutator.body().basic_blocks.len();
        for bb in 0..bb_count {
            let bb = BasicBlock::from_usize(bb);

            // If it has a concrete callee...
            let Some(TerminalCallKind::Static(_, target_instance_mono)) =
                get_callee_from_terminator(
                    tcx,
                    *instance,
                    &body_mutator.body().basic_blocks[bb].terminator,
                    &body_mutator.body().local_decls,
                )
            else {
                continue;
            };
            let target_instance_mono = MaybeConcretizedFunc::from(target_instance_mono);

            // Determine what it borrows
            let Some(callee_borrows) = &func_facts.get(&target_instance_mono) else {
                // This could happen if the optimized MIR reveals that a given function is
                // unreachable.
                continue;
            };

            // Determine the set of tokens borrowed by this function.
            let mut ensure_not_borrowed = Vec::new();

            for (ty, (mutbl, tie)) in &callee_borrows.borrows {
                ensure_not_borrowed.push((*ty, *mutbl, *tie));
            }

            for (ty, mutability, tied) in ensure_not_borrowed.iter().copied() {
                body_mutator.ensure_not_borrowed_at(bb, TokenKey(ty), mutability);

                if let Some(tied) = tied {
                    // Compute the type as which the function result is going to be bound.
                    let Ok(mapped_region) = find_region_with_name(
                        tcx,
                        // N.B. we need to use the monomorphized ID since the non-monomorphized
                        //  ID could just be the parent trait function def, which won't have the
                        //  user's regions.
                        get_fn_sig_maybe_closure(tcx, target_instance_mono.def_id())
                            .skip_binder()
                            .skip_binder()
                            .output(),
                        tied,
                    ) else {
                        // TODO: Log here just in case.
                        continue;
                    };

                    body_mutator.tie_token_to_its_return(bb, TokenKey(ty), mutability, |region| {
                        region == mapped_region
                    });
                }
            }
        }

        drop(body_mutator);

        // Feed the query system the shadow function's properties.
        let shadow_kind = tcx.def_kind(orig_id);
        let shadow_def = tcx
            .create_def(
                tcx.local_parent(orig_id),
                Symbol::intern(&format!(
                    "{}_autoken_shadow_{id_gen}",
                    tcx.opt_item_name(orig_id.to_def_id())
                        .unwrap_or_else(|| sym::unnamed.get()),
                )),
                shadow_kind,
            )
            .def_id();
        id_gen += 1;

        feed::<DefKindFeeder>(tcx, shadow_def, shadow_kind);
        feed::<MirBuiltFeeder>(tcx, shadow_def, tcx.alloc_steal_mir(body));
        feed::<OptLocalDefIdToHirIdFeeder>(
            tcx,
            shadow_def,
            Some(tcx.local_def_id_to_hir_id(orig_id)),
        );
        feed::<VisibilityFeeder>(tcx, shadow_def, tcx.visibility(orig_id));

        if shadow_kind == DefKind::AssocFn {
            feed::<AssociatedItemFeeder>(tcx, shadow_def, tcx.associated_item(orig_id));
        }

        // ...and queue it up for borrow checking!
        shadows.push(shadow_def);
    }

    // Finally, borrow check everything in a single go to avoid issues with stolen values.
    for shadow in shadows {
        let _ = tcx.mir_borrowck(shadow);
    }
}

fn ensure_no_borrow<'tcx>(
    tcx: TyCtxt<'tcx>,
    func_facts: &FxHashMap<MaybeConcretizedFunc<'tcx>, FuncFacts<'tcx>>,
    instance: MaybeConcretizedFunc<'tcx>,
    action: &str,
) {
    let Some(facts) = func_facts.get(&instance) else {
        return;
    };

    if !facts.borrows.is_empty() {
        tcx.sess.dcx().span_err(
            tcx.def_span(instance.def_id()),
            format!(
                "cannot {action} because it borrows {}",
                facts
                    .borrows
                    .iter()
                    .map(|(k, (m, _))| format!("{k} {}", m.mutably_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
}

#[derive(Debug, Clone)]
struct FuncFacts<'tcx> {
    borrows: FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)>,
}

struct FnFactAnalysisCx<'tcx> {
    tcx: TyCtxt<'tcx>,
    analysis_queue: Vec<MaybeConcretizedFunc<'tcx>>,
}

fn should_analyze<'tcx>(tcx: TyCtxt<'tcx>, instance: MaybeConcretizedFunc<'tcx>) -> bool {
    if is_tie_func(tcx, instance.def_id()) || is_absorb_func(tcx, instance.def_id()) {
        instance.args.is_some()
    } else {
        try_grab_optimized_mir_of_instance(tcx, instance.def).is_found()
    }
}

fn analyze_fn_facts<'tcx>(
    cx: &mut GraphPropagatorCx<
        '_,
        '_,
        FnFactAnalysisCx<'tcx>,
        MaybeConcretizedFunc<'tcx>,
        FuncFacts<'tcx>,
    >,
    instance: MaybeConcretizedFunc<'tcx>,
) -> FuncFacts<'tcx> {
    let tcx = cx.context_mut().tcx;

    assert!(should_analyze(tcx, instance));

    // If this function has a hardcoded fact set, use those.
    if is_tie_func(tcx, instance.def_id()) {
        return FuncFacts {
            borrows: instantiate_set(tcx, instance.args.unwrap()[1].as_type().unwrap()),
        };
    };

    // Acquire the function body.
    let body = try_grab_optimized_mir_of_instance(tcx, instance.def).unwrap();

    // Ensure that we analyze the facts of each unsized function since unsize-checking depends
    // on this information being available.
    for_each_concrete_unsized_func(tcx, instance, body, |instance| {
        let instance = instance.into();

        if should_analyze(tcx, instance) {
            cx.context_mut().analysis_queue.push(instance);
        }
    });

    // See who the function may call and where.
    let mut borrows = FxHashMap::default();

    for bb in body.basic_blocks.iter() {
        // If the terminator is a call terminator.
        let Some(TerminalCallKind::Static(_, target_instance)) =
            get_callee_from_terminator(tcx, instance, &bb.terminator, &body.local_decls)
        else {
            continue;
        };
        let target_instance = target_instance.into();

        // Recurse into its callee.
        if !should_analyze(tcx, target_instance) {
            continue;
        }

        let Some(target_facts) = cx.analyze(target_instance) else {
            continue;
        };

        let lt_id = is_tie_func(tcx, target_instance.def_id()).then(|| {
            let param = target_instance.args.unwrap()[0].as_type().unwrap();
            if param.is_unit() {
                return None;
            }

            let first_field = param.ty_adt_def().unwrap().all_fields().next().unwrap();
            let first_field = tcx.type_of(first_field.did).skip_binder();
            let TyKind::Ref(first_field, _pointee, _mut) = first_field.kind() else {
                unreachable!();
            };

            Some(first_field.get_name().unwrap())
        });

        for (borrow_key, (borrow_mut, _)) in &target_facts.borrows {
            let (curr_mut, curr_lt) = borrows
                .entry(*borrow_key)
                .or_insert((Mutability::Not, None));

            if borrow_mut.is_mut() {
                *curr_mut = Mutability::Mut;
            }

            if let Some(Some(lt_id)) = lt_id {
                *curr_lt = Some(lt_id);
            }
        }
    }

    // Now, apply the absorption rules.
    if is_absorb_func(tcx, instance.def_id()) {
        instantiate_set_proc(
            tcx,
            instance.args.unwrap()[0].as_type().unwrap(),
            &mut |ty, mutability| match borrows.entry(ty) {
                hash_map::Entry::Occupied(entry) => {
                    if mutability.is_mut() || entry.get().0 == Mutability::Not {
                        entry.remove();
                    }
                }
                hash_map::Entry::Vacant(_) => {}
            },
        );
    }

    FuncFacts { borrows }
}
