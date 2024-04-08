use std::collections::hash_map;

use rustc_hir::{
    def::DefKind,
    def_id::{DefId, DefIndex, LocalDefId},
    LangItem,
};

use rustc_middle::{
    mir::{BasicBlock, Mutability, Terminator, TerminatorKind},
    ty::{GenericArgs, GenericParamDefKind, Instance, Ty, TyCtxt, TyKind},
};
use rustc_span::Symbol;

use crate::{
    analyzer::sym::unnamed,
    mir::{TokenKey, TokenMirBuilder},
    util::{
        feeder::{
            feed,
            feeders::{
                MirBuiltFeeder, MirBuiltStasher, OptLocalDefIdToHirIdFeeder, VisibilityFeeder,
            },
            read_feed,
        },
        graph::{GraphPropagator, GraphPropagatorCx},
        hash::FxHashMap,
        mir::{
            find_region_with_name, for_each_unsized_func, get_static_callee_from_terminator,
            safeishly_grab_def_id_mir, safeishly_grab_instance_mir, MirGrabResult,
        },
        ty::{get_fn_sig_maybe_closure, is_annotated_ty},
    },
};

pub fn analyze(tcx: TyCtxt<'_>) {
    let id_count = tcx.untracked().definitions.read().def_index_count();

    let mut id_gen = 0;

    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    //
    // N.B. we use this instead of `iter_local_def_id` to avoid freezing the definition map.
    for i in 0..id_count {
        let local_def = LocalDefId {
            local_def_index: DefIndex::from_usize(i),
        };

        if safeishly_grab_def_id_mir(tcx, local_def).is_some() {
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

    for i in 0..id_count {
        let local_def = LocalDefId {
            local_def_index: DefIndex::from_usize(i),
        };

        // Ensure that we're analyzing a function...
        if !matches!(tcx.def_kind(local_def), DefKind::Fn | DefKind::AssocFn) {
            continue;
        }

        // ...which can be properly monomorphized.
        let mut args_wf = true;
        let args =
            // N.B. we use `for_item` instead of `tcx.generics_of` to ensure that we also iterate
            // over the generic arguments of the parent.
            GenericArgs::for_item(tcx, local_def.to_def_id(), |param, _| match param.kind {
                // We can handle these
                GenericParamDefKind::Lifetime => tcx.lifetimes.re_erased.into(),
                GenericParamDefKind::Const {
                    is_host_effect: true,
                    ..
                } => tcx.consts.true_.into(),

                // We can't handle these; return a dummy value and set the `args_wf` flag.
                GenericParamDefKind::Type { .. } => {
                    args_wf = false;
                    tcx.types.unit.into()
                }
                GenericParamDefKind::Const { .. } => {
                    args_wf = false;
                    tcx.consts.true_.into()
                }
            });

        if !args_wf {
            continue;
        }

        let instance = Instance::new(local_def.to_def_id(), args);
        if !should_analyze(tcx, instance) {
            continue;
        }

        func_facts.context_mut().analysis_queue.push(instance);
    }

    while let Some(instance) = func_facts.context_mut().analysis_queue.pop() {
        func_facts.analyze(instance);
    }

    let func_facts = func_facts.into_fact_map();

    // Check for undeclared unsizing.
    for instance in func_facts.keys().copied() {
        let MirGrabResult::Found(body) = safeishly_grab_instance_mir(tcx, instance.def) else {
            continue;
        };

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

        for_each_unsized_func(tcx, instance, body, |instance| {
            ensure_no_borrow(tcx, &func_facts, instance, "unsize this function")
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
                body_mutator.tie_token_to_my_return(TokenKey::Ty(*key), *tied);
            }
        }

        let bb_count = body_mutator.body().basic_blocks.len();
        for bb in 0..bb_count {
            let bb = BasicBlock::from_usize(bb);

            // If it has a concrete callee...
            let Some(Terminator {
                kind: TerminatorKind::Call { func: callee, .. },
                ..
            }) = &body_mutator.body().basic_blocks[bb].terminator
            else {
                continue;
            };

            let Some(target_instance_mono) = get_static_callee_from_terminator(
                tcx,
                instance,
                &body_mutator.body().local_decls,
                callee,
            ) else {
                continue;
            };

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
                body_mutator.ensure_not_borrowed_at(bb, TokenKey::Ty(ty), mutability);

                if let Some(tied) = tied {
                    // Compute the type as which the function result is going to be bound.
                    let mapped_region = find_region_with_name(
                        tcx,
                        // N.B. we need to use the monomorphized ID since the non-monomorphized
                        //  ID could just be the parent trait function def, which won't have the
                        //  user's regions.
                        get_fn_sig_maybe_closure(tcx, target_instance_mono.def_id())
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
                "{}_autoken_shadow_{id_gen}",
                tcx.opt_item_name(orig_id.to_def_id())
                    .unwrap_or_else(|| unnamed.get()),
            )),
            shadow_kind,
        );
        id_gen += 1;

        feed::<MirBuiltFeeder>(tcx, shadow_def.def_id(), tcx.alloc_steal_mir(body));
        feed::<OptLocalDefIdToHirIdFeeder>(
            tcx,
            shadow_def.def_id(),
            Some(tcx.local_def_id_to_hir_id(orig_id)),
        );
        feed::<VisibilityFeeder>(tcx, shadow_def.def_id(), tcx.visibility(orig_id));

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

fn ensure_no_borrow<'tcx>(
    tcx: TyCtxt<'tcx>,
    func_facts: &FxHashMap<Instance<'tcx>, FuncFacts<'tcx>>,
    instance: Instance<'tcx>,
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
                    .keys()
                    .map(|k| k.to_string())
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
    analysis_queue: Vec<Instance<'tcx>>,
}

fn should_analyze<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> bool {
    matches!(
        safeishly_grab_instance_mir(tcx, instance.def),
        MirGrabResult::Found(_)
    )
}

// FIXME: Ensure that facts collected after a self-recursive function was analyzed are also
//  propagated to it.
fn analyze_fn_facts<'tcx>(
    cx: &mut GraphPropagatorCx<'_, '_, FnFactAnalysisCx<'tcx>, Instance<'tcx>, FuncFacts<'tcx>>,
    instance: Instance<'tcx>,
) -> FuncFacts<'tcx> {
    let tcx = cx.context_mut().tcx;

    // If this function has a hardcoded fact set, use those.
    if is_tie_func(tcx, instance.def_id()) {
        return FuncFacts {
            borrows: instantiate_set(tcx, instance.args[1].as_type().unwrap()),
        };
    };

    // Acquire the function body.
    let MirGrabResult::Found(body) = safeishly_grab_instance_mir(tcx, instance.def) else {
        unreachable!();
    };

    // Ensure that we analyze the facts of each unsized function since unsize-checking depends
    // on this information being available.
    for_each_unsized_func(tcx, instance, body, |instance| {
        if should_analyze(tcx, instance) {
            cx.context_mut().analysis_queue.push(instance);
        }
    });

    // See who the function may call and where.
    let mut borrows = FxHashMap::default();

    for bb in body.basic_blocks.iter() {
        // If the terminator is a call terminator.
        let Some(Terminator {
            kind: TerminatorKind::Call { func: callee, .. },
            ..
        }) = &bb.terminator
        else {
            continue;
        };

        let Some(target_instance) =
            get_static_callee_from_terminator(tcx, &instance, &body.local_decls, callee)
        else {
            continue;
        };

        // Recurse into its callee.
        if !should_analyze(tcx, target_instance) {
            continue;
        }

        let Some(target_facts) = cx.analyze(target_instance) else {
            continue;
        };

        let lt_id = is_tie_func(tcx, target_instance.def_id()).then(|| {
            let param = target_instance.args[0].as_type().unwrap();
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
    if tcx.opt_item_name(instance.def_id()) == Some(sym::__autoken_absorb_only.get()) {
        instantiate_set_proc(
            tcx,
            instance.args[0].as_type().unwrap(),
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

fn is_tie_func(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    tcx.opt_item_name(def_id) == Some(sym::__autoken_declare_tied.get())
}

fn instantiate_set<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
) -> FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)> {
    let mut set = FxHashMap::<Ty<'tcx>, (Mutability, Option<Symbol>)>::default();

    instantiate_set_proc(tcx, ty, &mut |ty, mutability| match set.entry(ty) {
        hash_map::Entry::Occupied(entry) => {
            if mutability.is_mut() {
                entry.into_mut().0 = Mutability::Mut;
            }
        }
        hash_map::Entry::Vacant(entry) => {
            entry.insert((mutability, None));
        }
    });

    set
}

fn instantiate_set_proc<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    add: &mut impl FnMut(Ty<'tcx>, Mutability),
) {
    match ty.kind() {
        // Union
        TyKind::Tuple(fields) => {
            for field in fields.iter() {
                instantiate_set_proc(tcx, field, add);
            }
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_ref_ty_marker.get()) => {
            add(generics[0].as_type().unwrap(), Mutability::Not);
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_mut_ty_marker.get()) => {
            add(generics[0].as_type().unwrap(), Mutability::Mut);
        }
        TyKind::Adt(def, generics)
            if is_annotated_ty(def, sym::__autoken_downgrade_ty_marker.get()) =>
        {
            let mut set = instantiate_set(tcx, generics[0].as_type().unwrap());

            for (mutability, _) in set.values_mut() {
                *mutability = Mutability::Not;
            }

            for (ty, (mutability, _)) in set {
                add(ty, mutability);
            }
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_diff_ty_marker.get()) => {
            let mut set = instantiate_set(tcx, generics[0].as_type().unwrap());

            fn remover_func<'set, 'tcx>(
                set: &'set mut FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)>,
            ) -> impl FnMut(Ty<'tcx>, Mutability) + 'set {
                |ty, mutability| match set.entry(ty) {
                    hash_map::Entry::Occupied(entry) => {
                        if mutability.is_mut() {
                            entry.remove();
                        } else {
                            entry.into_mut().0 = Mutability::Not;
                        }
                    }
                    hash_map::Entry::Vacant(_) => {}
                }
            }

            instantiate_set_proc(
                tcx,
                generics[1].as_type().unwrap(),
                &mut remover_func(&mut set),
            );

            for (ty, (mutability, _)) in set {
                add(ty, mutability);
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
