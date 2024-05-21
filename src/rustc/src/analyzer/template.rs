use rustc_hir::{def::DefKind, def_id::LocalDefId};
use rustc_macros::{TyDecodable, TyEncodable};
use rustc_middle::{
    mir::{BasicBlock, Local, Terminator, TerminatorKind},
    ty::{
        fold::RegionFolder, BoundVar, GenericArgsRef, Instance, InstanceDef, Mutability, ParamEnv,
        Region, RegionKind, Ty, TyCtxt, TypeFoldable,
    },
};
use rustc_span::{Span, Symbol};

use crate::util::{
    feeder::{
        feed,
        feeders::{
            AssociatedItemFeeder, DefKindFeeder, MirBuiltFeeder, MirBuiltStasher,
            OptLocalDefIdToHirIdFeeder, VisibilityFeeder,
        },
        read_feed,
    },
    hash::{FxHashMap, FxHashSet},
    mir::{get_callee_from_terminator, TerminalCallKind},
    ty::{
        find_region_with_name, get_fn_sig_maybe_closure, try_resolve_instance,
        FunctionCallAndRegions, GenericTransformer, MaybeConcretizedFunc, MutabilityExt,
    },
};

use super::{
    mir::TokenMirBuilder,
    overlap::BodyOverlapFacts,
    sets::{instantiate_set_proc, parse_tie_func},
    sym,
    trace::TraceFacts,
};

#[derive(Debug, Clone, TyEncodable, TyDecodable)]
pub struct BodyTemplateFacts<'tcx> {
    /// The set of region-type-set pairs that can be leaked from the current function.
    pub permitted_leaks: Vec<(Region<'tcx>, Ty<'tcx>)>,

    /// The set of calls made by this function.
    pub calls: Vec<TemplateCall<'tcx>>,

    /// The set of locals held by yields.
    pub yield_locals: FxHashSet<Local>,
}

#[derive(Debug, Clone, TyEncodable, TyDecodable)]
pub struct TemplateCall<'tcx> {
    // The span of the function call.
    pub span: Span,

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
        param_env_user: ParamEnv<'tcx>,
        orig_id: LocalDefId,
    ) -> (Self, LocalDefId) {
        let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
            unreachable!();
        };

        let mut body_mutator = TokenMirBuilder::new(tcx, param_env_user, &mut body);
        let mut permitted_leaks = Vec::new();
        let mut yield_locals = FxHashSet::default();
        let mut calls = Vec::new();
        let fn_ret_ty = get_fn_sig_maybe_closure(tcx, orig_id.to_def_id());

        let bb_count = body_mutator.body().basic_blocks.len();
        for bb in 0..bb_count {
            let bb = BasicBlock::from_usize(bb);

            // Yields can be treated as if they were function calls borrowing everything.
            if let Some(Terminator {
                kind: TerminatorKind::Yield { .. },
                ..
            }) = &body_mutator.body()[bb].terminator
            {
                yield_locals.insert(body_mutator.ensure_not_borrowed_at(bb));
            }

            // If the current basic block is a call...
            let (span, callee) = match get_callee_from_terminator(
                tcx,
                param_env_user,
                MaybeConcretizedFunc {
                    def: InstanceDef::Item(orig_id.to_def_id()),
                    args: None,
                },
                &body_mutator.body().basic_blocks[bb].terminator,
                &body_mutator.body().local_decls,
            ) {
                Some(TerminalCallKind::Static(span, callee)) => (span, callee),
                Some(TerminalCallKind::Generic(span, callee)) => (span, callee),
                _ => {
                    continue;
                }
            };

            // Determine whether it has any special effects on ties.
            'tie: {
                let Some(func) = parse_tie_func(tcx, callee) else {
                    break 'tie;
                };

                let Some(tied_to) = func.tied_to else {
                    break 'tie;
                };

                let region = match find_region_with_name(
                    tcx,
                    fn_ret_ty.skip_binder().skip_binder(),
                    tied_to,
                ) {
                    Ok(region) => region,
                    Err(symbols) => {
                        tcx.dcx()
                            .struct_err(format!(
                                "lifetime with name {tied_to} not found in output of function{}",
                                if symbols.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        "; found {}",
                                        symbols
                                            .iter()
                                            .map(|v| v.to_string())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                }
                            ))
                            .with_span(span)
                            .with_note(
                                "it is not currently possible to tie lifetimes which appear in input \
                                 parameters to tokens",
                            )
                            .emit();
                        break 'tie;
                    }
                };

                // HACK: Reject regions which appear anywhere other than the output type.
                if !func.is_unsafe {
                    let mut soundness_hole = false;

                    if !matches!(region.kind(), RegionKind::ReEarlyParam(_)) {
                        soundness_hole = true;
                    }

                    for bound in param_env_user.caller_bounds() {
                        bound.fold_with(&mut RegionFolder::new(tcx, &mut |re, _| {
                            if re == region {
                                soundness_hole = true;
                            }

                            re
                        }));
                    }

                    if soundness_hole {
                        tcx.dcx()
                            .struct_err(
                                "ties to lifetimes appearing in generic bounds or input parameters \
                                types are currently rejected due to soundness issues",
                            )
                            .with_span(span)
                            .with_help("if this use is safe, prefix the `tie!` directive with `unsafe`")
                            .emit();
                    }
                }

                permitted_leaks.push((region, func.acquired_set));
            }

            // Determine mask
            let mask = FunctionCallAndRegions::new(tcx, param_env_user, callee);

            // Give it the opportunity to kill off some borrows and tie stuff to itself.
            let enb_local = body_mutator.ensure_not_borrowed_at(bb);
            let tied_locals = (0..mask.param_count)
                .map(|i| body_mutator.tie_token_to_function_return(bb, mask, BoundVar::from_u32(i)))
                .collect();

            calls.push(TemplateCall {
                span,
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
                permitted_leaks,
                calls,
                yield_locals,
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
        let mut borrowing_locals =
            FxHashMap::<Local, (Instance<'tcx>, FxHashMap<Ty<'tcx>, Mutability>)>::default();

        fn add_local_borrow<'tcx>(
            bs: &mut FxHashMap<Local, (Instance<'tcx>, FxHashMap<Ty<'tcx>, Mutability>)>,
            local: Local,
            token: Ty<'tcx>,
            instance: Instance<'tcx>,
            mutability: Mutability,
        ) {
            bs.entry(local)
                .or_insert((instance, FxHashMap::default()))
                .1
                .entry(token)
                .or_insert(Mutability::Not)
                .upgrade(mutability);
        }

        for call in &self.calls {
            let callee = match try_resolve_instance(
                tcx,
                ParamEnv::reveal_all(),
                args.instantiate_arg(tcx, ParamEnv::reveal_all(), call.func.instance),
            ) {
                Ok(Some(callee)) => callee,
                Ok(None) | Err(_) => continue,
            };

            let Some(callee_facts) = trace.facts(callee) else {
                continue;
            };

            for (&borrow_ty, &(borrow_mut, borrow_sym)) in &callee_facts.borrows {
                add_local_borrow(
                    &mut borrowing_locals,
                    call.prevent_call_local,
                    borrow_ty,
                    callee,
                    borrow_mut,
                );

                if let Some(borrow_sym) = borrow_sym {
                    let Some(linked) = call.func.get_linked(tcx, Some(args), borrow_sym) else {
                        tcx.dcx().span_err(
                            call.span,
                            format!(
                                "failed to find lifetime {borrow_sym} to which {borrow_ty} is tied \
                                 in the return type of the function"
                            ),
                        );

                        continue;
                    };
                    for tie_local in linked {
                        add_local_borrow(
                            &mut borrowing_locals,
                            call.tied_locals[tie_local.as_usize()],
                            borrow_ty,
                            callee,
                            borrow_mut,
                        );
                    }
                }
            }
        }

        // Validate borrow overlaps
        rustc_middle::ty::print::with_forced_trimmed_paths! {
            overlaps.validate_overlaps(tcx, |types| {
                // Handle yields
                for types in types.orders() {
                    let Some(first) = borrowing_locals.get(types.left) else {
                        continue;
                    };

                    if !self.yield_locals.contains(types.right) {
                        continue;
                    }

                    let Some((token, mutability)) = first.1.iter().next() else {
                        continue;
                    };

                    return Some((
                        token.to_string(),
                        types.map(
                            (*mutability, first.0.to_string()),
                            (Mutability::Mut, "`.await`".to_string()),
                        ),
                    ));
                }

                // Handle regular borrows
                let types = types.map(
                    borrowing_locals.get(&types.left)?,
                    borrowing_locals.get(&types.right)?,
                );

                let types = types.maybe_rev(types.left.1.len() <= types.right.1.len());

                for (token, first_mut) in &types.left.1 {
                    let Some(second_mut) = types.right.1.get(token) else {
                        continue;
                    };

                    if !first_mut.is_compatible_with(*second_mut) {
                        return Some((
                            token.to_string(),
                            types.map(
                                (*first_mut, types.left.0.to_string()),
                                (*second_mut, types.right.0.to_string()),
                            ),
                        ));
                    }
                }

                None
            })
        }

        // Validate leaked locals
        let mut permitted_leaks = FxHashSet::default();
        for &(re, set) in &self.permitted_leaks {
            let set = args.instantiate_arg(tcx, ParamEnv::reveal_all(), set);

            instantiate_set_proc(tcx, set, &mut |ty, _| {
                permitted_leaks.insert((re, ty));
            });
        }

        overlaps.validate_leaks(tcx, |re, local| {
            let borrows = borrowing_locals.get(&local)?;

            for &borrow in borrows.1.keys() {
                if permitted_leaks.contains(&(re, borrow)) {
                    continue;
                }

                return Some(format!(
                    "since the token {borrow} is not tied to the return region {}",
                    re.get_name().unwrap_or(sym::ANON_LT.get()),
                ));
            }

            None
        });
    }
}
