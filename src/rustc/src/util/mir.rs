use std::sync::OnceLock;

use rustc_data_structures::steal::Steal;
use rustc_hir::{def_id::LocalDefId, ImplItemKind, ItemKind, Node, TraitFn, TraitItemKind};
use rustc_index::IndexVec;
use rustc_middle::{
    mir::{Body, Local, LocalDecl, Terminator, TerminatorKind},
    ty::{EarlyBinder, Instance, InstanceDef, ParamEnv, Ty, TyCtxt, TyKind, TypeAndMut},
};
use rustc_span::Symbol;

// === Misc === //

pub struct CachedSymbol {
    raw: &'static str,
    sym: OnceLock<Symbol>,
}

impl CachedSymbol {
    pub const fn new(raw: &'static str) -> Self {
        Self {
            raw,
            sym: OnceLock::new(),
        }
    }

    pub fn get(&self) -> Symbol {
        *self.sym.get_or_init(|| Symbol::intern(self.raw))
    }
}

// === get_callee_from_terminator === //

pub fn get_static_callee_from_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    caller: &Instance<'tcx>,
    caller_local_decls: &IndexVec<Local, LocalDecl<'tcx>>,
    terminator: &Terminator<'tcx>,
) -> Option<Instance<'tcx>> {
    let TerminatorKind::Call { func: callee, .. } = &terminator.kind else {
        return None;
    };

    let callee = callee.ty(caller_local_decls, tcx);
    let callee = caller.instantiate_mir_and_normalize_erasing_regions(
        tcx,
        ParamEnv::reveal_all(),
        EarlyBinder::bind(callee),
    );

    let TyKind::FnDef(callee_id, generics) = callee.kind() else {
        return None;
    };

    Some(Instance::expect_resolve(
        tcx,
        ParamEnv::reveal_all(),
        *callee_id,
        generics,
    ))
}

// === `safeishly_grab_def_id_mir` === //

pub fn safeishly_grab_def_id_mir(tcx: TyCtxt<'_>, id: LocalDefId) -> Option<&Steal<Body<'_>>> {
    // Copied from `rustc_hir_typecheck::primary_body_of`
    match tcx.hir_node_by_def_id(id) {
        Node::Item(item) => match item.kind {
            ItemKind::Const(_, _, _) | ItemKind::Static(_, _, _) => {
                // (fallthrough)
            }
            ItemKind::Fn(_, _, _) => {
                // (fallthrough)
            }
            _ => return None,
        },
        Node::TraitItem(item) => match item.kind {
            TraitItemKind::Const(_, Some(_)) => {
                // (fallthrough)
            }
            TraitItemKind::Fn(_, TraitFn::Provided(_)) => {
                // (fallthrough)
            }
            _ => return None,
        },
        Node::ImplItem(item) => match item.kind {
            ImplItemKind::Const(_, _) => {
                // (fallthrough)
            }
            ImplItemKind::Fn(_, _) => {
                // (fallthrough)
            }
            _ => return None,
        },
        Node::AnonConst(_) => {
            // (fallthrough)
        }
        _ => return None,
    }

    Some(tcx.mir_built(id))
}

// === `safeishly_grab_instance_mir` === //

#[derive(Debug)]
pub enum MirGrabResult<'tcx> {
    Found(&'tcx Body<'tcx>),
    Dynamic,
    BottomsOut,
}

pub fn safeishly_grab_instance_mir<'tcx>(
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

        // This is a shim around `FnDef` (or maybe an `FnPtr`?) for `FnTrait::call_x`. We generate
        // the shim MIR for it and let the regular instance body processing handle it.
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
pub fn get_unsized_ty<'tcx>(
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
