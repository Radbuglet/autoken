use rustc_hir::def_id::DefId;
use rustc_middle::{
    bug,
    ty::{
        self, fold::RegionFolder, AdtDef, Binder, EarlyBinder, FnSig, GenericArg, GenericArgKind,
        Instance, List, Mutability, ParamConst, ParamEnv, Region, Ty, TyCtxt, TyKind, TypeFoldable,
        TypeFolder, TypeSuperFoldable, TypeVisitableExt,
    },
};
use rustc_span::{ErrorGuaranteed, Symbol};

// === Type Matching === //

pub fn is_generic_ty(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Param(_) | TyKind::Alias(_, _))
}

pub fn is_annotated_ty(def: &AdtDef<'_>, marker: Symbol) -> bool {
    let mut fields = def.all_fields();

    let Some(field) = fields.next() else {
        return false;
    };

    if field.name != marker {
        return false;
    }

    let None = fields.next() else {
        return false;
    };

    true
}

// === Signature Parsing === //

pub fn get_fn_sig_maybe_closure(tcx: TyCtxt<'_>, def_id: DefId) -> EarlyBinder<Binder<FnSig<'_>>> {
    match tcx.type_of(def_id).skip_binder().kind() {
        TyKind::Closure(_, args) => {
            let sig = args.as_closure().sig();

            EarlyBinder::bind(sig.map_bound(|sig| {
                let inputs = sig.inputs();
                assert_eq!(inputs.len(), 1);
                let inputs = &inputs[0].tuple_fields();

                FnSig {
                    inputs_and_output: tcx
                        .mk_type_list_from_iter(inputs.iter().chain([sig.output()])),
                    c_variadic: sig.c_variadic,
                    unsafety: sig.unsafety,
                    abi: sig.abi,
                }
            }))
        }
        _ => tcx.fn_sig(def_id),
    }
}

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

// === Mutability === //

pub trait MutabilityExt {
    fn upgrade(&mut self, other: Mutability);
}

impl MutabilityExt for Mutability {
    fn upgrade(&mut self, other: Mutability) {
        *self = (*self).max(other);
    }
}

// === GenericTransformer === //

pub trait GenericTransformer<'tcx>: Sized + Copy + 'tcx {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>;

    fn instantiate_arg_iter<'a, T>(
        self,
        tcx: TyCtxt<'tcx>,
        iter: impl IntoIterator<Item = T> + 'a,
    ) -> impl Iterator<Item = T> + 'a
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
        'tcx: 'a,
    {
        iter.into_iter()
            .map(move |arg| self.instantiate_arg(tcx, arg))
    }

    fn instantiate_args(
        self,
        tcx: TyCtxt<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> &'tcx List<GenericArg<'tcx>> {
        tcx.mk_args_from_iter(args.iter().map(|arg| self.instantiate_arg(tcx, arg)))
    }

    fn instantiate_func(
        self,
        tcx: TyCtxt<'tcx>,
        func: ConcretizedFunc<'tcx>,
    ) -> ConcretizedFunc<'tcx> {
        ConcretizedFunc(func.def_id(), self.instantiate_args(tcx, func.args()))
    }
}

// === ConcretizedFunc === //

pub type ConcretizedArgs<'tcx> = &'tcx List<GenericArg<'tcx>>;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct ConcretizedFunc<'tcx>(pub DefId, pub ConcretizedArgs<'tcx>);

impl<'tcx> From<Instance<'tcx>> for ConcretizedFunc<'tcx> {
    fn from(instance: Instance<'tcx>) -> Self {
        Self(instance.def_id(), instance.args)
    }
}

impl<'tcx> ConcretizedFunc<'tcx> {
    pub fn def_id(self) -> DefId {
        self.0
    }

    pub fn args(self) -> ConcretizedArgs<'tcx> {
        self.1
    }

    pub fn resolve_instance(
        self,
        tcx: TyCtxt<'tcx>,
    ) -> Result<Option<Instance<'tcx>>, ErrorGuaranteed> {
        tcx.resolve_instance(
            tcx.erase_regions(ParamEnv::reveal_all().and((self.def_id(), self.args()))),
        )
    }
}

impl<'tcx> GenericTransformer<'tcx> for ConcretizedFunc<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        Instance::new(self.0, self.1).instantiate_mir_and_normalize_erasing_regions(
            tcx,
            ParamEnv::reveal_all(),
            EarlyBinder::bind(ty),
        )
    }
}

// === MaybeConcretizedArgs === //

pub type MaybeConcretizedArgs<'tcx> = Option<ConcretizedArgs<'tcx>>;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct MaybeConcretizedFunc<'tcx>(pub DefId, pub MaybeConcretizedArgs<'tcx>);

impl<'tcx> From<ConcretizedFunc<'tcx>> for MaybeConcretizedFunc<'tcx> {
    fn from(func: ConcretizedFunc<'tcx>) -> Self {
        Self(func.def_id(), Some(func.args()))
    }
}

impl<'tcx> MaybeConcretizedFunc<'tcx> {
    pub fn def_id(self) -> DefId {
        self.0
    }

    pub fn args(self) -> MaybeConcretizedArgs<'tcx> {
        self.1
    }

    pub fn as_concretized(self) -> Option<ConcretizedFunc<'tcx>> {
        match self {
            MaybeConcretizedFunc(did, Some(args)) => Some(ConcretizedFunc(did, args)),
            MaybeConcretizedFunc(_, None) => None,
        }
    }
}

impl<'tcx> GenericTransformer<'tcx> for MaybeConcretizedFunc<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        if let Some(concretized) = self.as_concretized() {
            concretized.instantiate_arg(tcx, ty)
        } else {
            ty
        }
    }
}

// === `instantiate_ignoring_regions` === //

pub fn instantiate_ignoring_regions<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    args: &[GenericArg<'tcx>],
) -> Ty<'tcx> {
    ty.fold_with(&mut ArgFolderIgnoreRegions {
        tcx,
        args,
        binders_passed: 0,
    })
}

// Adapted from rustc_middle/src/ty/generic_args.rs
struct ArgFolderIgnoreRegions<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    args: &'a [GenericArg<'tcx>],

    /// Number of region binders we have passed through while doing the instantiation
    binders_passed: u32,
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
    fn type_param_expected(&self, p: ty::ParamTy, ty: Ty<'tcx>, kind: GenericArgKind<'tcx>) -> ! {
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
