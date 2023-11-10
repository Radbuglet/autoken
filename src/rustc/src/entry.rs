use std::{path::PathBuf, process};

use rustc_driver::{
    catch_with_exit_code, init_rustc_env_logger, install_ice_hook, Callbacks, Compilation,
    RunCompiler,
};

use rustc_interface::{interface::Compiler, Queries};
use rustc_session::{config::ErrorOutputType, EarlyErrorHandler};

use crate::analyzer::AnalyzerConfig;

const ICE_URL: &str = "https://www.github.com/Radbuglet/autoken/issues";

pub fn main_inner(args: Vec<String>) -> ! {
    // Install rustc's default logger
    let handler = EarlyErrorHandler::new(ErrorOutputType::default());
    init_rustc_env_logger(&handler);

    // Install a custom ICE hook for ourselves
    install_ice_hook(ICE_URL, |_| ());

    // Run the compiler with the user's specified arguments
    process::exit(catch_with_exit_code(|| {
        RunCompiler::new(
            &args,
            &mut AnalyzeMirCallbacks {
                config: AnalyzerConfig {},
            },
        )
        .run()
    }));
}

struct AnalyzeMirCallbacks {
    config: AnalyzerConfig,
}

impl Callbacks for AnalyzeMirCallbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        // We need access to the MIR so let's encode that.
        //
        // Miri doesn't use this because it only encodes the MIR for reachable functions and that can
        // break with clever `#[no_mangle]` hacks. Luckily, this analysis also only looks at functions
        // which are reachable from the main function so this is an okay limitation.
        config.opts.unstable_opts.always_encode_mir = true;

        // Define version-checking CFGs
        {
            // Never update these!
            const CFG_AUTOKEN_CHECKING_VERSIONS: &str = "__autoken_checking_versions";
            const CFG_MAJOR_IS_PREFIX: &str = "__autoken_major_is_";
            const CFG_SUPPORTED_MINOR_OR_LESS_IS_PREFIX: &str =
                "__autoken_supported_minor_or_less_is_";
            const CFG_DEPRECATED_MINOR_OR_LESS_IS_PREFIX: &str =
                "__autoken_deprecated_minor_or_less_is_";

            // Keep these in sync with `userland/build.rs`

            // Emit the CFGS
            let mut set = |var: String| config.crate_cfg.insert((var, None));

            set(CFG_AUTOKEN_CHECKING_VERSIONS.to_string());
            set(format!("{CFG_MAJOR_IS_PREFIX}{CURRENT_FORMAT_MAJOR}"));

            for v in (DEPRECATED_FORMAT_MINOR + 1)..=CURRENT_FORMAT_MINOR {
                set(format!("{CFG_SUPPORTED_MINOR_OR_LESS_IS_PREFIX}{v}"));
            }

            for v in 0..=DEPRECATED_FORMAT_MINOR {
                set(format!("{CFG_DEPRECATED_MINOR_OR_LESS_IS_PREFIX}{v}"));
            }
        }

        // We also have to hack in a little environment variable to override the sysroot.
        if let Ok(ovr) = std::env::var("AUTOKEN_OVERRIDE_SYSROOT") {
            config.opts.maybe_sysroot = Some(PathBuf::from(ovr));
        }
    }

    fn after_analysis<'tcx>(
        &mut self,
        _handler: &EarlyErrorHandler,
        compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            // Ensure that this is valid MIR
            if tcx.sess.compile_status().is_ok() && std::env::var("AUTOKEN_SKIP_ANALYSIS").is_err()
            {
                // Run our custom analysis engine
                self.config.analyze(compiler, tcx);
            }
        });

        Compilation::Continue
    }
}

include!("../CURRENT_FORMAT.in");
