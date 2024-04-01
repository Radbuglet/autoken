use std::collections::hash_map;

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{Mutability, Ty, TyCtxt, TyKind};
use rustc_span::{Span, Symbol};

use crate::util::{
    hash::FxHashMap,
    ty::{is_annotated_ty, is_generic_ty},
};

use super::sym;

pub fn is_tie_func(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    tcx.opt_item_name(def_id) == Some(sym::__autoken_declare_tied.get())
}

pub fn instantiate_set<'tcx>(
    tcx: TyCtxt<'tcx>,
    span: Span,
    ty: Ty<'tcx>,
    add_generic_union_set: Option<&mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)>,
) -> FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)> {
    let mut set = FxHashMap::<Ty<'tcx>, (Mutability, Option<Symbol>)>::default();

    instantiate_set_proc(
        tcx,
        span,
        ty,
        &mut |ty, mutability| match set.entry(ty) {
            hash_map::Entry::Occupied(entry) => {
                if mutability.is_mut() {
                    entry.into_mut().0 = Mutability::Mut;
                }
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert((Mutability::Mut, None));
            }
        },
        add_generic_union_set,
    );

    set
}

pub fn instantiate_set_proc<'tcx>(
    tcx: TyCtxt<'tcx>,
    span: Span,
    ty: Ty<'tcx>,
    add_ty: &mut dyn FnMut(Ty<'tcx>, Mutability),
    mut add_generic_union_set: Option<&mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)>,
) {
    match ty.kind() {
        // Union
        TyKind::Tuple(fields) => {
            for field in fields.iter() {
                instantiate_set_proc(
                    tcx,
                    span,
                    field,
                    add_ty,
                    add_generic_union_set.as_deref_mut(),
                );
            }
        }

        // Ref
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_ref_ty_marker.get()) => {
            add_ty(generics[0].as_type().unwrap(), Mutability::Not);
        }

        // Mut
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_mut_ty_marker.get()) => {
            add_ty(generics[0].as_type().unwrap(), Mutability::Mut);
        }

        // Downgrade
        TyKind::Adt(def, generics)
            if is_annotated_ty(def, sym::__autoken_downgrade_ty_marker.get()) =>
        {
            let mut set = instantiate_set(
                tcx,
                span,
                generics[0].as_type().unwrap(),
                add_generic_union_set
                    .as_deref_mut()
                    .map(|add_generic_union_set| {
                        |set: Ty<'tcx>, _mut: Mutability| {
                            add_generic_union_set(set, Mutability::Not)
                        }
                    })
                    .as_mut()
                    .map(|v| v as &mut (dyn FnMut(Ty<'tcx>, Mutability) + '_)),
            );

            for (mutability, _) in set.values_mut() {
                *mutability = Mutability::Not;
            }

            for (ty, (mutability, _)) in set {
                add_ty(ty, mutability);
            }
        }

        // Difference
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_diff_ty_marker.get()) => {
            let mut set = instantiate_set(tcx, span, generics[0].as_type().unwrap(), None);

            instantiate_set_proc(
                tcx,
                span,
                generics[1].as_type().unwrap(),
                &mut |ty, mutability| match set.entry(ty) {
                    hash_map::Entry::Occupied(entry) => {
                        if mutability.is_mut() {
                            entry.remove();
                        } else {
                            entry.into_mut().0 = Mutability::Not;
                        }
                    }
                    hash_map::Entry::Vacant(_) => {}
                },
                None,
            );

            for (ty, (mutability, _)) in set {
                add_ty(ty, mutability);
            }
        }

        // Generics
        _ if is_generic_ty(ty) => {
            if let Some(add_generic_union_set) = &mut add_generic_union_set {
                add_generic_union_set(ty, Mutability::Mut);
            } else {
                tcx.dcx()
                    .span_err(span, "generic sets can only appear in top-level unions");
            }
        }

        _ => unreachable!(),
    }
}
