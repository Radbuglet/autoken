use rustc_borrowck::consumers::{
    get_body_with_borrowck_facts, BodyWithBorrowckFacts, Borrows, ConsumerOptions,
};

use rustc_hir::def_id::LocalDefId;
use rustc_middle::{
    mir::{traversal::reverse_postorder, BasicBlock, Body, Location, Statement, Terminator},
    ty::TyCtxt,
};
use rustc_mir_dataflow::{
    Analysis, AnalysisDomain, Forward, Results, ResultsVisitable, ResultsVisitor,
};

// === Analysis === //

#[derive(Debug, Clone)]
pub struct BodyOverlapFacts<'tcx> {
    _ty: &'tcx (),
}

impl<'tcx> BodyOverlapFacts<'tcx> {
    pub fn can_borrow_check(tcx: TyCtxt<'tcx>, did: LocalDefId) -> bool {
        // `get_body_with_borrowck_facts` skips a validation step compared to `mir_borrowck` so we
        // reintroduce it here.
        let (input_body, _promoted) = tcx.mir_promoted(did);
        let input_body = &input_body.borrow();
        !input_body.should_skip() && input_body.tainted_by_errors.is_none()
    }

    pub fn new(tcx: TyCtxt<'tcx>, did: LocalDefId) -> Self {
        // Determine the start and end locations of our borrows.
        // FIXME: Does not use `tcx.local_def_id_to_hir_id(def).owner` to determine the origin of the
        //  inference context.
        let facts = get_body_with_borrowck_facts(tcx, did, ConsumerOptions::RegionInferenceContext);

        // Run fix-point analysis to figure out which sections of code have which borrows.
        let mut results = Borrows::new(
            tcx,
            &facts.body,
            &facts.region_inference_context,
            &facts.borrow_set,
        )
        .into_engine(tcx, &facts.body)
        .iterate_to_fixpoint();

        let mut visitor = BorrowckVisitor { facts: &facts };

        eprintln!("=== {did:?} ===");
        rustc_mir_dataflow::visit_results(
            &facts.body,
            reverse_postorder(&facts.body).map(|(bb, _)| bb),
            &mut results,
            &mut visitor,
        );

        Self { _ty: &() }
    }
}

struct BorrowckVisitor<'mir, 'tcx> {
    facts: &'mir BodyWithBorrowckFacts<'tcx>,
}

impl<'mir, 'tcx, R> ResultsVisitor<'mir, 'tcx, R> for BorrowckVisitor<'mir, 'tcx> {
    type FlowState = <BorrowckResults<'mir, 'tcx> as ResultsVisitable<'tcx>>::FlowState;

    fn visit_statement_before_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        statement: &'mir Statement<'tcx>,
        _location: Location,
    ) {
        eprintln!("{statement:?}- {:?}", state);
    }

    fn visit_statement_after_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        statement: &'mir Statement<'tcx>,
        _location: Location,
    ) {
        eprintln!("{statement:?}+ {:?}", state);
    }

    fn visit_terminator_before_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        terminator: &'mir Terminator<'tcx>,
        _location: Location,
    ) {
        eprintln!("{terminator:?}- {state:?}");
    }

    fn visit_terminator_after_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        terminator: &'mir Terminator<'tcx>,
        _location: Location,
    ) {
        eprintln!("{terminator:?}+ {state:?}");
    }
}

// === Forked === //

struct BorrowckResults<'mir, 'tcx> {
    borrows: Results<'tcx, Borrows<'mir, 'tcx>>,
}

impl<'mir, 'tcx> ResultsVisitable<'tcx> for BorrowckResults<'mir, 'tcx> {
    // All three analyses are forward, but we have to use just one here.
    type Direction = Forward;
    type FlowState = <Borrows<'mir, 'tcx> as AnalysisDomain<'tcx>>::Domain;

    fn new_flow_state(&self, body: &Body<'tcx>) -> Self::FlowState {
        self.borrows.analysis.bottom_value(body)
    }

    fn reset_to_block_entry(&self, state: &mut Self::FlowState, block: BasicBlock) {
        state.clone_from(self.borrows.entry_set_for_block(block));
    }

    fn reconstruct_before_statement_effect(
        &mut self,
        state: &mut Self::FlowState,
        stmt: &Statement<'tcx>,
        loc: Location,
    ) {
        self.borrows
            .analysis
            .apply_before_statement_effect(&mut *state, stmt, loc);
    }

    fn reconstruct_statement_effect(
        &mut self,
        state: &mut Self::FlowState,
        stmt: &Statement<'tcx>,
        loc: Location,
    ) {
        self.borrows
            .analysis
            .apply_statement_effect(&mut *state, stmt, loc);
    }

    fn reconstruct_before_terminator_effect(
        &mut self,
        state: &mut Self::FlowState,
        term: &Terminator<'tcx>,
        loc: Location,
    ) {
        self.borrows
            .analysis
            .apply_before_terminator_effect(&mut *state, term, loc);
    }

    fn reconstruct_terminator_effect(
        &mut self,
        state: &mut Self::FlowState,
        term: &Terminator<'tcx>,
        loc: Location,
    ) {
        self.borrows
            .analysis
            .apply_terminator_effect(&mut *state, term, loc);
    }
}
