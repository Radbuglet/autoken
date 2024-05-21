use rustc_ast::Mutability;
use rustc_hir::{
    def::DefKind,
    def_id::{DefId, LOCAL_CRATE},
    Constness, LangItem,
};

use rustc_middle::ty::{Instance, ParamEnv, TyCtxt};
use rustc_session::config::CrateType;
use rustc_span::Span;

use std::fmt::Write;

use crate::{
    analyzer::overlap::BodyOverlapFacts,
    util::{
        feeder::{feeders::MirBuiltStasher, read_feed},
        hash::FxHashMap,
        meta::{get_crate_cache_path, save_to_file, try_load_from_file},
        mir::{
            for_each_concrete_unsized_func, has_optimized_mir, iter_all_local_def_ids,
            try_grab_base_mir_of_def_id, try_grab_optimized_mir_of_instance,
        },
    },
};

use self::{template::BodyTemplateFacts, trace::TraceFacts};

// === Modules === //

mod mir;
mod overlap;
mod sets;
mod sym;
mod template;
mod trace;

// === Driver === //

type SerializedCrateData<'tcx> =
    FxHashMap<DefId, (BodyTemplateFacts<'tcx>, BodyOverlapFacts<'tcx>)>;

pub fn analyze(tcx: TyCtxt<'_>) {
    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`
    for local_def in iter_all_local_def_ids(tcx) {
        if try_grab_base_mir_of_def_id(tcx, local_def).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, local_def).is_some());
        }
    }

    // Generate borrow-checking templates for each local function
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
            (template, Some(shadow_did), None::<BodyOverlapFacts>),
        );
    }

    // Generate trace facts
    let trace = TraceFacts::compute(tcx);

    // Check for undeclared unsizing in trace
    for &instance in trace.facts.keys() {
        let body = try_grab_optimized_mir_of_instance(tcx, instance.def).unwrap();

        if tcx.entry_fn(()).map(|(did, _)| did) == Some(instance.def_id()) {
            ensure_no_borrow(
                tcx,
                &trace,
                instance,
                tcx.def_span(instance.def_id()),
                "use this main function",
            );
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
                &trace,
                instance,
                tcx.def_span(instance.def_id()),
                "use this method as a destructor",
            );
        }

        for_each_concrete_unsized_func(
            tcx,
            ParamEnv::reveal_all(),
            instance.into(),
            body,
            |span, instance| ensure_no_borrow(tcx, &trace, instance, span, "unsize this function"),
        );
    }

    // Borrow-check each template fact
    for (orig_did, (_, shadow_did, overlaps)) in &mut templates {
        *overlaps = Some(BodyOverlapFacts::new(tcx, *orig_did, shadow_did.unwrap()));
    }

    // Load other crates' facts
    for &krate in tcx.crates(()) {
        let path = get_crate_cache_path(tcx, krate);

        let Some(map) =
            try_load_from_file::<SerializedCrateData<'_>>(tcx, "AuToken metadata", &path)
        else {
            continue;
        };

        for (did, (template, overlap)) in map {
            assert!(!templates.contains_key(&did));
            templates.insert(did, (template, None, Some(overlap)));
        }
    }

    // Validate each traced function using their template
    for &instance in trace.facts.keys() {
        let Some((template, _, overlaps)) = templates.get(&instance.def_id()) else {
            continue;
        };

        template.validate(tcx, &trace, overlaps.as_ref().unwrap(), instance.args);
    }

    // Save my crate's facts
    if tcx.needs_metadata() && !tcx.crate_types().contains(&CrateType::ProcMacro) {
        let path = get_crate_cache_path(tcx, LOCAL_CRATE);

        let serialized = templates
            .iter()
            .filter(|(did, _)| did.is_local())
            .map(|(&did, (template, _, overlaps))| {
                (did, (template.clone(), overlaps.as_ref().unwrap().clone()))
            })
            .collect::<SerializedCrateData<'_>>();

        save_to_file(tcx, "AuToken metadata", &path, &serialized);
    }
}

fn ensure_no_borrow<'tcx>(
    tcx: TyCtxt<'tcx>,
    trace: &TraceFacts<'tcx>,
    instance: Instance<'tcx>,
    span: Span,
    action: &str,
) {
    let Some(facts) = trace.facts(instance) else {
        return;
    };

    rustc_middle::ty::print::with_forced_trimmed_paths! {
        if !facts.borrows.is_empty() {
            let mut diag = tcx.sess.dcx().struct_err(format!(
                "cannot {action} because it borrows unabsorbed tokens",
            ));

            diag.span(span);

            let mut borrow_list = String::new();
            let mut borrow_strings = Vec::new();

            for (ty, (mutability, _)) in &facts.borrows {
                borrow_strings.push(format!("{}{ty}", match mutability {
                    Mutability::Not => "&",
                    Mutability::Mut => "&mut ",
                }));
            }

            borrow_strings.sort_unstable();

            for (i, borrow_string) in borrow_strings.iter().enumerate() {
                let is_first_line = i == 0;
                let is_last_line = i == borrow_strings.len() - 1;

                writeln!(
                    &mut borrow_list,
                    "{} {borrow_string}{}",
                    if is_first_line {
                        "uses"
                    } else {
                        "    "
                    },
                    if is_last_line {
                        "."
                    } else {
                        ","
                    }
                ).unwrap();
            }

            diag.note(borrow_list);

            diag.span_note(tcx.def_span(instance.def_id()), format!("{instance} was unsized"));

            diag.emit();
        }
    }
}
