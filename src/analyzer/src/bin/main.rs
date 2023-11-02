use autoken::{
    analyzer::AnalyzerConfig,
    mir_reader::{compile_analyze_mir, compile_collect_mir},
};

const MULTIPLEX_ENV: &str = "AUTOKEN_DRIVER_MODE";
const ICE_URL: &str = "https://www.github.com/Radbuglet/autoken/issues";

fn main() {
    let rustc_args = std::env::args().collect::<Vec<_>>();

    match std::env::var(MULTIPLEX_ENV) {
        Ok(var) if var == "compile" => {
            compile_collect_mir(&rustc_args);
        }
        Ok(var) if var == "analyze" => {
            compile_analyze_mir(
                &rustc_args,
                ICE_URL,
                Box::new(|compiler, tcx| AnalyzerConfig {}.analyze(compiler, tcx)),
            );
        }
        _ => panic!(
			"the environment variable {MULTIPLEX_ENV} was set to neither `compile` nor `analyze` so \
		     we have no clue how to handle this request",
		),
    }
}
