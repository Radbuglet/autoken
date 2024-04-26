use rustc_middle::{mir::Location, ty::TyCtxt};
use rustc_mir_dataflow::{impls::MaybeLiveLocals, Analysis};

pub fn analyze_borrow_overlap(tcx: TyCtxt<'_>) {
    let Some((main, _)) = tcx.entry_fn(()) else {
        return;
    };

    let Some(main) = main.as_local() else {
        return;
    };

    let body = &*tcx.mir_promoted(main).0.borrow();
    let mut results = MaybeLiveLocals
        .into_engine(tcx, body)
        .iterate_to_fixpoint()
        .into_results_cursor(body);

    for (bb_loc, bb) in body.basic_blocks.iter_enumerated() {
        for (stmt_loc, _) in bb.statements.iter().enumerate() {
            let loc = Location {
                block: bb_loc,
                statement_index: stmt_loc,
            };

            results.seek_before_primary_effect(loc);
            let state = results.get();

            eprintln!("{loc:?}:");
            for local in state.iter() {
                eprintln!("- {:?}", body.local_decls[local].ty);
            }
        }
    }
}
