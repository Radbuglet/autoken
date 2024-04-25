use rustc_hir::def_id::DefId;
use rustc_middle::ty::{GenericArgs, GenericArgsRef, GenericParamDefKind, TyCtxt};

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
