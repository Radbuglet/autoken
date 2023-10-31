use autoken::{analyzer::AnalyzerConfig, mir_reader::compile_analyze_mir};

fn main() {
    let mut analyzer = AnalyzerConfig {};

    compile_analyze_mir(
        &["autoken".into(), "foo.rs".into()],
        "example.com",
        Box::new(move |compiler, tcx| analyzer.analyze(compiler, tcx)),
    );
}
