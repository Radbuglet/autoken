use rustc_hir::def_id::DefId;
use rustc_middle::ty::{
    GenericArg, GenericArgs, GenericArgsRef, GenericParamDefKind, Ty, TyCtxt, TypeFoldable,
};

pub fn try_resolve_mono_args_for_func(
    tcx: TyCtxt<'_>,
    def_id: DefId,
) -> Option<GenericArgsRef<'_>> {
    let mut args_wf = true;
    let args =
        // N.B. we use `for_item` instead of `tcx.generics_of` to ensure that we also iterate
        // over the generic arguments of the parent.
        GenericArgs::for_item(tcx, def_id, |param, _| match param.kind {
            // We can handle these
            GenericParamDefKind::Lifetime => tcx.lifetimes.re_erased.into(),
            GenericParamDefKind::Const {
                is_host_effect: true,
                ..
            } => tcx.consts.true_.into(),

            // We can't handle these; return a dummy value and set the `args_wf` flag.
            GenericParamDefKind::Type { .. } => {
                args_wf = false;
                tcx.types.unit.into()
            }
            GenericParamDefKind::Const { .. } => {
                args_wf = false;
                tcx.consts.true_.into()
            }
        });

    args_wf.then_some(args)
}

// === `instantiate_ignoring_regions` === //

pub fn instantiate_ignoring_regions<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    args: &[GenericArg<'tcx>],
) -> Ty<'tcx> {
    ty.fold_with(&mut adapted::ArgFolderIgnoreRegions {
        tcx,
        args,
        binders_passed: 0,
    })
}

mod adapted {
    use rustc_middle::{
        bug,
        ty::{
            self, GenericArg, GenericArgKind, ParamConst, Ty, TyCtxt, TypeFoldable, TypeFolder,
            TypeSuperFoldable, TypeVisitableExt,
        },
    };

    // Adapted from rustc_middle/src/ty/generic_args.rs
    pub struct ArgFolderIgnoreRegions<'a, 'tcx> {
        pub tcx: TyCtxt<'tcx>,
        pub args: &'a [GenericArg<'tcx>],

        /// Number of region binders we have passed through while doing the instantiation
        pub binders_passed: u32,
    }

    impl<'a, 'tcx> TypeFolder<TyCtxt<'tcx>> for ArgFolderIgnoreRegions<'a, 'tcx> {
        #[inline]
        fn interner(&self) -> TyCtxt<'tcx> {
            self.tcx
        }

        fn fold_binder<T: TypeFoldable<TyCtxt<'tcx>>>(
            &mut self,
            t: ty::Binder<'tcx, T>,
        ) -> ty::Binder<'tcx, T> {
            self.binders_passed += 1;
            let t = t.super_fold_with(self);
            self.binders_passed -= 1;
            t
        }

        fn fold_region(&mut self, r: ty::Region<'tcx>) -> ty::Region<'tcx> {
            r
        }

        fn fold_ty(&mut self, t: Ty<'tcx>) -> Ty<'tcx> {
            if !t.has_param() {
                return t;
            }

            match *t.kind() {
                ty::Param(p) => self.ty_for_param(p, t),
                _ => t.super_fold_with(self),
            }
        }

        fn fold_const(&mut self, c: ty::Const<'tcx>) -> ty::Const<'tcx> {
            if let ty::ConstKind::Param(p) = c.kind() {
                self.const_for_param(p, c)
            } else {
                c.super_fold_with(self)
            }
        }
    }

    impl<'a, 'tcx> ArgFolderIgnoreRegions<'a, 'tcx> {
        fn ty_for_param(&self, p: ty::ParamTy, source_ty: Ty<'tcx>) -> Ty<'tcx> {
            // Look up the type in the args. It really should be in there.
            let opt_ty = self.args.get(p.index as usize).map(|k| k.unpack());
            let ty = match opt_ty {
                Some(GenericArgKind::Type(ty)) => ty,
                Some(kind) => self.type_param_expected(p, source_ty, kind),
                None => self.type_param_out_of_range(p, source_ty),
            };

            self.shift_vars_through_binders(ty)
        }

        #[cold]
        #[inline(never)]
        fn type_param_expected(
            &self,
            p: ty::ParamTy,
            ty: Ty<'tcx>,
            kind: GenericArgKind<'tcx>,
        ) -> ! {
            bug!(
                "expected type for `{:?}` ({:?}/{}) but found {:?} when instantiating, args={:?}",
                p,
                ty,
                p.index,
                kind,
                self.args,
            )
        }

        #[cold]
        #[inline(never)]
        fn type_param_out_of_range(&self, p: ty::ParamTy, ty: Ty<'tcx>) -> ! {
            bug!(
                "type parameter `{:?}` ({:?}/{}) out of range when instantiating, args={:?}",
                p,
                ty,
                p.index,
                self.args,
            )
        }

        fn const_for_param(&self, p: ParamConst, source_ct: ty::Const<'tcx>) -> ty::Const<'tcx> {
            // Look up the const in the args. It really should be in there.
            let opt_ct = self.args.get(p.index as usize).map(|k| k.unpack());
            let ct = match opt_ct {
                Some(GenericArgKind::Const(ct)) => ct,
                Some(kind) => self.const_param_expected(p, source_ct, kind),
                None => self.const_param_out_of_range(p, source_ct),
            };

            self.shift_vars_through_binders(ct)
        }

        #[cold]
        #[inline(never)]
        fn const_param_expected(
            &self,
            p: ty::ParamConst,
            ct: ty::Const<'tcx>,
            kind: GenericArgKind<'tcx>,
        ) -> ! {
            bug!(
                "expected const for `{:?}` ({:?}/{}) but found {:?} when instantiating args={:?}",
                p,
                ct,
                p.index,
                kind,
                self.args,
            )
        }

        #[cold]
        #[inline(never)]
        fn const_param_out_of_range(&self, p: ty::ParamConst, ct: ty::Const<'tcx>) -> ! {
            bug!(
                "const parameter `{:?}` ({:?}/{}) out of range when instantiating args={:?}",
                p,
                ct,
                p.index,
                self.args,
            )
        }

        /// It is sometimes necessary to adjust the De Bruijn indices during instantiation. This occurs
        /// when we are instantating a type with escaping bound vars into a context where we have
        /// passed through binders. That's quite a mouthful. Let's see an example:
        ///
        /// ```
        /// type Func<A> = fn(A);
        /// type MetaFunc = for<'a> fn(Func<&'a i32>);
        /// ```
        ///
        /// The type `MetaFunc`, when fully expanded, will be
        /// ```ignore (illustrative)
        /// for<'a> fn(fn(&'a i32))
        /// //      ^~ ^~ ^~~
        /// //      |  |  |
        /// //      |  |  DebruijnIndex of 2
        /// //      Binders
        /// ```
        /// Here the `'a` lifetime is bound in the outer function, but appears as an argument of the
        /// inner one. Therefore, that appearance will have a DebruijnIndex of 2, because we must skip
        /// over the inner binder (remember that we count De Bruijn indices from 1). However, in the
        /// definition of `MetaFunc`, the binder is not visible, so the type `&'a i32` will have a
        /// De Bruijn index of 1. It's only during the instantiation that we can see we must increase the
        /// depth by 1 to account for the binder that we passed through.
        ///
        /// As a second example, consider this twist:
        ///
        /// ```
        /// type FuncTuple<A> = (A,fn(A));
        /// type MetaFuncTuple = for<'a> fn(FuncTuple<&'a i32>);
        /// ```
        ///
        /// Here the final type will be:
        /// ```ignore (illustrative)
        /// for<'a> fn((&'a i32, fn(&'a i32)))
        /// //          ^~~         ^~~
        /// //          |           |
        /// //   DebruijnIndex of 1 |
        /// //               DebruijnIndex of 2
        /// ```
        /// As indicated in the diagram, here the same type `&'a i32` is instantiated once, but in the
        /// first case we do not increase the De Bruijn index and in the second case we do. The reason
        /// is that only in the second case have we passed through a fn binder.
        fn shift_vars_through_binders<T: TypeFoldable<TyCtxt<'tcx>>>(&self, val: T) -> T {
            if self.binders_passed == 0 || !val.has_escaping_bound_vars() {
                return val;
            }

            ty::fold::shift_vars(TypeFolder::interner(self), val, self.binders_passed)
        }
    }
}
