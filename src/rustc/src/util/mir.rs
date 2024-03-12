use std::sync::OnceLock;

use rustc_middle::{
    mir::{Body, Local, Operand, Place, ProjectionElem},
    ty::{InstanceDef, Ty, TyCtxt, TyKind, TypeAndMut},
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

// === `rename_mir_locals` === //

pub fn rename_mir_locals<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mut Body<'tcx>,
    mut renamer: impl FnMut(Local) -> Local,
) {
    for bb in body.basic_blocks.as_mut() {
        for stmt in &mut bb.statements {
            use rustc_middle::mir::StatementKind::*;

            match &mut stmt.kind {
                Assign(assign) => {
                    use rustc_middle::mir::Rvalue::*;

                    let (place, value) = &mut **assign;
                    rename_mir_place(tcx, place, &mut renamer);

                    match value {
                        Use(operand) => {
                            rename_mir_operand(tcx, operand, &mut renamer);
                        }
                        Repeat(operand, _ty_const) => {
                            rename_mir_operand(tcx, operand, &mut renamer)
                        }
                        Ref(_region, _kind, place) => {
                            rename_mir_place(tcx, place, &mut renamer);
                        }
                        ThreadLocalRef(_def_id) => {
                            // (nothing to do here)
                        }
                        AddressOf(_mut, place) => {
                            rename_mir_place(tcx, place, &mut renamer);
                        }
                        Len(place) => {
                            rename_mir_place(tcx, place, &mut renamer);
                        }
                        Cast(_kind, operand, _ty) => {
                            rename_mir_operand(tcx, operand, &mut renamer);
                        }
                        BinaryOp(_bin_op, sides) | CheckedBinaryOp(_bin_op, sides) => {
                            let (lhs, rhs) = &mut **sides;
                            rename_mir_operand(tcx, lhs, &mut renamer);
                            rename_mir_operand(tcx, rhs, &mut renamer);
                        }
                        NullaryOp(_null_op, _ty) => {
                            // (nothing to do here)
                        }
                        UnaryOp(_op, operand) => {
                            rename_mir_operand(tcx, operand, &mut renamer);
                        }
                        Discriminant(place) => {
                            rename_mir_place(tcx, place, &mut renamer);
                        }
                        Aggregate(_kind, fields) => {
                            for field in fields {
                                rename_mir_operand(tcx, field, &mut renamer);
                            }
                        }
                        ShallowInitBox(operand, _ty) => {
                            rename_mir_operand(tcx, operand, &mut renamer);
                        }
                        CopyForDeref(place) => {
                            rename_mir_place(tcx, place, &mut renamer);
                        }
                    }
                }
                FakeRead(read) => {
                    let (_cause, place) = &mut **read;
                    rename_mir_place(tcx, place, &mut renamer);
                }
                SetDiscriminant {
                    place,
                    variant_index: _,
                } => {
                    rename_mir_place(tcx, place, &mut renamer);
                }
                Deinit(place) => {
                    rename_mir_place(tcx, place, &mut renamer);
                }
                StorageLive(local) => {
                    *local = renamer(*local);
                }
                StorageDead(local) => {
                    *local = renamer(*local);
                }
                Retag(_kind, place) => {
                    rename_mir_place(tcx, place, &mut renamer);
                }
                PlaceMention(place) => {
                    rename_mir_place(tcx, place, &mut renamer);
                }
                AscribeUserType(place_ish, _ty) => {
                    let (place, _ty_proj) = &mut **place_ish;
                    rename_mir_place(tcx, place, &mut renamer);
                }
                Coverage(_coverage) => {
                    // (nothing to do here)
                }
                Intrinsic(intrinsic) => {
                    use rustc_middle::mir::NonDivergingIntrinsic::*;

                    match &mut **intrinsic {
                        Assume(operand) => rename_mir_operand(tcx, operand, &mut renamer),
                        CopyNonOverlapping(cno) => {
                            let rustc_middle::mir::CopyNonOverlapping { src, dst, count } = cno;
                            rename_mir_operand(tcx, src, &mut renamer);
                            rename_mir_operand(tcx, dst, &mut renamer);
                            rename_mir_operand(tcx, count, &mut renamer);
                        }
                    }
                }
                ConstEvalCounter => {
                    // (nothing to do here)
                }
                Nop => {
                    // (nothing to do here)
                }
            }
        }

        match &mut bb.terminator {
            Some(_) => todo!(),
            None => todo!(),
        }
    }
}

fn rename_mir_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    place: &mut Place<'tcx>,
    mut renamer: impl FnMut(Local) -> Local,
) {
    // Rename place origin
    place.local = renamer(place.local);

    // Rename place projections
    let mut rename_proj = |mut part| {
        if let ProjectionElem::Index(target) = &mut part {
            *target = renamer(*target);
        }

        part
    };
    let did_rename_projections = place
        .projection
        .iter()
        .any(|proj| proj != rename_proj(proj));

    if did_rename_projections {
        place.projection = tcx.mk_place_elems(
            place
                .projection
                .iter()
                .map(rename_proj)
                .collect::<Vec<_>>()
                .as_slice(),
        );
    }
}

fn rename_mir_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    operand: &mut Operand<'tcx>,
    renamer: impl FnMut(Local) -> Local,
) {
    match operand {
        Operand::Copy(place) => {
            rename_mir_place(tcx, place, renamer);
        }
        Operand::Move(place) => {
            rename_mir_place(tcx, place, renamer);
        }
        Operand::Constant(_const) => todo!(),
    }
}
