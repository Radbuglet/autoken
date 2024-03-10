use std::{path::PathBuf, process, sync::OnceLock};

use rustc_driver::{
    catch_with_exit_code, init_rustc_env_logger, install_ice_hook, Callbacks, Compilation,
    RunCompiler,
};

use rustc_interface::{interface::Compiler, Queries};
use rustc_session::{config::ErrorOutputType, EarlyErrorHandler};

use crate::analyzer::AnalysisDriver;

const ICE_URL: &str = "https://www.github.com/Radbuglet/autoken/issues";

static DRIVER: OnceLock<AnalysisDriver> = OnceLock::new();

pub fn main_inner(args: Vec<String>) -> ! {
    // Install rustc's default logger
    let handler = EarlyErrorHandler::new(ErrorOutputType::default());
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
            config.override_queries = Some(|_sess, loc, _ext| {
                DRIVER
                    .set(AnalysisDriver::new(loc.mir_built))
                    .ok()
                    .expect("`override_queries` called more than once");

                loc.mir_built = |tcx, id| {
                    DRIVER
                        .get()
                        .expect("analysis driver never initialized")
                        .mir_body(tcx, id)
                };
            });
        }
    }

    //     fn after_analysis<'tcx>(
    //         &mut self,
    //         _handler: &EarlyErrorHandler,
    //         _compiler: &Compiler,
    //         queries: &'tcx Queries<'tcx>,
    //     ) -> Compilation {
    //         if should_run_analysis() {
    //             queries.global_ctxt().unwrap().enter(|tcx| {
    //                 DRIVER
    //                     .get()
    //                     .expect("analysis driver never initialized")
    //                     .analyze(tcx);
    //             });
    //         }
    //
    //         Compilation::Continue
    //     }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        if should_run_analysis() {
            queries.global_ctxt().unwrap().enter(|tcx| {
                DRIVER
                    .get()
                    .expect("analysis driver never initialized")
                    .analyze(tcx);
            });
        }

        Compilation::Continue
    }
}
