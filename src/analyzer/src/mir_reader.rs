use std::process;

use rustc_driver::{
    catch_with_exit_code, init_rustc_env_logger, install_ice_hook, Callbacks, Compilation,
    RunCompiler,
};

use rustc_interface::{interface::Compiler, Queries};
use rustc_middle::ty::TyCtxt;
use rustc_session::{config::ErrorOutputType, EarlyErrorHandler};

type AnalyzerFn = Box<dyn FnMut(&Compiler, TyCtxt<'_>) + Send>;

/// Runs a regular session of `rustc` but ensures that external MIR is stored away in each crate's
/// `.rlib` file and gives an `AnalyzerFn` the opportunity to analyze the code after the MIR has been
/// successfully constructed. Other than these two changes, this functions exactly like a regular
/// `rustc` session.
pub fn compile_analyze_mir(
    rustc_args: &[String],
    ice_url: &'static str,
    analyzer: AnalyzerFn,
) -> ! {
    // Install rustc's default logger
    let handler = EarlyErrorHandler::new(ErrorOutputType::default());
    init_rustc_env_logger(&handler);

    // Install a custom ICE hook for ourselves
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
        // We need access to the MIR so let's encode that.
        //
        // Miri doesn't use this because it only encodes the MIR for reachable functions and that can
        // break with clever `#[no_mangle]` hacks. Luckily, this analysis also only looks at functions
        // which are reachable from the main function so this is an okay limitation.
        config.opts.unstable_opts.always_encode_mir = true;
    }

    fn after_analysis<'tcx>(
        &mut self,
        _handler: &EarlyErrorHandler,
        compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            // Ensure that this is valid MIR
            if tcx.sess.compile_status().is_ok() {
                // Run the user-provided analyzer
                (self.analyzer)(compiler, tcx);
            }
        });

        Compilation::Continue
    }
}
