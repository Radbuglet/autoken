use rustc_driver::EXIT_SUCCESS;
use rustc_interface::interface::Compiler;
use rustc_middle::{mir::TerminatorKind, ty::TyCtxt};

use crate::mir_reader::TyCtxtExt;

#[derive(Debug)]
pub struct Analyzer {}

impl Analyzer {
    pub fn analyze(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) -> i32 {
        let (main_fn, _) = tcx.entry_fn(()).unwrap();
        println!("The main function is {main_fn:?}");

        let main_body = tcx.any_mir_body(main_fn);
        for bb in main_body.basic_blocks.iter() {
            match &bb.terminator.as_ref().map(|t| &t.kind) {
                Some(TerminatorKind::Drop { place, .. }) => {
                    println!("Dropping {place:?}");
                }
                Some(TerminatorKind::Call { func, .. }) => {
                    println!("Calling {func:?}");
                }
                _ => {}
            }
        }

        EXIT_SUCCESS
    }
}
