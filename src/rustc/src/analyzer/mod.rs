use rustc_hir::{def::DefKind, Constness, LangItem};

use rustc_middle::ty::{Instance, ParamEnv, TyCtxt};

use crate::{
    analyzer::overlap::BodyOverlapFacts,
    util::{
        feeder::{feeders::MirBuiltStasher, read_feed},
        hash::FxHashMap,
        mir::{
            for_each_concrete_unsized_func, has_optimized_mir, iter_all_local_def_ids,
            try_grab_base_mir_of_def_id, try_grab_optimized_mir_of_instance,
        },
    },
};

use self::{template::BodyTemplateFacts, trace::TraceFacts};

mod mir;
mod overlap;
mod sets;
mod sym;
mod template;
mod trace;

// TODO: Double-check the short-circuits in the analysis routine to make sure we're not ignoring
//  important items.
pub fn analyze(tcx: TyCtxt<'_>) {
    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    for local_def in iter_all_local_def_ids(tcx) {
        if try_grab_base_mir_of_def_id(tcx, local_def).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, local_def).is_some());
        }
    }

    // Generate borrow-checking templates for each local function.
    //
    // TODO: Serialize these across crates.
    assert!(!tcx.untracked().definitions.is_frozen());

    let mut templates = FxHashMap::default();

    for did in iter_all_local_def_ids(tcx) {
        if read_feed::<MirBuiltStasher>(tcx, did).is_none()
            || !has_optimized_mir(tcx, did.to_def_id())
            || tcx.constness(did) == Constness::Const
        {
            continue;
        }

        let param_env_user = tcx.param_env(did);
        let (template, shadow_did) = BodyTemplateFacts::new(tcx, param_env_user, did);

        templates.insert(
            did.to_def_id(),
            (template, shadow_did, None::<BodyOverlapFacts>),
        );
    }

    // Generate trace facts.
    let trace = TraceFacts::compute(tcx);

    // Check for undeclared unsizing in trace.
    for &instance in trace.facts.keys() {
        let body = try_grab_optimized_mir_of_instance(tcx, instance.def).unwrap();

        if tcx.entry_fn(()).map(|(did, _)| did) == Some(instance.def_id()) {
            ensure_no_borrow(tcx, &trace, instance, "use this main function");
        }

        if tcx.def_kind(instance.def_id()) == DefKind::AssocFn
            && tcx
                .associated_item(instance.def_id())
                .trait_item_def_id
                .map(|method_did| tcx.parent(method_did))
                == Some(tcx.require_lang_item(LangItem::Drop, None))
        {
            ensure_no_borrow(tcx, &trace, instance, "use this method as a destructor");
        }

        for_each_concrete_unsized_func(
            tcx,
            ParamEnv::reveal_all(),
            instance.into(),
            body,
            |instance| ensure_no_borrow(tcx, &trace, instance, "unsize this function"),
        );
    }

    // Borrow-check each template fact
    for (orig_did, (_, shadow_did, overlaps)) in &mut templates {
        *overlaps = Some(BodyOverlapFacts::new(tcx, *orig_did, *shadow_did));
    }

    // Validate each traced function using their template
    for &instance in trace.facts.keys() {
        let Some((template, _, overlaps)) = templates.get(&instance.def_id()) else {
            continue;
        };

        template.validate(tcx, &trace, overlaps.as_ref().unwrap(), instance.args);
    }
}

fn ensure_no_borrow<'tcx>(
    tcx: TyCtxt<'tcx>,
    trace: &TraceFacts<'tcx>,
    instance: Instance<'tcx>,
    action: &str,
) {
    let Some(facts) = trace.facts(instance) else {
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
                    .map(|(k, (m, _))| format!(
                        "{k} {}",
                        if m.is_mut() { "mutably" } else { "immutably" }
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
}
