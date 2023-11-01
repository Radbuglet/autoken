use autoken::{analyzer::AnalyzerConfig, mir_reader::compile_analyze_mir};

fn main() {
    let mut analyzer = AnalyzerConfig {};

    let second_arg = std::env::args().nth(1).expect("missing path");

    compile_analyze_mir(
        &["autoken".into(), second_arg],
        "example.com",
        Box::new(move |compiler, tcx| analyzer.analyze(compiler, tcx)),
    );
}
