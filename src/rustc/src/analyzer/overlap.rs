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
        mut local_key: impl FnMut(Local) -> Option<Ty<'tcx>>,
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
            for (stmt_loc, stmt) in bb.statements.iter().enumerate() {
                let loc = Location {
                    block: bb_loc,
                    statement_index: stmt_loc,
                };

                results.seek_before_primary_effect(loc);
                let state = results.get();

                let mut active = Vec::new();

                for borrow in state.iter() {
                    let borrow = start_map.get_index(borrow.as_usize()).unwrap().1;
                    let local = borrow.borrowed_place.local;
                    let Some(local_key) = local_key(local) else {
                        continue;
                    };

                    let mutability = match borrow.kind {
                        BorrowKind::Shared => Mutability::Not,
                        BorrowKind::Fake => unreachable!(),
                        BorrowKind::Mut { .. } => Mutability::Mut,
                    };

                    active.push((local_key, mutability));
                }

                overlaps.push(OverlapPlace {
                    span: stmt.source_info.span,
                    active,
                });
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
                            tcx.dcx().span_err(
                                overlap.span,
                                format!("AuToken token {key} is borrowed mutably twice here"),
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
