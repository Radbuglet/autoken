use std::{path::PathBuf, process};

use rustc_data_structures::steal::Steal;
use rustc_driver::{
    catch_with_exit_code, init_rustc_env_logger, install_ice_hook, Callbacks, Compilation,
    RunCompiler,
};

use rustc_hir::{
    def::DefKind,
    def_id::{DefId, LocalDefId},
    HirId,
};
use rustc_interface::{interface::Compiler, Queries};
use rustc_middle::{
    dep_graph::DepNodeIndex,
    mir::Body,
    ty::{AssocItem, TyCtxt, Visibility},
};
use rustc_session::{config::ErrorOutputType, EarlyDiagCtxt};

use crate::util::feeder::{
    feed,
    feeders::{
        AssociatedItemFeeder, DefKindFeeder, MirBuiltFeeder, MirBuiltStasher,
        OptLocalDefIdToHirIdFeeder, VisibilityFeeder,
    },
    once_val, read_feed,
};

const ICE_URL: &str = "https://www.github.com/Radbuglet/autoken/issues";

pub fn main_inner(args: Vec<String>) -> ! {
    // Install rustc's default logger
    let handler = EarlyDiagCtxt::new(ErrorOutputType::default());
    init_rustc_env_logger(&handler);

    // Install a custom ICE hook for ourselves
    install_ice_hook(ICE_URL, |_| ());

    // Run the compiler with the user's specified arguments
    process::exit(catch_with_exit_code(|| {
        RunCompiler::new(&args, &mut AnalyzeMirCallbacks).run()
    }));
}

pub fn should_run_analysis() -> bool {
    std::env::var("AUTOKEN_SKIP_ANALYSIS").is_err()
}

struct AnalyzeMirCallbacks;

impl Callbacks for AnalyzeMirCallbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        // We need access to the MIR so let's encode that.
        //
        // Miri doesn't use this because it only encodes the MIR for reachable functions and that can
        // break with clever `#[no_mangle]` hacks. Luckily, this analysis also only looks at functions
        // which are reachable from the main function so this is an okay limitation.
        config.opts.unstable_opts.always_encode_mir = true;

        // We also have to hack in a little environment variable to override the sysroot.
        if let Ok(ovr) = std::env::var("AUTOKEN_OVERRIDE_SYSROOT") {
            config.opts.maybe_sysroot = Some(PathBuf::from(ovr));
        }

        if should_run_analysis() {
            config.override_queries = Some(|_sess, query| {
                // Feeders
                once_val! {
                    mir_built: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<Body<'tcx>> = query.mir_built;
                    opt_local_def_id_to_hir_id: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<HirId> = query.opt_local_def_id_to_hir_id;
                    visibility: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Visibility<DefId> = query.visibility;
                    associated_item: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> AssocItem = query.associated_item;
                    def_kind: for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> DefKind = query.def_kind;
                }

                query.mir_built = |tcx, id| {
                    // N.B. we have to force all reads of this query to be forever-red since,
                    // otherwise, the user may mistake a previously unfed DefId for a fed DefId and
                    // never update its contents.
                    tcx.dep_graph.read_index(DepNodeIndex::FOREVER_RED_NODE);

                    if let Some(fed) = read_feed::<MirBuiltFeeder>(tcx, id) {
                        fed
                    } else {
                        let built = mir_built.get()(tcx, id);
                        if read_feed::<MirBuiltStasher>(tcx, id).is_none() {
                            feed::<MirBuiltStasher>(
                                tcx,
                                id,
                                tcx.arena.alloc(built.borrow().clone()),
                            );
                        }
                        built
                    }
                };

                query.opt_local_def_id_to_hir_id = |tcx, id| {
                    // N.B. ibid
                    tcx.dep_graph.read_index(DepNodeIndex::FOREVER_RED_NODE);

                    if let Some(fed) = read_feed::<OptLocalDefIdToHirIdFeeder>(tcx, id) {
                        fed
                    } else {
                        (opt_local_def_id_to_hir_id.get())(tcx, id)
                    }
                };

                query.visibility = |tcx, id| {
                    // N.B. ibid
                    tcx.dep_graph.read_index(DepNodeIndex::FOREVER_RED_NODE);

                    if let Some(fed) = read_feed::<VisibilityFeeder>(tcx, id) {
                        fed
                    } else {
                        (visibility.get())(tcx, id)
                    }
                };

                query.associated_item = |tcx, id| {
                    // N.B. ibid
                    tcx.dep_graph.read_index(DepNodeIndex::FOREVER_RED_NODE);

                    if let Some(fed) = read_feed::<AssociatedItemFeeder>(tcx, id) {
                        fed
                    } else {
                        (associated_item.get())(tcx, id)
                    }
                };

                query.def_kind = |tcx, id| {
                    // N.B. ibid
                    tcx.dep_graph.read_index(DepNodeIndex::FOREVER_RED_NODE);

                    if let Some(fed) = read_feed::<DefKindFeeder>(tcx, id) {
                        fed
                    } else {
                        (def_kind.get())(tcx, id)
                    }
                };
            });
        }
    }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        if should_run_analysis() {
            queries
                .global_ctxt()
                .unwrap()
                .enter(|tcx| crate::analyzer::analyze(tcx));
        }

        Compilation::Continue
    }
}
