use petgraph::{visit::Dfs, Graph};
use rustc_borrowck::consumers::{BodyWithBorrowckFacts, BorrowIndex, Borrows, ConsumerOptions};

use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_index::bit_set::BitSet;
use rustc_macros::{TyDecodable, TyEncodable};
use rustc_middle::{
    mir::{traversal::reverse_postorder, Local, Location, Statement, Terminator},
    ty::{GenericArgs, Mutability, Region, TyCtxt},
};
use rustc_mir_dataflow::{Analysis, ResultsVisitor};
use rustc_span::Span;

use crate::util::{
    hash::{FxHashMap, FxHashSet},
    mir::get_body_with_borrowck_facts_but_sinful,
    pair::Pair,
    ty::{extract_free_region_list, re_as_vid, MutabilityExt},
};

// === Analysis === //

rustc_index::newtype_index! {
    #[orderable]
    #[debug_format = "bw{}"]
    #[encodable]
    pub struct SerBorrowIndex {}
}

#[derive(Debug, Clone, TyEncodable, TyDecodable)]
pub struct BodyOverlapFacts<'tcx> {
    borrows: FxHashMap<SerBorrowIndex, (Local, Span)>,
    overlaps: FxHashMap<SerBorrowIndex, BitSet<SerBorrowIndex>>,
    leaked_locals: FxHashMap<Region<'tcx>, Vec<Local>>,
    leaked_local_def_spans: FxHashMap<Local, Span>,
}

impl<'tcx> BodyOverlapFacts<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>, orig_did: DefId, shadow_did: LocalDefId) -> Self {
        // Determine the start and end locations of our borrows.
        let facts = get_body_with_borrowck_facts_but_sinful(
            tcx,
            shadow_did,
            ConsumerOptions::RegionInferenceContext,
        );

        let borrows = facts
            .borrow_set
            .location_map
            .iter()
            .enumerate()
            .map(|(bw, (loc, info))| {
                (
                    SerBorrowIndex::from_usize(bw),
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

        let overlaps = visitor
            .overlaps
            .into_iter()
            .map(|(k, v)| {
                (SerBorrowIndex::from_u32(k.as_u32()), {
                    let mut v2 = BitSet::new_empty(v.domain_size());
                    for i in v.iter() {
                        v2.insert(SerBorrowIndex::from_u32(i.as_u32()));
                    }
                    v2
                })
            })
            .collect();

        // Determine the bijection between universal regions in signature-land and inference-land.
        let mut universal_to_vid = FxHashMap::default();
        for arg in GenericArgs::identity_for_item(tcx, tcx.typeck_root_def_id(orig_did)) {
            let Some(re) = arg.as_region() else {
                continue;
            };

            universal_to_vid.insert(re, facts.region_inference_context.to_region_vid(re));
            universal_to_vid.insert(
                tcx.lifetimes.re_static,
                facts
                    .region_inference_context
                    .to_region_vid(tcx.lifetimes.re_static),
            );
        }

        // Now, use the region information to determine which locals are leaked
        let mut leaked_locals = FxHashMap::default();
        let mut leaked_local_def_spans = FxHashMap::default();
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

            // Determine which nodes are reachable from our universal regions.
            for (&origin_real, &origin_vid) in &universal_to_vid {
                let mut leaked_res = FxHashSet::default();

                let Some(&origin) = cst_nodes.get(&origin_vid) else {
                    continue;
                };

                let mut dfs = Dfs::new(&cst_graph, origin);

                while let Some(reachable) = dfs.next(&cst_graph) {
                    leaked_res.insert(reachable);
                }

                // Finally, let's go through each local to see if it has any regions linked to the
                // return type.
                let leaked_locals: &mut Vec<_> = leaked_locals.entry(origin_real).or_default();

                for (local, info) in facts.body.local_decls.iter_enumerated() {
                    let mut was_used = false;

                    for used in extract_free_region_list(tcx, info.ty, re_as_vid) {
                        let Some(used) = cst_nodes.get(&used) else {
                            continue;
                        };

                        if leaked_res.contains(used) {
                            leaked_locals.push(local);
                            was_used = true;
                        }
                    }

                    if was_used {
                        leaked_local_def_spans.insert(local, info.source_info.span);
                    }
                }
            }
        }

        Self {
            borrows,
            overlaps,
            leaked_locals,
            leaked_local_def_spans,
        }
    }

    pub fn validate_overlaps(
        &self,
        tcx: TyCtxt<'tcx>,
        mut are_conflicting: impl FnMut(Pair<Local>) -> Option<(String, Pair<(Mutability, String)>)>,
    ) {
        let dcx = tcx.dcx();

        for (&new_bw, conflicts) in &self.overlaps {
            for old_bw in conflicts.iter() {
                if old_bw == new_bw {
                    continue;
                }

                let (old_bw, old_bw_span) = self.borrows[&old_bw];
                let (new_bw, new_bw_span) = self.borrows[&new_bw];

                let Some((conflict, borrows)) = (are_conflicting)(Pair::new(old_bw, new_bw)) else {
                    continue;
                };

                let borrows = borrows.nat();
                let (old_bw_mut, old_reason) = borrows.left;
                let (new_bw_mut, new_reason) = borrows.right;

                assert!(!old_bw_mut.is_compatible_with(new_bw_mut));

                // Report the conflict
                dcx.struct_span_err(
                    new_bw_span,
                    format!("conflicting borrows on token {conflict}"),
                )
                .with_span_label(
                    old_bw_span,
                    format!(
                        "value first borrowed {}",
                        match old_bw_mut {
                            Mutability::Not => "immutably",
                            Mutability::Mut => "mutably",
                        }
                    ),
                )
                .with_span_label(
                    new_bw_span,
                    format!(
                        "value later borrowed {}",
                        match new_bw_mut {
                            Mutability::Not => "immutably",
                            Mutability::Mut => "mutably",
                        }
                    ),
                )
                .with_help(format!("first borrow originates from {old_reason}"))
                .with_help(format!("later borrow originates from {new_reason}"))
                .emit();
            }
        }
    }

    pub fn validate_leaks(
        &self,
        tcx: TyCtxt<'tcx>,
        mut can_leak: impl FnMut(Region<'tcx>, Local) -> Option<String>,
    ) {
        for (&region, locals) in &self.leaked_locals {
            for &local in locals {
                let Some(deny_reason) = (can_leak)(region, local) else {
                    continue;
                };

                tcx.dcx().span_err(
                    self.leaked_local_def_spans[&local],
                    format!("cannot leak local variable {deny_reason}"),
                );
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
