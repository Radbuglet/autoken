//! Implements a scheme for analyzing an entire program's MIR. This scheme was inspired *heavily*
//! by Miri.

use std::{path::PathBuf, process, rc::Rc};

use rustc_driver::{
    catch_with_exit_code, init_rustc_env_logger, install_ice_hook, Callbacks, Compilation,
    RunCompiler, DEFAULT_BUG_REPORT_URL,
};
use rustc_hir::def_id::DefId;
use rustc_interface::{interface::Compiler, Queries};
use rustc_middle::{
    mir::Body,
    query::{ExternProviders, LocalCrate},
    ty::{InstanceDef, TyCtxt},
};
use rustc_session::{config::ErrorOutputType, search_paths::PathKind, EarlyErrorHandler};

// === `compile_collect_mir` === //

/// Runs the rustc compiler as usual but hooks the query system to ensure that the MIR for non-local
/// crates is also saved.
pub fn compile_collect_mir(rustc_args: &[String]) -> ! {
    // Install rustc's default logger
    let handler = EarlyErrorHandler::new(ErrorOutputType::default());
    init_rustc_env_logger(&handler);

    // Install rustc's default ICE reporting systems. We report ICEs to them because we're essentially
    // running a regular rustc invocation with a special config.
    install_ice_hook(DEFAULT_BUG_REPORT_URL, |_| ());

    // Run the compiler with the user's specified arguments
    process::exit(catch_with_exit_code(|| {
        RunCompiler::new(rustc_args, &mut CollectMirCallbacks).run()
    }));
}

struct CollectMirCallbacks;

impl Callbacks for CollectMirCallbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        use rustc_hir as hir;
        use rustc_middle::middle::exported_symbols as sym;

        // Don't do any of our shenanigans if rustc just wants to print out some info.
        if !config.opts.prints.is_empty() {
            return;
        }

        // Queries overridden here affect the data stored in `rmeta` files of dependencies, which
        // will be used later in a later program invocation running `compile_analyze_mir`.
        config.override_queries = Some(|_session, local_providers, _extern_providers| {
            local_providers.exported_symbols = |tcx, LocalCrate| {
                let reachable_set = tcx
                    .with_stable_hashing_context(|hcx| tcx.reachable_set(()).to_sorted(&hcx, true));

                tcx.arena.alloc_from_iter(
                    // This is based on:
                    // https://github.com/rust-lang/rust/blob/2962e7c0089d5c136f4e9600b7abccfbbde4973d/compiler/rustc_codegen_ssa/src/back/symbol_export.rs#L62-L63
                    // https://github.com/rust-lang/rust/blob/2962e7c0089d5c136f4e9600b7abccfbbde4973d/compiler/rustc_codegen_ssa/src/back/symbol_export.rs#L174
                    reachable_set.into_iter().filter_map(|&local_def_id| {
                        // Do the same filtering that rustc does:
                        // https://github.com/rust-lang/rust/blob/2962e7c0089d5c136f4e9600b7abccfbbde4973d/compiler/rustc_codegen_ssa/src/back/symbol_export.rs#L84-L102
                        // Otherwise it may cause unexpected behaviours and ICEs
                        // (https://github.com/rust-lang/rust/issues/86261).
                        let is_reachable_non_generic = matches!(
                            tcx.hir().get(tcx.hir().local_def_id_to_hir_id(local_def_id)),
                            hir::Node::Item(&hir::Item {
                                kind: hir::ItemKind::Static(..) | hir::ItemKind::Fn(..),
                                ..
                            }) | hir::Node::ImplItem(&hir::ImplItem {
                                kind: hir::ImplItemKind::Fn(..),
                                ..
                            })
                            if !tcx.generics_of(local_def_id).requires_monomorphization(tcx)
                        );
                        (is_reachable_non_generic
                            && tcx
                                .codegen_fn_attrs(local_def_id)
                                .contains_extern_indicator())
                        .then_some((
                            sym::ExportedSymbol::NonGeneric(local_def_id.to_def_id()),
                            // Some dummy `SymbolExportInfo` here. We only use
                            // `exported_symbols` in shims/foreign_items.rs and the export info
                            // is ignored.
                            sym::SymbolExportInfo {
                                level: sym::SymbolExportLevel::C,
                                kind: sym::SymbolExportKind::Text,
                                used: false,
                            },
                        ))
                    }),
                )
            }
        });
    }
}

// === `compile_analyze_mir` === //

type AnalyzerFn = Box<dyn FnMut(&Compiler, TyCtxt<'_>) -> i32 + Send>;

pub fn compile_analyze_mir(
    rustc_args: &[String],
    ice_url: &'static str,
    analyzer: AnalyzerFn,
) -> ! {
    // Install rustc's default logger
    let handler = EarlyErrorHandler::new(ErrorOutputType::default());
    init_rustc_env_logger(&handler);

    // Install rustc's default ICE reporting systems.
    install_ice_hook(ice_url, |_| ());

    // Run the compiler with the user's specified arguments
    process::exit(catch_with_exit_code(|| {
        RunCompiler::new(rustc_args, &mut AnalyzeMirCallbacks { analyzer }).run()
    }));
}

struct AnalyzeMirCallbacks {
    analyzer: AnalyzerFn,
}

impl Callbacks for AnalyzeMirCallbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        config.override_queries = Some(|_sess, _local_providers, extern_providers| {
            extern_providers.used_crate_source = |tcx, cnum| {
                let mut providers = ExternProviders::default();
                rustc_metadata::provide_extern(&mut providers);
                let mut crate_source = (providers.used_crate_source)(tcx, cnum);
                // HACK: rustc will emit "crate ... required to be available in rlib format, but
                // was not found in this form" errors once we use `tcx.dependency_formats()` if
                // there's no rlib provided, so setting a dummy path here to workaround those errors.
                Rc::make_mut(&mut crate_source).rlib = Some((PathBuf::new(), PathKind::All));
                crate_source
            };
        });
    }

    fn after_analysis<'tcx>(
        &mut self,
        _handler: &EarlyErrorHandler,
        compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            // Ensure that this is valid MIR
            if tcx.sess.compile_status().is_err() {
                tcx.sess
                    .fatal("cannot analyze programs which failed compilation");
            }

            // Run the user-provided analyzer
            process::exit((self.analyzer)(compiler, tcx));
        });

        Compilation::Stop
    }
}

// === MIR helpers === //

pub trait TyCtxtExt<'tcx> {
    fn any_mir_body(&self, id: DefId) -> &'tcx Body<'tcx>;
}

impl<'tcx> TyCtxtExt<'tcx> for TyCtxt<'tcx> {
    fn any_mir_body(&self, id: DefId) -> &'tcx Body<'tcx> {
        self.instance_mir(InstanceDef::Item(id))
    }
}
