use std::sync::OnceLock;

use rustc_data_structures::graph::WithStartNode;
use rustc_hir::def::DefKind;
use rustc_index::IndexVec;
use rustc_middle::{
    mir::{
        AggregateKind, Body, BorrowKind, LocalDecl, MutBorrowKind, Place, Rvalue, SourceInfo,
        SourceScope, Statement, StatementKind,
    },
    ty::{InstanceDef, List, Ty, TyCtxt, TyKind, TypeAndMut},
};
use rustc_span::Symbol;

use crate::feeder::{feed, feeders::MirBuiltFeeder};

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

fn s_pluralize(v: i32) -> &'static str {
    if v == 1 {
        ""
    } else {
        "s"
    }
}

#[derive(Debug)]
enum MirGrabResult<'tcx> {
    Found(&'tcx Body<'tcx>),
    Dynamic,
    BottomsOut,
}

fn safeishly_grab_instance_mir<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: InstanceDef<'tcx>,
) -> MirGrabResult<'tcx> {
    match instance {
        // Items are defined by users and thus have MIR... even if they're from an external crate.
        InstanceDef::Item(item) => {
            // However, foreign items and lang-items don't have MIR
            if !tcx.is_foreign_item(item) {
                MirGrabResult::Found(tcx.instance_mir(instance))
            } else {
                MirGrabResult::BottomsOut
            }
        }

        // This is a shim around `FnDef` (or maybe an `FnPtr`?) for `FnTrait::call_x`. We generate the
        // shim MIR for it and let the regular instance body processing handle it.
        InstanceDef::FnPtrShim(_, _) => MirGrabResult::Found(tcx.instance_mir(instance)),

        // All the remaining things here require shims. We referenced...
        //
        // https://github.com/rust-lang/rust/blob/9c20ddd956426d577d77cb3f57a7db2227a3c6e9/compiler/rustc_mir_transform/src/shim.rs#L29
        //
        // ...to figure out which instance def types support this operation.

        // These are always supported.
        InstanceDef::ThreadLocalShim(_)
        | InstanceDef::DropGlue(_, _)
        | InstanceDef::ClosureOnceShim { .. }
        | InstanceDef::CloneShim(_, _)
        | InstanceDef::FnPtrAddrShim(_, _) => MirGrabResult::Found(tcx.instance_mir(instance)),

        // These are never supported and will never return to the user.
        InstanceDef::Intrinsic(_) => MirGrabResult::BottomsOut,

        // These are dynamic dispatches and should not be analyzed since we analyze them in a
        // different way.
        InstanceDef::VTableShim(_) | InstanceDef::ReifyShim(_) | InstanceDef::Virtual(_, _) => {
            MirGrabResult::Dynamic
        }

        // TODO: Handle these properly.
        InstanceDef::ConstructCoroutineInClosureShim { .. }
        | InstanceDef::CoroutineKindShim { .. } => MirGrabResult::Dynamic,
    }
}

// Referenced from https://github.com/rust-lang/rust/blob/4b85902b438f791c5bfcb6b1c5b476d5b88e2bef/compiler/rustc_codegen_cranelift/src/unsize.rs#L62
fn get_unsized_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    from_ty: Ty<'tcx>,
    to_ty: Ty<'tcx>,
) -> (Ty<'tcx>, Ty<'tcx>) {
    match (from_ty.kind(), to_ty.kind()) {
        // Reference unsizing
        (TyKind::Ref(_, a, _), TyKind::Ref(_, b, _))
        | (TyKind::Ref(_, a, _), TyKind::RawPtr(TypeAndMut { ty: b, mutbl: _ }))
        | (
            TyKind::RawPtr(TypeAndMut { ty: a, mutbl: _ }),
            TyKind::RawPtr(TypeAndMut { ty: b, mutbl: _ }),
        ) => get_unsized_ty(tcx, *a, *b),

        // Box unsizing
        (TyKind::Adt(def_a, _), TyKind::Adt(def_b, _)) if def_a.is_box() && def_b.is_box() => {
            get_unsized_ty(tcx, from_ty.boxed_ty(), to_ty.boxed_ty())
        }

        // Structural unsizing
        (TyKind::Adt(def_a, args_a), TyKind::Adt(def_b, args_b)) => {
            assert_eq!(def_a, def_b);

            for field in def_a.all_fields() {
                let from_ty = field.ty(tcx, args_a);
                let to_ty = field.ty(tcx, args_b);
                if from_ty != to_ty {
                    return get_unsized_ty(tcx, from_ty, to_ty);
                }
            }

            (from_ty, to_ty)
        }

        // Identity unsizing
        _ => (from_ty, to_ty),
    }
}

// === Symbols === //

struct CachedSymbol {
    raw: &'static str,
    sym: OnceLock<Symbol>,
}

impl CachedSymbol {
    const fn new(raw: &'static str) -> Self {
        Self {
            raw,
            sym: OnceLock::new(),
        }
    }

    fn get(&self) -> Symbol {
        *self.sym.get_or_init(|| Symbol::intern(self.raw))
    }
}

#[allow(non_upper_case_globals)]
mod sym {
    use super::CachedSymbol;

    pub static __autoken_permit_escape: CachedSymbol = CachedSymbol::new("__autoken_permit_escape");

    pub static __autoken_tie_ref: CachedSymbol = CachedSymbol::new("__autoken_tie_ref");

    pub static __autoken_tie_mut: CachedSymbol = CachedSymbol::new("__autoken_tie_mut");

    pub static __autoken_helper_limit_to: CachedSymbol =
        CachedSymbol::new("__autoken_helper_limit_to");
}
