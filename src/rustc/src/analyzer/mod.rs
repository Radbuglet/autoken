use rustc_middle::ty::TyCtxt;

use crate::util::{
    feeder::{feeders::MirBuiltStasher, read_feed},
    mir::{has_instance_mir, iter_all_local_def_ids, safeishly_grab_local_def_id_mir},
};

use self::{facts::FunctionFactStore, sets::is_tie_func};

mod facts;
mod mir;
mod sets;
mod sym;

pub fn analyze(tcx: TyCtxt<'_>) {
    // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
    for did in iter_all_local_def_ids(tcx) {
        if safeishly_grab_local_def_id_mir(tcx, did).is_some() {
            assert!(read_feed::<MirBuiltStasher>(tcx, did).is_some());
        }
    }

    // Collect facts for every function
    let mut facts = FunctionFactStore::default();

    for did in iter_all_local_def_ids(tcx) {
        let did = did.to_def_id();

        if !is_tie_func(tcx, did) && has_instance_mir(tcx, did) {
            facts.collect(tcx, did);
        }
    }

    facts.optimize();

    // Validate generic assumptions
    // TODO

    // Create shadow MIR
    // TODO
}
