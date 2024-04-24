use std::sync::OnceLock;

use rustc_data_structures::steal::Steal;
use rustc_hash::FxHashMap;
use rustc_hir::{
    def_id::{DefIndex, LocalDefId},
    ImplItemKind, ItemKind, Node, TraitFn, TraitItemKind,
};
use rustc_middle::{
    mir::{Body, CastKind, LocalDecls, Rvalue, StatementKind, Terminator, TerminatorKind},
    ty::{
        adjustment::PointerCoercion, fold::FnMutDelegate, GenericArg, Instance, InstanceDef,
        ParamEnv, Ty, TyCtxt, TyKind, TypeAndMut, VtblEntry,
    },
};
use rustc_span::{Span, Symbol};
use rustc_trait_selection::traits::supertraits;

use super::ty::{try_resolve_instance, GenericTransformer, MaybeConcretizedFunc};

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

// === `try_grab_base_mir_of_def_id` === //

pub fn try_grab_base_mir_of_def_id(tcx: TyCtxt<'_>, id: LocalDefId) -> Option<&Steal<Body<'_>>> {
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

// === `try_grab_optimized_mir_of_instance` === //

#[derive(Debug, Copy, Clone)]
pub enum MirGrabResult<'tcx> {
    Found(&'tcx Body<'tcx>),
    Dynamic,
    BottomsOut,
}

impl<'tcx> MirGrabResult<'tcx> {
    pub fn is_found(self) -> bool {
        matches!(self, Self::Found(_))
    }

    pub fn unwrap(self) -> &'tcx Body<'tcx> {
        match self {
            MirGrabResult::Found(body) => body,
            _ => unreachable!(),
        }
    }
}

pub fn try_grab_optimized_mir_of_instance<'tcx>(
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

// === `get_static_callee_from_terminator` === //

#[derive(Debug, Copy, Clone)]
pub enum TerminalCallKind<'tcx> {
    Static(Span, Instance<'tcx>),
    Generic(Span, Instance<'tcx>),
    Dynamic,
}

pub fn get_callee_from_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: MaybeConcretizedFunc<'tcx>,
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
            let dest_func = instance.instantiate_arg(tcx, dest_func);

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

            let dest = Instance::new(dest_did, dest_args);

            match try_resolve_instance(tcx, dest) {
                Ok(Some(dest)) => Some(TerminalCallKind::Static(*fn_span, dest)),

                // `Ok(None)` when the `GenericArgsRef` are still too generic
                Ok(None) => Some(TerminalCallKind::Generic(*fn_span, dest)),

                // the `Instance` resolution process couldn't complete due to errors elsewhere
                Err(_) => None,
            }
        }
        _ => None,
    }
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

pub fn for_each_concrete_unsized_func<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: MaybeConcretizedFunc<'tcx>,
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

            let from_ty = instance.instantiate_arg(tcx, from_op.ty(&body.local_decls, tcx));
            let to_ty = instance.instantiate_arg(tcx, *to_ty);

            match kind {
                PointerCoercion::ReifyFnPointer => {
                    let TyKind::FnDef(def, generics) = from_ty.kind() else {
                        unreachable!()
                    };

                    if let Ok(Some(func)) = try_resolve_instance(tcx, Instance::new(*def, generics))
                    {
                        f(func);
                    }
                }
                PointerCoercion::ClosureFnPointer(_) => {
                    let TyKind::Closure(def, generics) = from_ty.kind() else {
                        unreachable!()
                    };

                    if let Ok(Some(func)) = try_resolve_instance(tcx, Instance::new(*def, generics))
                    {
                        f(func);
                    }
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

                        if let Ok(Some(func)) = try_resolve_instance(
                            tcx,
                            Instance {
                                def: vtbl_method.def,
                                args: tcx.mk_args(
                                    [GenericArg::from(from_ty)]
                                        .into_iter()
                                        .chain(method_trait.args.iter().skip(1))
                                        .collect::<Vec<_>>()
                                        .as_slice(),
                                ),
                            },
                        ) {
                            f(func);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
