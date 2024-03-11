use rustc_data_structures::graph::WithStartNode;
use rustc_hir::def::DefKind;
use rustc_index::IndexVec;
use rustc_middle::{
    mir::{
        AggregateKind, BorrowKind, LocalDecl, MutBorrowKind, Operand, Place, Rvalue, SourceInfo,
        SourceScope, Statement, StatementKind, Terminator, TerminatorKind,
    },
    ty::{GenericArg, GenericArgKind, List, Ty, TyCtxt, TyKind},
};
use rustc_span::{
    source_map::{dummy_spanned, Spanned},
    Symbol,
};

use crate::util::feeder::{feed, feeders::MirBuiltFeeder};

// === Engine === //

pub struct AnalysisDriver<'tcx> {
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> AnalysisDriver<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self { tcx }
    }

    pub fn analyze(&mut self, tcx: TyCtxt<'_>) {
        // Find the main function
        let Some((main_fn, _)) = tcx.entry_fn(()) else {
            return;
        };

        let Some(main_fn) = main_fn.as_local() else {
            return;
        };

        // Find helper functions
        let tie_mut_shadow_fn = {
            let mut fn_id = None;
            for &item in tcx.hir().root_module().item_ids {
                if tcx.hir().name(item.hir_id()) == sym::__autoken_tie_mut_shadow.get() {
                    fn_id = Some(item.owner_id.def_id);
                }
            }
            fn_id.expect("missing `__autoken_tie_mut_shadow` in crate root")
        };

        // Get the MIR for the function.
        let body = tcx.mir_built(main_fn);

        // Create a new body for it.
        let mut body = body.borrow().clone();
        let token_local = body
            .local_decls
            .push(LocalDecl::new(tcx.types.unit, body.span));

        let token_local_ref = body.local_decls.push(LocalDecl::new(
            Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit),
            body.span,
        ));

        let start = body.basic_blocks.start_node();
        let source_info = SourceInfo {
            scope: SourceScope::from_u32(0),
            span: body.span,
        };

        body.basic_blocks.as_mut()[start].statements.extend([
            Statement {
                source_info,
                kind: StatementKind::Assign(Box::new((
                    Place {
                        local: token_local,
                        projection: List::empty(),
                    },
                    Rvalue::Aggregate(Box::new(AggregateKind::Tuple), IndexVec::new()),
                ))),
            },
            Statement {
                source_info,
                kind: StatementKind::Assign(Box::new((
                    Place {
                        local: token_local_ref,
                        projection: List::empty(),
                    },
                    Rvalue::Ref(
                        tcx.lifetimes.re_erased,
                        BorrowKind::Mut {
                            kind: MutBorrowKind::Default,
                        },
                        Place {
                            local: token_local,
                            projection: List::empty(),
                        },
                    ),
                ))),
            },
        ]);

        for bb in body.basic_blocks.as_mut().iter_mut() {
            let Some(Terminator {
                kind: TerminatorKind::Call { func, args, .. },
                ..
            }) = &mut bb.terminator
            else {
                continue;
            };

            let func_ty = func.ty(&body.local_decls, tcx);
            let TyKind::FnDef(callee_id, generics) = func_ty.kind() else {
                continue;
            };
            let callee_id = *callee_id;

            if tcx.item_name(callee_id) == sym::__autoken_tie_mut.get() {
                *func = Operand::function_handle(
                    tcx,
                    tie_mut_shadow_fn.to_def_id(),
                    *generics,
                    func.span(&body.local_decls),
                );

                args.push(dummy_spanned(Operand::Move(Place {
                    local: token_local_ref,
                    projection: List::empty(),
                })));
            }
        }

        // Define and setup the shadow.
        let main_fn_shadow = tcx.at(body.span).create_def(
            tcx.local_parent(main_fn),
            Symbol::intern(&format!(
                "{}_autoken_shadow",
                tcx.item_name(main_fn.to_def_id()),
            )),
            DefKind::Fn,
        );

        main_fn_shadow.opt_local_def_id_to_hir_id(tcx.opt_local_def_id_to_hir_id(main_fn));
        let main_fn_shadow = main_fn_shadow.def_id();

        // Give it some MIR
        feed::<MirBuiltFeeder>(tcx, main_fn_shadow, tcx.alloc_steal_mir(body));

        // Borrow check the shadow function
        dbg!(&tcx.mir_borrowck(main_fn_shadow));
    }
}

#[allow(non_upper_case_globals)]
mod sym {
    use crate::util::mir::CachedSymbol;

    pub static __autoken_permit_escape: CachedSymbol = CachedSymbol::new("__autoken_permit_escape");

    pub static __autoken_tie_ref: CachedSymbol = CachedSymbol::new("__autoken_tie_ref");

    pub static __autoken_tie_mut: CachedSymbol = CachedSymbol::new("__autoken_tie_mut");

    pub static __autoken_tie_ref_shadow: CachedSymbol =
        CachedSymbol::new("__autoken_tie_ref_shadow");

    pub static __autoken_tie_mut_shadow: CachedSymbol =
        CachedSymbol::new("__autoken_tie_mut_shadow");
}
