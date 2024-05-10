use std::collections::hash_map;

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{Instance, Mutability, Ty, TyCtxt, TyKind};
use rustc_span::Symbol;

use crate::util::{hash::FxHashMap, ty::is_annotated_ty};

use super::sym;

pub fn is_tie_func(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    tcx.opt_item_name(def_id) == Some(sym::__autoken_declare_tied.get())
}

pub fn is_absorb_func(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    tcx.opt_item_name(def_id) == Some(sym::__autoken_absorb_only.get())
}

#[derive(Debug, Copy, Clone)]
pub struct ParsedTieCall<'tcx> {
    pub acquired_set: Ty<'tcx>,
    pub tied_to: Option<Symbol>,
}

pub fn parse_tie_func<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
) -> Option<ParsedTieCall<'tcx>> {
    is_tie_func(tcx, instance.def_id()).then(|| {
        // Determine tied reference
        let tied_to = 'tied: {
            let param = instance.args[0].as_type().unwrap();
            if param.is_unit() {
                break 'tied None;
            }

            let first_field = param.ty_adt_def().unwrap().all_fields().next().unwrap();
            let first_field = tcx.type_of(first_field.did).skip_binder();
            let TyKind::Ref(first_field, _pointee, _mut) = first_field.kind() else {
                unreachable!();
            };

            Some(first_field.get_name().unwrap())
        };

        // Determine set type
        let acquired_set = instance.args[1].as_type().unwrap();

        ParsedTieCall {
            tied_to,
            acquired_set,
        }
    })
}

pub fn instantiate_set<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
) -> FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)> {
    let mut set = FxHashMap::<Ty<'tcx>, (Mutability, Option<Symbol>)>::default();

    instantiate_set_proc(tcx, ty, &mut |ty, mutability| match set.entry(ty) {
        hash_map::Entry::Occupied(entry) => {
            if mutability.is_mut() {
                entry.into_mut().0 = Mutability::Mut;
            }
        }
        hash_map::Entry::Vacant(entry) => {
            entry.insert((mutability, None));
        }
    });

    set
}

pub fn instantiate_set_proc<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    add: &mut impl FnMut(Ty<'tcx>, Mutability),
) {
    match ty.kind() {
        // Union
        TyKind::Tuple(fields) => {
            for field in fields.iter() {
                instantiate_set_proc(tcx, field, add);
            }
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_ref_ty_marker.get()) => {
            add(generics[0].as_type().unwrap(), Mutability::Not);
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_mut_ty_marker.get()) => {
            add(generics[0].as_type().unwrap(), Mutability::Mut);
        }
        TyKind::Adt(def, generics)
            if is_annotated_ty(def, sym::__autoken_downgrade_ty_marker.get()) =>
        {
            let mut set = instantiate_set(tcx, generics[0].as_type().unwrap());

            for (mutability, _) in set.values_mut() {
                *mutability = Mutability::Not;
            }

            for (ty, (mutability, _)) in set {
                add(ty, mutability);
            }
        }
        TyKind::Adt(def, generics) if is_annotated_ty(def, sym::__autoken_diff_ty_marker.get()) => {
            let mut set = instantiate_set(tcx, generics[0].as_type().unwrap());

            fn remover_func<'set, 'tcx>(
                set: &'set mut FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)>,
            ) -> impl FnMut(Ty<'tcx>, Mutability) + 'set {
                |ty, mutability| match set.entry(ty) {
                    hash_map::Entry::Occupied(entry) => {
                        if mutability.is_mut() {
                            entry.remove();
                        } else {
                            entry.into_mut().0 = Mutability::Not;
                        }
                    }
                    hash_map::Entry::Vacant(_) => {}
                }
            }

            instantiate_set_proc(
                tcx,
                generics[1].as_type().unwrap(),
                &mut remover_func(&mut set),
            );

            for (ty, (mutability, _)) in set {
                add(ty, mutability);
            }
        }
        _ => unreachable!(),
    }
}
