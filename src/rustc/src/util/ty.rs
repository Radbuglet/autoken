use std::hash;

use rustc_hir::def_id::DefId;
use rustc_infer::{infer::TyCtxtInferExt, traits::ObligationCause};
use rustc_middle::ty::{
    fold::RegionFolder, AdtDef, Binder, BoundRegion, BoundRegionKind, BoundVar, BoundVariableKind,
    EarlyBinder, ExistentialPredicate, FnSig, GenericArg, GenericArgKind, GenericArgs,
    GenericArgsRef, GenericParamDefKind, Instance, InstanceDef, List, Mutability, ParamEnv, Region,
    TermKind, Ty, TyCtxt, TyKind, TypeFoldable,
};
use rustc_span::{ErrorGuaranteed, Span, Symbol};
use rustc_trait_selection::traits::ObligationCtxt;

use super::hash::{FxHashMap, FxHashSet};

// === Type Matching === //

pub fn is_generic_ty_param(ty: Ty<'_>) -> bool {
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

pub type UnboundFnSig<'tcx> = EarlyBinder<Binder<'tcx, FnSig<'tcx>>>;

pub fn get_fn_sig_maybe_closure(tcx: TyCtxt<'_>, def_id: DefId) -> UnboundFnSig<'_> {
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

    found_region.ok_or_else(|| extract_free_region_list(tcx, ty, |re| re.get_name()))
}

pub fn extract_free_region_list<'tcx, R>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    mut f: impl FnMut(Region<'tcx>) -> Option<R>,
) -> Vec<R> {
    let mut found = Vec::new();
    let _ = ty.fold_with(&mut RegionFolder::new(tcx, &mut |region, _idx| {
        if let Some(region) = f(region) {
            found.push(region);
        }
        region
    }));
    found
}

pub fn err_failed_to_find_region(tcx: TyCtxt<'_>, span: Span, name: Symbol, symbols: &[Symbol]) {
    tcx.dcx().span_err(
        span,
        format!(
            "lifetime with name {name} not found in output of function{}",
            if symbols.is_empty() {
                String::new()
            } else {
                format!(
                    "; found {}",
                    symbols
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        ),
    );
}

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
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>;

    fn instantiate_arg_iter<'a, T>(
        self,
        tcx: TyCtxt<'tcx>,
        param_env: ParamEnv<'tcx>,
        iter: impl IntoIterator<Item = T> + 'a,
    ) -> impl Iterator<Item = T> + 'a
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
        'tcx: 'a,
    {
        iter.into_iter()
            .map(move |arg| self.instantiate_arg(tcx, param_env, arg))
    }
}

impl<'tcx> GenericTransformer<'tcx> for GenericArgsRef<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        tcx.instantiate_and_normalize_erasing_regions(self, param_env, EarlyBinder::bind(ty))
    }
}

impl<'tcx> GenericTransformer<'tcx> for Instance<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        self.args.instantiate_arg(tcx, param_env, ty)
    }
}

pub fn try_resolve_instance<'tcx>(
    tcx: TyCtxt<'tcx>,
    param_env: ParamEnv<'tcx>,
    instance: Instance<'tcx>,
) -> Result<Option<Instance<'tcx>>, ErrorGuaranteed> {
    Instance::resolve(tcx, param_env, instance.def_id(), instance.args)
}

// === MaybeConcretizedFunc === //

pub type MaybeConcretizedArgs<'tcx> = Option<GenericArgsRef<'tcx>>;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct MaybeConcretizedFunc<'tcx> {
    pub def: InstanceDef<'tcx>,
    pub args: MaybeConcretizedArgs<'tcx>,
}

impl<'tcx> From<Instance<'tcx>> for MaybeConcretizedFunc<'tcx> {
    fn from(func: Instance<'tcx>) -> Self {
        Self {
            def: func.def,
            args: Some(func.args),
        }
    }
}

impl<'tcx> MaybeConcretizedFunc<'tcx> {
    pub fn def_id(self) -> DefId {
        self.def.def_id()
    }

    pub fn as_concretized(self) -> Option<Instance<'tcx>> {
        Some(Instance {
            def: self.def,
            args: self.args?,
        })
    }
}

impl<'tcx> GenericTransformer<'tcx> for MaybeConcretizedArgs<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        if let Some(concretized) = self {
            concretized.instantiate_arg(tcx, param_env, ty)
        } else {
            ty
        }
    }
}

impl<'tcx> GenericTransformer<'tcx> for MaybeConcretizedFunc<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        if let Some(concretized) = self.as_concretized() {
            concretized.instantiate_arg(tcx, param_env, ty)
        } else {
            ty
        }
    }
}

// === Region preserving operations === //

pub fn instantiate_ty_and_normalize_preserving_regions<'tcx, F>(
    tcx: TyCtxt<'tcx>,
    param_env: ParamEnv<'tcx>,
    ty: F,
    args: MaybeConcretizedArgs<'tcx>,
) -> F
where
    F: TypeFoldable<TyCtxt<'tcx>>,
{
    normalize_preserving_regions(
        tcx,
        param_env,
        instantiate_preserving_regions(tcx, ty, args),
    )
}

pub fn instantiate_preserving_regions<'tcx, F>(
    tcx: TyCtxt<'tcx>,
    ty: F,
    args: MaybeConcretizedArgs<'tcx>,
) -> F
where
    F: TypeFoldable<TyCtxt<'tcx>>,
{
    if let Some(args) = args {
        ty.fold_with(
            &mut instantiate_preserving_regions_util::ArgFolderIgnoreRegions {
                tcx,
                args,
                binders_passed: 0,
            },
        )
    } else {
        ty
    }
}

mod instantiate_preserving_regions_util {
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

pub fn normalize_preserving_regions<'tcx, F>(
    tcx: TyCtxt<'tcx>,
    param_env: ParamEnv<'tcx>,
    ty: F,
) -> F
where
    F: TypeFoldable<TyCtxt<'tcx>>,
{
    // In case you're wondering, we have an `infcx` here since normalization has a lot of special
    // superpowers in the type-inference scenario. If we're normalizing types without inference
    // variables, this should just perform normalization logic similar to what `normalize_erasing_regions`
    // gives us.
    //
    // N.B. this doesn't exercise the same code paths as `normalize_erasing_regions`...
    //
    // - `TyCtxt::try_normalize_erasing_regions`:
    //   - `TyCtxt::erase_regions`
    //   - `Ty::try_fold_with::<TryNormalizeAfterErasingRegionsFolder>`
    //     - For each type and constant...
    //       - `TyCtxt::try_normalize_generic_arg_after_erasing_regions`
    //         - `InferCtxt's At::query_normalize()`
    //           - If new solver: `rustc_trait_selection::solve::normalize::deeply_normalize_with_skipped_universes`
    //           - If old solver: `Ty::try_fold_with::<QueryNormalizer>`
    //         - `InferCtxt::resolve_vars_if_possible`
    //         - `TyCtxt::erase_regions`
    //
    //
    // Meanwhile, we do:
    //
    // - `ObligationCtxt::deeply_normalize`
    //   - `NormalizeExt::deeply_normalize`
    //     - If new solver: `rustc_trait_selection::solve::normalize::deeply_normalize_with_skipped_universes`
    //     - If old solver:
    //       TODO: Document
    //
    ObligationCtxt::new(&tcx.infer_ctxt().build())
        .deeply_normalize(&ObligationCause::dummy(), param_env, ty)
        .unwrap()
}

// === BindableRegions === //

#[derive(Debug, Copy, Clone)]
pub struct BindableRegions<'tcx> {
    pub param_env: ParamEnv<'tcx>,
    pub instance: Instance<'tcx>,
    pub generalized: Binder<'tcx, Ty<'tcx>>,
    pub param_count: u32,
}

impl<'tcx> BindableRegions<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, instance: Instance<'tcx>) -> Self {
        // Let's grab the signature for this instance.
        let sig = instantiate_preserving_regions(
            tcx,
            get_fn_sig_maybe_closure(tcx, instance.def_id()).skip_binder(),
            Some(instance.args),
        );

        // The inner binder gives us late-bound regions. We want them to be free so let's skip it.
        let sig = sig.skip_binder();

        // Let's get the return type since that's what we're interested in.
        let sig = sig.output();

        // Now, we have two types of free regions to handle: `ReBoundEarly` for early-bound regions
        // and `ReParam` for late-bound regions. Each of these can be mapped individually so let's
        // count them.
        let mut param_count = 0;

        let sig = sig.fold_with(&mut RegionFolder::new(tcx, &mut |_, debrujin| {
            let re = Region::new_bound(
                tcx,
                debrujin,
                BoundRegion {
                    var: BoundVar::from_u32(param_count),
                    kind: BoundRegionKind::BrAnon,
                },
            );
            param_count += 1;
            re
        }));

        let sig = Binder::bind_with_vars(
            sig,
            tcx.mk_bound_variable_kinds_from_iter(
                (0..param_count).map(|_| BoundVariableKind::Region(BoundRegionKind::BrAnon)),
            ),
        );

        Self {
            param_env,
            instance,
            generalized: sig,
            param_count,
        }
    }

    pub fn get_linked(
        &self,
        tcx: TyCtxt<'tcx>,
        args: MaybeConcretizedArgs<'tcx>,
        name: Symbol,
    ) -> Option<FxHashSet<BoundVar>> {
        // Instantiate our generic signature with the instance's information.
        let trait_sig = instantiate_ty_and_normalize_preserving_regions(
            tcx,
            self.param_env,
            self.generalized,
            args,
        );

        // Determine the concrete function we're calling.
        let concrete = try_resolve_instance(
            tcx,
            self.param_env,
            args.instantiate_arg(tcx, self.param_env, self.instance),
        )
        .unwrap()
        .unwrap();

        // Now, get our impl's concrete signature and instantiate it too.
        let concrete_sig = get_fn_sig_maybe_closure(tcx, concrete.def_id())
            .skip_binder()
            .output();

        let concrete_sig = instantiate_ty_and_normalize_preserving_regions(
            tcx,
            self.param_env,
            concrete_sig,
            Some(concrete.args),
        );

        // Figure out the types' correspondences.
        todo!();
    }
}

// === FunctionMap === //

pub struct FunctionMap<K, V> {
    pub map: FxHashMap<K, Option<V>>,
}

impl<K, V> Default for FunctionMap<K, V> {
    fn default() -> Self {
        Self {
            map: FxHashMap::default(),
        }
    }
}

impl<K: hash::Hash + Eq, V: Eq> FunctionMap<K, V> {
    pub fn insert(&mut self, domain: K, value: V) -> bool {
        self.map
            .entry(domain)
            .and_modify(|v| {
                if v.as_ref() != Some(&value) {
                    *v = None;
                }
            })
            .or_insert(Some(value))
            .is_none()
    }

    pub fn invalidate(&mut self, domain: K) {
        self.map.insert(domain, None);
    }
}

// === `par_traverse_regions` === //

pub fn par_traverse_regions<'tcx>(
    left: Ty<'tcx>,
    right: Ty<'tcx>,
    f: impl FnMut(Region<'tcx>, Region<'tcx>),
) {
    ParRegionTraversal::new(f).traverse_types(left, right);
}

pub struct ParRegionTraversal<F> {
    handler: F,
    passed_binders: u32,
}

impl<'tcx, F> ParRegionTraversal<F>
where
    F: FnMut(Region<'tcx>, Region<'tcx>),
{
    pub fn new(handler: F) -> Self {
        Self {
            handler,
            passed_binders: 0,
        }
    }

    fn traverse(&mut self, left: GenericArgKind<'tcx>, right: GenericArgKind<'tcx>) {
        match (left, right) {
            (GenericArgKind::Lifetime(left), GenericArgKind::Lifetime(right)) => {
                self.traverse_lifetimes(left, right);
            }
            (GenericArgKind::Type(left), GenericArgKind::Type(right)) => {
                self.traverse_types(left, right);
            }

            // Ignored: constants don't affect linked regions
            (GenericArgKind::Const(_), GenericArgKind::Const(_)) => {}
            _ => unreachable!(),
        }
    }

    fn traverse_lists(
        &mut self,
        left: impl IntoIterator<Item = GenericArgKind<'tcx>>,
        right: impl IntoIterator<Item = GenericArgKind<'tcx>>,
    ) {
        for (left, right) in left.into_iter().zip(right.into_iter()) {
            self.traverse(left, right);
        }
    }

    fn traverse_types(&mut self, left: Ty<'tcx>, right: Ty<'tcx>) {
        match (*left.kind(), *right.kind()) {
            (TyKind::Adt(_, left), TyKind::Adt(_, right)) => {
                self.traverse_generics(left, right);
            }
            (TyKind::Array(left, _), TyKind::Array(right, _)) => {
                self.traverse_types(left, right);
            }
            (TyKind::Slice(left), TyKind::Slice(right)) => {
                self.traverse_types(left, right);
            }
            (TyKind::RawPtr(left), TyKind::RawPtr(right)) => {
                self.traverse_types(left.ty, right.ty);
            }
            (TyKind::Ref(left_re, left_ty, _), TyKind::Ref(right_re, right_ty, _)) => {
                self.traverse_lifetimes(left_re, right_re);
                self.traverse_types(left_ty, right_ty);
            }
            (TyKind::FnDef(_, left), TyKind::FnDef(_, right)) => {
                self.traverse_generics(left, right);
            }
            (TyKind::FnPtr(left), TyKind::FnPtr(right)) => {
                self.passed_binders += 1;
                self.traverse_type_lists(
                    left.skip_binder().inputs_and_output,
                    right.skip_binder().inputs_and_output,
                );
                self.passed_binders -= 1;
            }
            (TyKind::Dynamic(left_ty, left_re, _), TyKind::Dynamic(right_ty, right_re, _)) => {
                self.traverse_lifetimes(left_re, right_re);
                self.traverse_predicate_lists(left_ty, right_ty);
            }
            (TyKind::Closure(_, left), TyKind::Closure(_, right)) => {
                self.traverse_generics(left, right);
            }
            (TyKind::Tuple(left), TyKind::Tuple(right)) => {
                self.traverse_type_lists(left, right);
            }

            // Unsupported.
            (TyKind::CoroutineClosure(..), TyKind::CoroutineClosure(..)) => todo!(),
            (TyKind::Coroutine(..), TyKind::Coroutine(..)) => todo!(),
            (TyKind::CoroutineWitness(..), TyKind::CoroutineWitness(..)) => todo!(),

            // All these types are dead ends.
            (TyKind::Bool, TyKind::Bool) => {}
            (TyKind::Char, TyKind::Char) => {}
            (TyKind::Int(..), TyKind::Int(..)) => {}
            (TyKind::Uint(..), TyKind::Uint(..)) => {}
            (TyKind::Float(..), TyKind::Float(..)) => {}
            (TyKind::Foreign(..), TyKind::Foreign(..)) => {}
            (TyKind::Str, TyKind::Str) => {}
            (TyKind::Never, TyKind::Never) => {}

            // We just ignore errors since this crate is already rejected and
            // we have to do something sensible.
            (TyKind::Error(..), TyKind::Error(..)) => {}

            // Non-applicable.
            (TyKind::Infer(..), TyKind::Infer(..)) => unreachable!(),
            (TyKind::Placeholder(..), TyKind::Placeholder(..)) => unreachable!(),
            (TyKind::Param(..), TyKind::Param(..)) => unreachable!(),
            (TyKind::Alias(..), TyKind::Alias(..)) => unreachable!(),
            (TyKind::Bound(..), TyKind::Bound(..)) => unreachable!(),
            _ => unreachable!(),
        }
    }

    fn traverse_lifetimes(&mut self, left: Region<'tcx>, right: Region<'tcx>) {
        (self.handler)(left, right);
    }

    fn traverse_generics(&mut self, left: GenericArgsRef<'tcx>, right: GenericArgsRef<'tcx>) {
        self.traverse_lists(
            left.iter().map(GenericArg::unpack),
            right.iter().map(GenericArg::unpack),
        );
    }

    fn traverse_predicate_lists(
        &mut self,
        left: &'tcx List<Binder<ExistentialPredicate<'tcx>>>,
        right: &'tcx List<Binder<ExistentialPredicate<'tcx>>>,
    ) {
        self.passed_binders += 1;
        for (left, right) in left.iter().zip(right.iter()) {
            let left = left.skip_binder();
            let right = right.skip_binder();

            match (left, right) {
                (ExistentialPredicate::Trait(left), ExistentialPredicate::Trait(right)) => {
                    self.traverse_generics(left.args, right.args);
                }
                (
                    ExistentialPredicate::Projection(left),
                    ExistentialPredicate::Projection(right),
                ) => {
                    self.traverse_generics(left.args, right.args);

                    match (left.term.unpack(), right.term.unpack()) {
                        (TermKind::Ty(left), TermKind::Ty(right)) => {
                            self.traverse_types(left, right);
                        }
                        (TermKind::Const(_), TermKind::Const(_)) => {}
                        _ => unreachable!(),
                    }
                }
                (ExistentialPredicate::AutoTrait(_), ExistentialPredicate::AutoTrait(_)) => {}
                _ => unreachable!(),
            }
        }

        self.passed_binders -= 1;
    }

    fn traverse_type_lists(&mut self, left: &'tcx List<Ty<'tcx>>, right: &'tcx List<Ty<'tcx>>) {
        self.traverse_lists(
            left.iter().map(GenericArgKind::Type),
            right.iter().map(GenericArgKind::Type),
        )
    }
}
