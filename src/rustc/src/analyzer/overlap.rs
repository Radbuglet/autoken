use std::collections::hash_map;

use rustc_borrowck::{
    borrow_set::BorrowData,
    consumers::{
        calculate_borrows_out_of_scope_at_location, get_body_with_borrowck_facts, BorrowIndex,
        ConsumerOptions,
    },
};
use rustc_data_structures::fx::FxIndexMap;
use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::def_id::LocalDefId;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{
        BasicBlock, Body, BorrowKind, CallReturnPlaces, Local, Location, Mutability, Statement,
        Terminator, TerminatorEdges,
    },
    ty::{ParamEnv, Ty, TyCtxt},
};
use rustc_mir_dataflow::{
    fmt::DebugWithContext, Analysis, AnalysisDomain, Forward, GenKill, GenKillAnalysis,
};
use rustc_span::Span;

use crate::util::ty::{GenericTransformer, MaybeConcretizedFunc, MutabilityExt};

// === Analysis === //

#[derive(Debug, Clone)]
pub struct BodyOverlapFacts<'tcx> {
    pub overlaps: Vec<OverlapPlace<'tcx>>,
}

#[derive(Debug, Clone)]
pub struct OverlapPlace<'tcx> {
    pub span: Span,
    pub active: Vec<(Ty<'tcx>, Mutability)>,
}

impl<'tcx> BodyOverlapFacts<'tcx> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        did: LocalDefId,
        mut local_key: impl FnMut(Local) -> Vec<Ty<'tcx>>,
    ) -> Self {
        // Determine the start and end locations of our borrows.
        let facts = get_body_with_borrowck_facts(tcx, did, ConsumerOptions::RegionInferenceContext);
        let start_map = &facts.borrow_set.location_map;
        let end_map = calculate_borrows_out_of_scope_at_location(
            &facts.body,
            &facts.region_inference_context,
            &facts.borrow_set,
        );

        // Run fix-point analysis to figure out which sections of code have which borrows.
        let mut results = RegionAwareLiveness {
            start_map,
            end_map: &end_map,
        }
        .into_engine(tcx, &facts.body)
        .iterate_to_fixpoint()
        .into_results_cursor(&facts.body);

        // Determine overlap sets.
        let mut overlaps = Vec::new();

        for (bb_loc, bb) in facts.body.basic_blocks.iter_enumerated() {
            let locs = bb
                .statements
                .iter()
                .enumerate()
                .map(|(stmt_loc, stmt)| {
                    (
                        Location {
                            block: bb_loc,
                            statement_index: stmt_loc,
                        },
                        stmt.source_info.span,
                    )
                })
                .chain(bb.terminator.as_ref().into_iter().map(|terminator| {
                    (
                        Location {
                            block: bb_loc,
                            statement_index: bb.statements.len(),
                        },
                        terminator.source_info.span,
                    )
                }));

            for (loc, span) in locs {
                results.seek_before_primary_effect(loc);
                let state = results.get();

                let mut active = Vec::new();

                for borrow in state.iter() {
                    let borrow = start_map.get_index(borrow.as_usize()).unwrap().1;
                    let local = borrow.borrowed_place.local;

                    for local_key in local_key(local) {
                        let mutability = match borrow.kind {
                            BorrowKind::Shared => Mutability::Not,
                            BorrowKind::Fake => unreachable!(),
                            BorrowKind::Mut { .. } => Mutability::Mut,
                        };

                        active.push((local_key, mutability));
                    }
                }

                // TODO: Optimize representation
                overlaps.push(OverlapPlace { span, active });
            }
        }

        Self { overlaps }
    }

    pub fn validate(
        &self,
        tcx: TyCtxt<'tcx>,
        param_env: ParamEnv<'tcx>,
        instance: MaybeConcretizedFunc<'tcx>,
    ) {
        let mut borrows = FxHashMap::default();

        for overlap in &self.overlaps {
            borrows.clear();

            for &(key, mutability) in &overlap.active {
                let key = instance.instantiate_arg(tcx, param_env, key);

                match borrows.entry(key) {
                    hash_map::Entry::Vacant(entry) => {
                        entry.insert(mutability);
                    }
                    hash_map::Entry::Occupied(entry) => {
                        let entry = entry.into_mut();
                        if entry.is_mut() || mutability.is_mut() {
                            // TODO: Improve diagnostics
                            tcx.dcx().span_err(
                                overlap.span,
                                format!("conflicting borrows on token {key} at this point"),
                            );
                        }

                        entry.upgrade(mutability);
                    }
                }
            }
        }
    }
}

// === Dataflow Analyzer === //

pub struct RegionAwareLiveness<'tcx, 'a> {
    start_map: &'a FxIndexMap<Location, BorrowData<'tcx>>,
    end_map: &'a FxIndexMap<Location, Vec<BorrowIndex>>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct LiveSet(pub FxHashSet<BorrowIndex>);

impl<'tcx, 'a> AnalysisDomain<'tcx> for RegionAwareLiveness<'tcx, 'a> {
    type Domain = BitSet<BorrowIndex>;
    type Direction = Forward;

    const NAME: &'static str = "region_aware_liveness";

    fn bottom_value(&self, _body: &Body<'tcx>) -> Self::Domain {
        BitSet::new_empty(self.start_map.len())
    }

    fn initialize_start_block(&self, _body: &Body<'tcx>, _state: &mut Self::Domain) {
        // (no locals are live on function entry)
    }
}

impl<'tcx, 'a> RegionAwareLiveness<'tcx, 'a> {
    fn handle_start_loc(&self, trans: &mut impl GenKill<BorrowIndex>, location: Location) {
        if let Some(borrow) = self.start_map.get_index_of(&location) {
            trans.gen(BorrowIndex::from_usize(borrow));
        }
    }

    fn handle_end_loc(&self, trans: &mut impl GenKill<BorrowIndex>, location: Location) {
        if let Some(borrows) = self.end_map.get(&location) {
            trans.kill_all(borrows.iter().copied());
        }
    }
}

impl<'tcx, 'a> GenKillAnalysis<'tcx> for RegionAwareLiveness<'tcx, 'a> {
    type Idx = BorrowIndex;

    fn domain_size(&self, _body: &Body<'tcx>) -> usize {
        self.start_map.len()
    }

    fn statement_effect(
        &mut self,
        trans: &mut impl GenKill<Self::Idx>,
        _statement: &Statement<'tcx>,
        location: Location,
    ) {
        self.handle_end_loc(trans, location);
    }

    fn before_statement_effect(
        &mut self,
        trans: &mut impl GenKill<Self::Idx>,
        _statement: &Statement<'tcx>,
        location: Location,
    ) {
        self.handle_start_loc(trans, location);
    }

    fn terminator_effect<'mir>(
        &mut self,
        trans: &mut Self::Domain,
        terminator: &'mir Terminator<'tcx>,
        location: Location,
    ) -> TerminatorEdges<'mir, 'tcx> {
        self.handle_end_loc(trans, location);
        terminator.edges()
    }

    fn before_terminator_effect(
        &mut self,
        trans: &mut Self::Domain,
        _terminator: &Terminator<'tcx>,
        location: Location,
    ) {
        self.handle_start_loc(trans, location);
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
