use rustc_hir::{def::DefKind, def_id::LocalDefId};
use rustc_middle::{
    mir::{BasicBlock, Local},
    ty::{BoundVar, GenericArgsRef, InstanceDef, Mutability, ParamEnv, Ty, TyCtxt},
};
use rustc_span::Symbol;

use crate::util::{
    feeder::{
        feed,
        feeders::{
            AssociatedItemFeeder, DefKindFeeder, MirBuiltFeeder, MirBuiltStasher,
            OptLocalDefIdToHirIdFeeder, VisibilityFeeder,
        },
        read_feed,
    },
    hash::FxHashMap,
    mir::{get_callee_from_terminator, TerminalCallKind},
    ty::{FunctionCallAndRegions, GenericTransformer, MaybeConcretizedFunc, MutabilityExt},
};

use super::{
    mir::TokenMirBuilder,
    overlap::BodyOverlapFacts,
    sets::{parse_tie_func, ParsedTieCall},
    sym,
    trace::TraceFacts,
};

pub struct BodyTemplateFacts<'tcx> {
    /// The set of tie directives in this function.
    pub tied_to: Vec<ParsedTieCall<'tcx>>,

    /// The set of calls made by this function.
    pub calls: Vec<TemplateCall<'tcx>>,
}

pub struct TemplateCall<'tcx> {
    /// The function we called.
    pub func: FunctionCallAndRegions<'tcx>,

    /// The local borrowed mutably before the call is made.
    pub prevent_call_local: Local,

    /// The locals to which each free lifetime is tied after the call has been
    /// made.
    pub tied_locals: Vec<Local>,
}

impl<'tcx> BodyTemplateFacts<'tcx> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        param_env: ParamEnv<'tcx>,
        orig_id: LocalDefId,
    ) -> (Self, LocalDefId) {
        let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
            unreachable!();
        };

        let mut body_mutator = TokenMirBuilder::new(tcx, param_env, &mut body);
        let mut calls = Vec::new();
        let mut ties = Vec::new();

        let bb_count = body_mutator.body().basic_blocks.len();
        for bb in 0..bb_count {
            let bb = BasicBlock::from_usize(bb);

            // If the current basic block is a call...
            let callee = match get_callee_from_terminator(
                tcx,
                param_env,
                MaybeConcretizedFunc {
                    def: InstanceDef::Item(orig_id.to_def_id()),
                    args: None,
                },
                &body_mutator.body().basic_blocks[bb].terminator,
                &body_mutator.body().local_decls,
            ) {
                Some(TerminalCallKind::Static(_, callee)) => callee,
                Some(TerminalCallKind::Generic(_, callee)) => callee,
                _ => continue,
            };

            // Determine whether it has any special effects on ties.
            if let Some(func) = parse_tie_func(tcx, callee) {
                ties.push(func);
            }

            // Determine mask
            let mask = FunctionCallAndRegions::new(tcx, param_env, callee);

            // Give it the opportunity to kill off some borrows and tie stuff
            // to itself.
            let enb_local = body_mutator.ensure_not_borrowed_at(bb);
            let tied_locals = (0..mask.param_count)
                .map(|i| body_mutator.tie_token_to_function_return(bb, mask, BoundVar::from_u32(i)))
                .collect();

            calls.push(TemplateCall {
                prevent_call_local: enb_local,
                tied_locals,
                func: mask,
            });
        }

        drop(body_mutator);

        // Feed the query system the shadow function's properties.
        let shadow_kind = tcx.def_kind(orig_id);
        let shadow_def = tcx
            .create_def(
                tcx.local_parent(orig_id),
                Symbol::intern(&format!(
                    "{}_autoken_shadow",
                    tcx.opt_item_name(orig_id.to_def_id())
                        .unwrap_or_else(|| sym::unnamed.get()),
                )),
                shadow_kind,
            )
            .def_id();

        feed::<DefKindFeeder>(tcx, shadow_def, shadow_kind);
        feed::<MirBuiltFeeder>(tcx, shadow_def, tcx.alloc_steal_mir(body));
        feed::<OptLocalDefIdToHirIdFeeder>(
            tcx,
            shadow_def,
            Some(tcx.local_def_id_to_hir_id(orig_id)),
        );
        feed::<VisibilityFeeder>(tcx, shadow_def, tcx.visibility(orig_id));

        if shadow_kind == DefKind::AssocFn {
            feed::<AssociatedItemFeeder>(tcx, shadow_def, tcx.associated_item(orig_id));
        }

        (
            Self {
                calls,
                tied_to: Vec::new(),
            },
            shadow_def,
        )
    }

    pub fn validate(
        &self,
        tcx: TyCtxt<'tcx>,
        trace: &TraceFacts<'tcx>,
        overlaps: &BodyOverlapFacts<'tcx>,
        args: GenericArgsRef<'tcx>,
    ) {
        // Determine what each local borrows
        let mut borrowing_locals = FxHashMap::<Local, FxHashMap<Ty<'tcx>, Mutability>>::default();

        fn add_local_borrow<'tcx>(
            bs: &mut FxHashMap<Local, FxHashMap<Ty<'tcx>, Mutability>>,
            local: Local,
            token: Ty<'tcx>,
            mutability: Mutability,
        ) {
            bs.entry(local)
                .or_default()
                .entry(token)
                .or_insert(Mutability::Not)
                .upgrade(mutability);
        }

        for call in &self.calls {
            let callee = args.instantiate_arg(tcx, ParamEnv::reveal_all(), call.func.instance);
            let Some(callee) = trace.facts(callee) else {
                continue;
            };

            for (&borrow_ty, &(borrow_mut, borrow_sym)) in &callee.borrows {
                add_local_borrow(
                    &mut borrowing_locals,
                    call.prevent_call_local,
                    borrow_ty,
                    borrow_mut,
                );

                if let Some(borrow_sym) = borrow_sym {
                    for tie_local in call.func.get_linked(tcx, Some(args), borrow_sym).unwrap() {
                        add_local_borrow(
                            &mut borrowing_locals,
                            call.tied_locals[tie_local.as_usize()],
                            borrow_ty,
                            borrow_mut,
                        );
                    }
                }
            }
        }

        // Validate borrow overlaps
        overlaps.validate_overlaps(tcx, |first, second| {
            let first = borrowing_locals.get(&first)?;
            let second = borrowing_locals.get(&second)?;

            let (first, second) = if first.len() > second.len() {
                (first, second)
            } else {
                (second, first)
            };

            for (token, first_mut) in first {
                let Some(second_mut) = second.get(token) else {
                    continue;
                };

                if !first_mut.is_compatible_with(*second_mut) {
                    // FIXME: Mutabilities need to be swapped
                    return Some((token.to_string(), *first_mut, *second_mut));
                }
            }

            None
        });

        // Validate leaked locals
        // TODO
    }
}
