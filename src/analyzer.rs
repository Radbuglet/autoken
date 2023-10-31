use rustc_driver::EXIT_SUCCESS;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;

#[derive(Debug)]
pub struct AnalyzerConfig {}

impl AnalyzerConfig {
    pub fn analyze(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) -> i32 {
        let (main_fn, _) = tcx.entry_fn(()).unwrap();

        EXIT_SUCCESS
    }
}
