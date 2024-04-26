use rustc_borrowck::{
    borrow_set::BorrowData,
    consumers::{
        calculate_borrows_out_of_scope_at_location, get_body_with_borrowck_facts, BorrowIndex,
        ConsumerOptions,
    },
};
use rustc_data_structures::fx::FxIndexMap;
use rustc_hash::FxHashSet;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{BasicBlock, Body, CallReturnPlaces, Location, Statement, Terminator, TerminatorEdges},
    ty::TyCtxt,
};
use rustc_mir_dataflow::{
    fmt::DebugWithContext, Analysis, AnalysisDomain, Forward, GenKill, GenKillAnalysis,
};

pub fn analyze_ensure_no_overlap(tcx: TyCtxt<'_>) {
    let Some((main, _)) = tcx.entry_fn(()) else {
        return;
    };

    let Some(main) = main.as_local() else {
        return;
    };

    let facts = get_body_with_borrowck_facts(tcx, main, ConsumerOptions::RegionInferenceContext);
    let start_map = &facts.borrow_set.location_map;
    let end_map = calculate_borrows_out_of_scope_at_location(
        &facts.body,
        &facts.region_inference_context,
        &facts.borrow_set,
    );

    //     eprintln!("Starts:");
    //     for (loc, borrow) in &facts.borrow_set.location_map {
    //         let local = borrow.borrowed_place.local;
    //         eprintln!("- {loc:?}: {:?}", facts.body.local_decls[local].ty);
    //     }
    //
    //     eprintln!("Ends:");
    //     for (loc, borrow) in end_map.iter() {
    //         for borrow in borrow {
    //             eprintln!(
    //                 "- {:?} -> {loc:?}",
    //                 facts
    //                     .borrow_set
    //                     .location_map
    //                     .get_index(borrow.as_usize())
    //                     .unwrap()
    //                     .0,
    //             );
    //         }
    //     }

    let mut results = RegionAwareLiveness {
        start_map,
        end_map: &end_map,
    }
    .into_engine(tcx, &facts.body)
    .iterate_to_fixpoint()
    .into_results_cursor(&facts.body);

    for (bb_loc, bb) in facts.body.basic_blocks.iter_enumerated() {
        for (stmt_loc, _) in bb.statements.iter().enumerate() {
            let loc = Location {
                block: bb_loc,
                statement_index: stmt_loc,
            };

            results.seek_before_primary_effect(loc);
            let state = results.get();

            eprintln!("{loc:?}:");
            for borrow in state.iter() {
                let local = start_map
                    .get_index(borrow.as_usize())
                    .unwrap()
                    .1
                    .borrowed_place
                    .local;

                eprintln!("- {:?}", facts.body.local_decls[local].ty);
            }
        }
    }
}

pub struct RegionAwareLiveness<'tcx, 'a> {
    start_map: &'a FxIndexMap<Location, BorrowData<'tcx>>,
    end_map: &'a FxIndexMap<Location, Vec<BorrowIndex>>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct LiveSet(pub FxHashSet<BorrowIndex>);

impl<'tcx, 'a> AnalysisDomain<'tcx> for RegionAwareLiveness<'tcx, 'a> {
    type Domain = BitSet<BorrowIndex>;
    type Direction = Forward;

    const NAME: &'static str = "RegionAwareLiveness";

    fn bottom_value(&self, body: &Body<'tcx>) -> Self::Domain {
        BitSet::new_empty(body.local_decls.len())
    }

    fn initialize_start_block(&self, _body: &Body<'tcx>, _state: &mut Self::Domain) {}
}

impl<'tcx, 'a> RegionAwareLiveness<'tcx, 'a> {
    fn handle_loc(&self, trans: &mut impl GenKill<BorrowIndex>, location: Location) {
        match (
            self.start_map.get_index_of(&location),
            self.end_map.get(&location),
        ) {
            (Some(borrow), None) => {
                trans.gen(BorrowIndex::from_usize(borrow));
            }
            (None, Some(borrows)) => {
                trans.kill_all(borrows.iter().copied());
            }
            (Some(_), Some(_)) => unreachable!(),
            (None, None) => {}
        }
    }
}

impl<'tcx, 'a> GenKillAnalysis<'tcx> for RegionAwareLiveness<'tcx, 'a> {
    type Idx = BorrowIndex;

    fn domain_size(&self, body: &Body<'tcx>) -> usize {
        body.basic_blocks.iter().map(|bb| bb.statements.len()).sum()
    }

    fn statement_effect(
        &mut self,
        trans: &mut impl GenKill<Self::Idx>,
        _statement: &Statement<'tcx>,
        location: Location,
    ) {
        self.handle_loc(trans, location);
    }

    fn terminator_effect<'mir>(
        &mut self,
        trans: &mut Self::Domain,
        terminator: &'mir Terminator<'tcx>,
        location: Location,
    ) -> TerminatorEdges<'mir, 'tcx> {
        self.handle_loc(trans, location);
        terminator.edges()
    }

    fn call_return_effect(
        &mut self,
        _trans: &mut Self::Domain,
        _block: BasicBlock,
        _return_places: CallReturnPlaces<'_, 'tcx>,
    ) {
    }
}

impl<'tcx, 'a> DebugWithContext<RegionAwareLiveness<'tcx, 'a>> for BorrowIndex {}
