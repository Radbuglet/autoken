use petgraph::{visit::Dfs, Graph};
use rustc_borrowck::consumers::{
    get_body_with_borrowck_facts, BodyWithBorrowckFacts, BorrowIndex, Borrows, ConsumerOptions,
};

use rustc_hir::def_id::LocalDefId;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{traversal::reverse_postorder, BasicBlock, Body, Local, Location, Statement, Terminator},
    ty::{Region, RegionVid, TyCtxt},
};
use rustc_mir_dataflow::{
    Analysis, AnalysisDomain, Forward, Results, ResultsVisitable, ResultsVisitor,
};
use rustc_span::Span;

use crate::util::{
    hash::{FxHashMap, FxHashSet},
    ty::{
        extract_free_region_list, get_fn_sig_maybe_closure, normalize_preserving_regions,
        par_traverse_regions, FunctionMap,
    },
};

// === Analysis === //

#[derive(Debug, Clone)]
pub struct BodyOverlapFacts<'tcx> {
    borrows: FxHashMap<BorrowIndex, (Local, Span)>,
    overlaps: FxHashMap<BorrowIndex, BitSet<BorrowIndex>>,
    leaked_locals: FxHashMap<Region<'tcx>, Vec<Local>>,
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

        let borrow_locations = facts
            .borrow_set
            .location_map
            .iter()
            .enumerate()
            .map(|(bw, (loc, info))| {
                (
                    BorrowIndex::from_usize(bw),
                    (info.borrowed_place.local, facts.body.source_info(*loc).span),
                )
            })
            .collect();

        // Run fix-point analysis to figure out which sections of code have which borrows.
        let mut results = Borrows::new(
            tcx,
            &facts.body,
            &facts.region_inference_context,
            &facts.borrow_set,
        )
        .into_engine(tcx, &facts.body)
        .iterate_to_fixpoint();

        let mut visitor = BorrowckVisitor {
            facts: &facts,
            overlaps: FxHashMap::default(),
        };

        rustc_mir_dataflow::visit_results(
            &facts.body,
            reverse_postorder(&facts.body).map(|(bb, _)| bb),
            &mut results,
            &mut visitor,
        );

        let overlaps = visitor.overlaps;

        // Determine the bijection between the inferred return type and the actual return type.
        let real_ret_ty = normalize_preserving_regions(
            tcx,
            tcx.param_env(did),
            get_fn_sig_maybe_closure(tcx, did.to_def_id())
                .skip_binder()
                .output(),
        )
        .skip_binder();

        let infer_ret_ty = facts.body.local_decls[Local::from_u32(0)].ty;
        let mut infer_to_real = FunctionMap::default();

        par_traverse_regions(infer_ret_ty, real_ret_ty, |inf, real| {
            infer_to_real.insert(inf.as_var(), real);
        });

        // Now, use the region information to determine which locals are leaked
        let mut leaked_locals = FxHashMap::default();
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
            for origin_re_var in extract_free_region_list(tcx, infer_ret_ty, re_as_vid) {
                let mut leaked_res = FxHashSet::default();

                let Some(&origin) = cst_nodes.get(&origin_re_var) else {
                    continue;
                };

                let mut dfs = Dfs::new(&cst_graph, origin);

                while let Some(reachable) = dfs.next(&cst_graph) {
                    leaked_res.insert(reachable);
                }

                // Finally, let's go through each local to see if it has any regions linked to the
                // return type.
                let origin_real = infer_to_real.map[&origin_re_var].unwrap();
                let leaked_locals: &mut Vec<_> = leaked_locals.entry(origin_real).or_default();

                for (local, info) in facts.body.local_decls.iter_enumerated() {
                    for used in extract_free_region_list(tcx, info.ty, re_as_vid) {
                        let Some(used) = cst_nodes.get(&used) else {
                            continue;
                        };

                        if leaked_res.contains(used) {
                            leaked_locals.push(local);
                        }
                    }
                }
            }
        }

        Self {
            borrows: borrow_locations,
            overlaps,
            leaked_locals,
        }
    }

    pub fn validate_overlaps(
        &self,
        tcx: TyCtxt<'tcx>,
        mut are_conflicting: impl FnMut(Local, Local) -> bool,
    ) {
        let dcx = tcx.dcx();

        for (&new_bw, conflicts) in &self.overlaps {
            for old_bw in conflicts.iter() {
                let (old_bw_local, old_bw_span) = self.borrows[&old_bw];
                let (new_bw_local, new_bw_span) = self.borrows[&new_bw];

                if !(are_conflicting)(new_bw_local, old_bw_local) {
                    continue;
                }

                // Report the conflict
                // TODO: Better diagnostics
                dcx.struct_span_warn(new_bw_span, "conflicting AuToken borrows")
                    .with_span_label(old_bw_span, "first borrow here")
                    .with_span_label(new_bw_span, "second borrow here")
                    .emit();
            }
        }
    }
}

struct BorrowckVisitor<'mir, 'tcx> {
    facts: &'mir BodyWithBorrowckFacts<'tcx>,
    overlaps: FxHashMap<BorrowIndex, BitSet<BorrowIndex>>,
}

impl<'mir, 'tcx> BorrowckVisitor<'mir, 'tcx> {
    fn push_overlap_set(&mut self, location: Location, set: &BitSet<BorrowIndex>) {
        let Some(started) = self.facts.borrow_set.location_map.get_index_of(&location) else {
            return;
        };
        self.overlaps
            .insert(BorrowIndex::from_usize(started), set.clone());
    }
}

impl<'mir, 'tcx, R> ResultsVisitor<'mir, 'tcx, R> for BorrowckVisitor<'mir, 'tcx> {
    type FlowState = <BorrowckResults<'mir, 'tcx> as ResultsVisitable<'tcx>>::FlowState;

    fn visit_statement_before_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        _statement: &'mir Statement<'tcx>,
        location: Location,
    ) {
        self.push_overlap_set(location, state);
    }

    fn visit_statement_after_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        _statement: &'mir Statement<'tcx>,
        location: Location,
    ) {
        self.push_overlap_set(location, state);
    }

    fn visit_terminator_before_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        _terminator: &'mir Terminator<'tcx>,
        location: Location,
    ) {
        self.push_overlap_set(location, state);
    }

    fn visit_terminator_after_primary_effect(
        &mut self,
        _results: &mut R,
        state: &Self::FlowState,
        _terminator: &'mir Terminator<'tcx>,
        location: Location,
    ) {
        self.push_overlap_set(location, state);
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
