use std::collections::hash_map;

use rustc_middle::ty::{Instance, Mutability, ParamEnv, Ty, TyCtxt};
use rustc_span::Symbol;

use crate::{
    analyzer::sets::{
        instantiate_set, instantiate_set_proc, is_absorb_func, is_tie_func, parse_tie_func,
    },
    util::{
        graph::{GraphPropagator, GraphPropagatorCx},
        hash::FxHashMap,
        mir::{
            for_each_concrete_unsized_func, get_callee_from_terminator, has_optimized_mir,
            iter_all_local_def_ids, try_grab_optimized_mir_of_instance, TerminalCallKind,
        },
        ty::try_resolve_mono_args_for_func,
    },
};

// === Analyzer === //

#[derive(Debug, Clone)]
pub struct TraceFacts<'tcx> {
    pub facts: FxHashMap<Instance<'tcx>, TracedFuncFacts<'tcx>>,
}

#[derive(Debug, Clone)]
pub struct TracedFuncFacts<'tcx> {
    pub borrows: FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)>,
}

impl<'tcx> TraceFacts<'tcx> {
    pub fn compute(tcx: TyCtxt<'tcx>) -> Self {
        let mut facts = GraphPropagator::new(
            TraceCx {
                tcx,
                analysis_queue: Vec::new(),
            },
            &analyze_fn_facts,
        );

        for did in iter_all_local_def_ids(tcx) {
            let did = did.to_def_id();

            if !has_optimized_mir(tcx, did) {
                continue;
            }

            let Some(args) = try_resolve_mono_args_for_func(tcx, did) else {
                continue;
            };

            let instance = Instance::new(did, args);

            if !should_analyze(tcx, instance) {
                continue;
            }

            facts.cx_mut().analysis_queue.push(instance);
        }

        while let Some(next) = facts.cx_mut().analysis_queue.pop() {
            facts.analyze(next);
        }

        Self {
            facts: facts.into_fact_map(),
        }
    }

    pub fn facts(&self, instance: Instance<'tcx>) -> Option<&TracedFuncFacts<'tcx>> {
        self.facts.get(&instance)
    }
}

// === Trace routine === //

struct TraceCx<'tcx> {
    tcx: TyCtxt<'tcx>,
    analysis_queue: Vec<Instance<'tcx>>,
}

fn should_analyze<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> bool {
    try_grab_optimized_mir_of_instance(tcx, instance.def).is_found()
}

fn analyze_fn_facts<'tcx>(
    cx: &mut GraphPropagatorCx<TraceCx<'tcx>, Instance<'tcx>, TracedFuncFacts<'tcx>>,
    instance: Instance<'tcx>,
) -> TracedFuncFacts<'tcx> {
    let tcx = cx.cx().tcx;

    assert!(should_analyze(tcx, instance));

    // If this function has a hardcoded fact set, use those.
    if is_tie_func(tcx, instance.def_id()) {
        return TracedFuncFacts {
            borrows: instantiate_set(tcx, instance.args[1].as_type().unwrap()),
        };
    }

    // Acquire the function body.
    let body = try_grab_optimized_mir_of_instance(tcx, instance.def).unwrap();

    // Ensure that we analyze the facts of each unsized function since unsize-checking depends
    // on this information being available.
    //
    // We use `reveal_all` since we're tracing fully concrete function instantiations which will
    // always be revealable without where clauses.
    for_each_concrete_unsized_func(
        tcx,
        ParamEnv::reveal_all(),
        instance.into(),
        body,
        |instance| {
            if should_analyze(tcx, instance) {
                cx.cx().analysis_queue.push(instance);
            }
        },
    );

    // See who th e function may call and where.
    let mut borrows = FxHashMap::default();

    for bb in body.basic_blocks.iter() {
        // If the terminator is a call terminator.
        let Some(TerminalCallKind::Static(_, target_instance)) = get_callee_from_terminator(
            tcx,
            ParamEnv::reveal_all(),
            instance.into(),
            &bb.terminator,
            &body.local_decls,
        ) else {
            continue;
        };

        // Recurse into its callee.
        if !should_analyze(tcx, target_instance) {
            continue;
        }

        let Some(target_facts) = cx.analyze(target_instance) else {
            continue;
        };

        let lt_id = parse_tie_func(tcx, target_instance).and_then(|v| v.tied_to);

        for (borrow_key, (borrow_mut, _)) in &target_facts.borrows {
            let (curr_mut, curr_lt) = borrows
                .entry(*borrow_key)
                .or_insert((Mutability::Not, None));

            if borrow_mut.is_mut() {
                *curr_mut = Mutability::Mut;
            }

            if let Some(lt_id) = lt_id {
                *curr_lt = Some(lt_id);
            }
        }
    }

    // Now, apply the absorption rules.
    if is_absorb_func(tcx, instance.def_id()) {
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

    TracedFuncFacts { borrows }
}
