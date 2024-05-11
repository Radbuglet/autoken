use petgraph::{visit::Dfs, Graph};
use rustc_borrowck::consumers::{BodyWithBorrowckFacts, BorrowIndex, Borrows, ConsumerOptions};

use rustc_hir::def_id::LocalDefId;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{traversal::reverse_postorder, Local, Location, Statement, Terminator},
    ty::{Mutability, Region, TyCtxt},
};
use rustc_mir_dataflow::{Analysis, ResultsVisitor};
use rustc_span::Span;

use crate::util::{
    hash::{FxHashMap, FxHashSet},
    mir::get_body_with_borrowck_facts_but_sinful,
    ty::{
        extract_free_region_list, get_fn_sig_maybe_closure, normalize_preserving_regions,
        par_traverse_regions, re_as_vid, FunctionRelation, MutabilityExt,
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
    pub fn new(tcx: TyCtxt<'tcx>, did: LocalDefId) -> Self {
        // Determine the start and end locations of our borrows.
        let facts = get_body_with_borrowck_facts_but_sinful(
            tcx,
            did,
            ConsumerOptions::RegionInferenceContext,
        );

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
        let mut infer_to_real = FunctionRelation::default();

        par_traverse_regions(infer_ret_ty, real_ret_ty, |inf, real, _| {
            if let Some(inf) = re_as_vid(inf) {
                infer_to_real.insert(inf, real);
            }
        });

        // Now, use the region information to determine which locals are leaked
        let mut leaked_locals = FxHashMap::default();
        {
            let mut cst_graph = Graph::new();
            let mut cst_nodes = FxHashMap::default();

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
        mut are_conflicting: impl FnMut(Local, Local) -> Option<(String, Mutability, Mutability)>,
    ) {
        let dcx = tcx.dcx();

        for (&new_bw, conflicts) in &self.overlaps {
            for old_bw in conflicts.iter() {
                if old_bw == new_bw {
                    continue;
                }

                let (old_bw, old_bw_span) = self.borrows[&old_bw];
                let (new_bw, new_bw_span) = self.borrows[&new_bw];

                let Some((conflict, old_bw_mut, new_bw_mut)) = (are_conflicting)(new_bw, old_bw)
                else {
                    continue;
                };

                assert!(!old_bw_mut.is_compatible_with(new_bw_mut));

                // Report the conflict
                dcx.struct_span_err(
                    new_bw_span,
                    format!("conflicting borrows on token {conflict}"),
                )
                .with_span_label(
                    old_bw_span,
                    format!(
                        "value first borrowed {} here",
                        match old_bw_mut {
                            Mutability::Not => "immutably",
                            Mutability::Mut => "mutably",
                        }
                    ),
                )
                .with_span_label(
                    new_bw_span,
                    format!(
                        "value later borrowed {} here",
                        match new_bw_mut {
                            Mutability::Not => "immutably",
                            Mutability::Mut => "mutably",
                        }
                    ),
                )
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
    type FlowState = BitSet<BorrowIndex>;

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
