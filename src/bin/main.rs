use autoken::{analyzer::Analyzer, mir_reader::compile_analyze_mir};

fn main() {
    let mut analyzer = Analyzer {};

    compile_analyze_mir(
        &["autoken".into(), "foo.rs".into()],
        "example.com",
        Box::new(move |compiler, tcx| analyzer.analyze(compiler, tcx)),
    );
}
