use autoken::{analyzer::AnalyzerConfig, mir_reader::compile_analyze_mir};

const ICE_URL: &str = "https://www.github.com/Radbuglet/autoken/issues";

fn main() {
    let rustc_args = std::env::args().collect::<Vec<_>>();

    compile_analyze_mir(
        &rustc_args,
        ICE_URL,
        Box::new(|compiler, tcx| AnalyzerConfig {}.analyze(compiler, tcx)),
    );
}
