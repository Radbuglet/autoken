use std::sync::OnceLock;

use rustc_data_structures::steal::Steal;
use rustc_hash::FxHashMap;
use rustc_hir::{
    def::DefKind,
    def_id::{DefId, DefIndex, LocalDefId},
    ImplItemKind, ItemKind, LangItem, Node, TraitFn, TraitItemKind,
};
use rustc_middle::{
    mir::{Body, CastKind, LocalDecls, Rvalue, StatementKind, Terminator, TerminatorKind},
    ty::{
        adjustment::PointerCoercion,
        fold::{FnMutDelegate, RegionFolder},
        EarlyBinder, GenericArg, Instance, List, ParamEnv, Region, Ty, TyCtxt, TyKind, TypeAndMut,
        TypeFoldable, VtblEntry,
    },
};
use rustc_span::{ErrorGuaranteed, Span, Symbol};
use rustc_trait_selection::traits::supertraits;

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

// === `iter_all_local_def_ids` === //

// N.B. we use this instead of `iter_local_def_id` to avoid freezing the definition map.
pub fn iter_all_local_def_ids(tcx: TyCtxt<'_>) -> impl Iterator<Item = LocalDefId> {
    let idx_count = tcx.untracked().definitions.read().def_index_count();

    (0..idx_count).map(|i| LocalDefId {
        local_def_index: DefIndex::from_usize(i),
    })
}

// === `find_region_with_name` === //

pub fn find_region_with_name<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    name: Symbol,
) -> Result<Region<'tcx>, Vec<Symbol>> {
    let mut found_region = None;

    let _ = ty.fold_with(&mut RegionFolder::new(tcx, &mut |region, _idx| {
        if found_region.is_none() && region.get_name() == Some(name) {
            found_region = Some(region);
        }
        region
    }));

    found_region.ok_or_else(|| {
        let mut found = Vec::new();
        let _ = ty.fold_with(&mut RegionFolder::new(tcx, &mut |region, _idx| {
            if let Some(name) = region.get_name() {
                found.push(name);
            }
            region
        }));
        found
    })
}

// === `safeishly_grab_def_id_mir` === //

pub fn safeishly_grab_local_def_id_mir(
    tcx: TyCtxt<'_>,
    id: LocalDefId,
) -> Option<&Steal<Body<'_>>> {
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

// === `does_have_instance_mir` === //

pub fn does_have_instance_mir(tcx: TyCtxt<'_>, did: DefId) -> bool {
    let is_func_kind = matches!(
        tcx.def_kind(did),
        DefKind::Fn | DefKind::AssocFn | DefKind::Closure
    );

    is_func_kind && !tcx.is_foreign_item(did) && tcx.is_mir_available(did)
}

// === get_static_callee_from_terminator === //

#[derive(Debug, Copy, Clone)]
pub enum TerminalCallKind<'tcx> {
    Static(Span, DefId, &'tcx List<GenericArg<'tcx>>),
    Generic(Span, DefId, &'tcx List<GenericArg<'tcx>>),
    Dynamic,
}

pub fn get_static_callee_from_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    terminator: &Option<Terminator<'tcx>>,
    local_decls: &LocalDecls<'tcx>,
) -> Option<TerminalCallKind<'tcx>> {
    match &terminator.as_ref()?.kind {
        TerminatorKind::Call {
            func: dest_func,
            fn_span,
            ..
        } => {
            // Get the type of the function local we're calling.
            let dest_func = dest_func.ty(local_decls, tcx);

            // Attempt to fetch a `DefId` and arguments for the callee.
            let (dest_did, dest_args) = match dest_func.kind() {
                TyKind::FnPtr(_) => {
                    return Some(TerminalCallKind::Dynamic);
                }
                TyKind::FnDef(did, args) => (*did, *args),
                TyKind::Closure(did, args) => (*did, args.as_closure().args),
                _ => unreachable!(),
            };

            let Ok(dest_args) =
                tcx.try_normalize_erasing_regions(ParamEnv::reveal_all(), dest_args)
            else {
                // TODO: What does it mean when this fails?
                return None;
            };

            match resolve_instance(tcx, dest_did, dest_args) {
                Ok(Some(dest_instance)) => Some(TerminalCallKind::Static(
                    *fn_span,
                    dest_instance.def_id(),
                    dest_instance.args,
                )),

                // `Ok(None)` when the `GenericArgsRef` are still too generic
                Ok(None) => Some(TerminalCallKind::Generic(*fn_span, dest_did, dest_args)),

                // the `Instance` resolution process couldn't complete due to errors elsewhere
                Err(_) => None,
            }
        }
        TerminatorKind::Drop {
            place: dest_obj, ..
        } => {
            // TODO: Reinstate
            //             let dest_ty = dest_obj.ty(local_decls, tcx);
            //             let def_id = tcx.require_lang_item(LangItem::DropInPlace, None);
            //             let args = tcx.mk_args(&[dest_ty.ty.into()]);
            //
            //             Some(TerminalCallKind::Static(def_id, args))
            None
        }
        _ => None,
    }
}

pub fn resolve_instance<'tcx>(
    tcx: TyCtxt<'tcx>,
    did: DefId,
    args: &'tcx List<GenericArg<'tcx>>,
) -> Result<Option<Instance<'tcx>>, ErrorGuaranteed> {
    tcx.resolve_instance(tcx.erase_regions(ParamEnv::reveal_all().and((did, args))))
}

// === Unsizing Analysis === //

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

pub fn for_each_unsized_func<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    body: &Body<'tcx>,
    mut f: impl FnMut(Instance<'tcx>),
) {
    for bb in body.basic_blocks.iter() {
        for stmt in bb.statements.iter() {
            let StatementKind::Assign(stmt) = &stmt.kind else {
                continue;
            };
            let (_place, rvalue) = &**stmt;

            let Rvalue::Cast(CastKind::PointerCoercion(kind), from_op, to_ty) = rvalue else {
                continue;
            };

            let from_ty = instance.instantiate_mir_and_normalize_erasing_regions(
                tcx,
                ParamEnv::reveal_all(),
                EarlyBinder::bind(from_op.ty(&body.local_decls, tcx)),
            );

            let to_ty = instance.instantiate_mir_and_normalize_erasing_regions(
                tcx,
                ParamEnv::reveal_all(),
                EarlyBinder::bind(*to_ty),
            );

            match kind {
                PointerCoercion::ReifyFnPointer => {
                    let TyKind::FnDef(def, generics) = from_ty.kind() else {
                        unreachable!()
                    };

                    f(Instance::expect_resolve(
                        tcx,
                        ParamEnv::reveal_all(),
                        *def,
                        generics,
                    ));
                }
                PointerCoercion::ClosureFnPointer(_) => {
                    let TyKind::Closure(def, generics) = from_ty.kind() else {
                        unreachable!()
                    };

                    f(Instance::expect_resolve(
                        tcx,
                        ParamEnv::reveal_all(),
                        *def,
                        generics,
                    ));
                }
                PointerCoercion::Unsize => {
                    // Finds the type the coercion actually changed.
                    let (from_ty, to_ty) = get_unsized_ty(tcx, from_ty, to_ty);

                    // Ensures that we're analyzing a dynamic type unsizing coercion.
                    let TyKind::Dynamic(binders, ..) = to_ty.kind() else {
                        continue;
                    };

                    // Extract the principal non-auto-type from the dynamic type.
                    let Some(binder) = binders.principal() else {
                        continue;
                    };

                    // Do some magic with binders... I guess.
                    let base_binder = tcx.erase_regions(binder.with_self_ty(tcx, to_ty));

                    let mut super_trait_def_id_to_trait_ref = FxHashMap::default();

                    for binder in supertraits(tcx, base_binder) {
                        let trait_ref = tcx.replace_bound_vars_uncached(
                            binder,
                            FnMutDelegate {
                                regions: &mut |_re| tcx.lifetimes.re_erased,
                                types: &mut |_| unreachable!(),
                                consts: &mut |_, _| unreachable!(),
                            },
                        );

                        super_trait_def_id_to_trait_ref.insert(trait_ref.def_id, trait_ref);
                    }

                    // Get the actual methods which make up the trait's vtable since those are
                    // the things we can actually call.
                    let vtable_entries = tcx.vtable_entries(base_binder);

                    for vtable_entry in vtable_entries {
                        let VtblEntry::Method(vtbl_method) = vtable_entry else {
                            continue;
                        };

                        let method_trait = tcx.trait_of_item(vtbl_method.def_id()).unwrap();
                        let method_trait = &super_trait_def_id_to_trait_ref[&method_trait];

                        f(Instance::expect_resolve(
                            tcx,
                            ParamEnv::reveal_all(),
                            vtbl_method.def_id(),
                            tcx.mk_args(
                                [GenericArg::from(from_ty)]
                                    .into_iter()
                                    .chain(method_trait.args.iter().skip(1))
                                    .collect::<Vec<_>>()
                                    .as_slice(),
                            ),
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}
