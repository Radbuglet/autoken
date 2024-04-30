use rustc_hir::def_id::DefId;
use rustc_middle::ty::{
    fold::RegionFolder, AdtDef, Binder, BoundRegion, BoundRegionKind, BoundVar, DebruijnIndex,
    EarlyBinder, FnSig, GenericArg, GenericArgsRef, Instance, InstanceDef, List, Mutability,
    ParamEnv, Region, RegionKind, Ty, TyCtxt, TyKind, TypeFoldable,
};
use rustc_span::{ErrorGuaranteed, Span, Symbol};

use crate::util::hash::FxHashMap;

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

    fn instantiate_args(
        self,
        tcx: TyCtxt<'tcx>,
        param_env: ParamEnv<'tcx>,
        args: &'tcx List<GenericArg<'tcx>>,
    ) -> &'tcx List<GenericArg<'tcx>> {
        tcx.mk_args_from_iter(
            args.iter()
                .map(|arg| self.instantiate_arg(tcx, param_env, arg)),
        )
    }

    fn instantiate_instance(
        self,
        tcx: TyCtxt<'tcx>,
        param_env: ParamEnv<'tcx>,
        func: Instance<'tcx>,
    ) -> Instance<'tcx> {
        Instance {
            def: func.def,
            args: self.instantiate_args(tcx, param_env, func.args),
        }
    }
}

// === Instance === //

impl<'tcx> GenericTransformer<'tcx> for Instance<'tcx> {
    fn instantiate_arg<T>(self, tcx: TyCtxt<'tcx>, param_env: ParamEnv<'tcx>, ty: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        self.instantiate_mir_and_normalize_erasing_regions(tcx, param_env, EarlyBinder::bind(ty))
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

// === BindableRegions === //

#[derive(Debug, Copy, Clone)]
pub struct BindableRegions<'tcx> {
    pub generalized: Ty<'tcx>,
    pub param_count: u32,
}

impl<'tcx> BindableRegions<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>, sig: UnboundFnSig<'tcx>) -> Self {
        // The first binder simply tells us that this function has early bound regions and types.
        let sig = sig.skip_binder();

        // The second binder gives us late-bound regions. We want them to be free so let's skip it
        // too.
        let sig = sig.skip_binder();

        // Let's get the return type since that's what we're interested in.
        let sig = sig.output();

        // Now, we have two types of free regions to handle: `ReBoundEarly` for early-bound regions
        // and `ReParam` for late-bound regions. Each of these can be mapped individually so let's
        // count them.
        #[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
        enum ReParam {
            Early(u32),
            Late(u32),
        }

        let mut param_map = FxHashMap::default();

        let mut map_region = |debrujin: DebruijnIndex, idx: ReParam| -> Region<'tcx> {
            let param_count = BoundVar::from_usize(param_map.len());
            let param = *param_map.entry(idx).or_insert(param_count);

            Region::new_bound(
                tcx,
                debrujin,
                BoundRegion {
                    kind: BoundRegionKind::BrAnon,
                    var: param,
                },
            )
        };

        let _ = sig.fold_with(&mut RegionFolder::new(tcx, &mut |re, debrujin| {
            match &*re {
                RegionKind::ReEarlyParam(re) => map_region(debrujin, ReParam::Early(re.index)),
                RegionKind::ReBound(_, re) => map_region(debrujin, ReParam::Late(re.var.as_u32())),

                // These can just be ignored since they can only take on one value.
                RegionKind::ReStatic => re,

                // These are impossible.
                // TODO: Justify
                RegionKind::ReLateParam(_)
                | RegionKind::ReVar(_)
                | RegionKind::RePlaceholder(_)
                | RegionKind::ReErased => {
                    unreachable!()
                }

                // Just ignore these—the crate will already not compile.
                RegionKind::ReError(_) => re,
            }
        }));

        Self {
            generalized: sig,
            param_count: param_map.len() as u32,
        }
    }

    pub fn get_linked(&self, name: Symbol) -> Option<BoundVar> {
        todo!();
    }
}
