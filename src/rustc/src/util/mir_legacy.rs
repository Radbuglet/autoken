use rustc_index::IndexVec;
use rustc_middle::{
    mir::{Body, Local, LocalDecl, Operand},
    ty::{EarlyBinder, Instance, InstanceDef, ParamEnv, TyCtxt, TyKind},
};

#[derive(Debug)]
pub enum MirGrabResult<'tcx> {
    Found(&'tcx Body<'tcx>),
    Dynamic,
    BottomsOut,
}

pub fn get_static_callee_from_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    caller: &Instance<'tcx>,
    caller_local_decls: &IndexVec<Local, LocalDecl<'tcx>>,
    callee: &Operand<'tcx>,
) -> Option<Instance<'tcx>> {
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
