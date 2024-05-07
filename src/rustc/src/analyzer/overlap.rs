use petgraph::{visit::Dfs, Graph};
use rustc_borrowck::consumers::{
    get_body_with_borrowck_facts, BodyWithBorrowckFacts, Borrows, ConsumerOptions,
};

use rustc_hir::def_id::LocalDefId;
use rustc_middle::{
    mir::{traversal::reverse_postorder, BasicBlock, Body, Local, Location, Statement, Terminator},
    ty::{Region, RegionVid, TyCtxt},
};
use rustc_mir_dataflow::{
    Analysis, AnalysisDomain, Forward, Results, ResultsVisitable, ResultsVisitor,
};

use crate::util::{
    hash::{FxHashMap, FxHashSet},
    ty::extract_free_region_list,
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
        eprintln!("=== {did:?} === ");

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

        let mut visitor = BorrowckVisitor { _facts: &facts };

        rustc_mir_dataflow::visit_results(
            &facts.body,
            reverse_postorder(&facts.body).map(|(bb, _)| bb),
            &mut results,
            &mut visitor,
        );

        // Now, use the region information to determine which locals are leaked
        {
            let mut cst_graph = Graph::new();
            let mut cst_nodes = FxHashMap::default();

            fn re_as_vid(re: Region<'_>) -> Option<RegionVid> {
                re.is_var().then(|| re.as_var())
            }

            // Build constraint graph
            for cst in facts.region_inference_context.outlives_constraints() {
                // Left outlives right.
                let left = cst.sup;
                let right = cst.sub;

                let left = *cst_nodes
                    .entry(left)
                    .or_insert_with(|| cst_graph.add_node(left));

                let right = *cst_nodes
                    .entry(right)
                    .or_insert_with(|| cst_graph.add_node(right));

                cst_graph.add_edge(right, left, ());
            }

            // Determine which nodes are reachable from our leaked regions.
            let mut leaked = FxHashSet::default();

            for origin in extract_free_region_list(
                tcx,
                facts.body.local_decls[Local::from_u32(0)].ty,
                re_as_vid,
            ) {
                let Some(&origin) = cst_nodes.get(&origin) else {
                    continue;
                };

                let mut bfs = Dfs::new(&cst_graph, origin);

                while let Some(reachable) = bfs.next(&cst_graph) {
                    leaked.insert(reachable);
                }
            }

            // Finally, let's go through each local to see if it has any regions linked to the
            // return type.
            for (local, info) in facts.body.local_decls.iter_enumerated() {
                for used in extract_free_region_list(tcx, info.ty, re_as_vid) {
                    let Some(used) = cst_nodes.get(&used) else {
                        continue;
                    };

                    if leaked.contains(used) {
                        eprintln!(
                            "{local:?} of type {:?} is tied to the return value",
                            info.ty
                        );
                    }
                }
            }
        }

        Self { _ty: &() }
    }
}

struct BorrowckVisitor<'mir, 'tcx> {
    _facts: &'mir BodyWithBorrowckFacts<'tcx>,
}

impl<'mir, 'tcx, R> ResultsVisitor<'mir, 'tcx, R> for BorrowckVisitor<'mir, 'tcx> {
    type FlowState = <BorrowckResults<'mir, 'tcx> as ResultsVisitable<'tcx>>::FlowState;

    fn visit_statement_before_primary_effect(
        &mut self,
        _results: &mut R,
        _state: &Self::FlowState,
        _statement: &'mir Statement<'tcx>,
        _location: Location,
    ) {
    }

    fn visit_statement_after_primary_effect(
        &mut self,
        _results: &mut R,
        _state: &Self::FlowState,
        _statement: &'mir Statement<'tcx>,
        _location: Location,
    ) {
    }

    fn visit_terminator_before_primary_effect(
        &mut self,
        _results: &mut R,
        _state: &Self::FlowState,
        _terminator: &'mir Terminator<'tcx>,
        _location: Location,
    ) {
    }

    fn visit_terminator_after_primary_effect(
        &mut self,
        _results: &mut R,
        _state: &Self::FlowState,
        _terminator: &'mir Terminator<'tcx>,
        _location: Location,
    ) {
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
