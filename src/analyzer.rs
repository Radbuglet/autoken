use rustc_driver::EXIT_SUCCESS;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;

#[derive(Debug)]
pub struct Analyzer {}

impl Analyzer {
    pub fn analyze(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) -> i32 {
        println!("The main function is {:?}", tcx.entry_fn(()));
        EXIT_SUCCESS
    }
}
