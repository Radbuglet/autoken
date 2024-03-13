use std::collections::{hash_map, HashMap};

use rustc_hir::{
    def::DefKind,
    def_id::{DefIndex, LocalDefId},
};

use rustc_middle::{
    mir::{Mutability, Terminator, TerminatorKind},
    ty::{EarlyBinder, Instance, ParamEnv, Ty, TyCtxt, TyKind},
};
use rustc_span::Symbol;

use crate::{
    analyzer::sym::unnamed,
    util::{
        feeder::{feed, feeders::MirBuiltFeeder},
        hash::FxHashMap,
        mir::{safeishly_grab_instance_mir, MirGrabResult},
    },
};

// === Engine === //

#[derive(Debug, Default)]
pub struct AnalysisDriver<'tcx> {
    func_facts: FxHashMap<Instance<'tcx>, Option<FuncFacts<'tcx>>>,
    id_gen: u64,
}

#[derive(Debug)]
struct FuncFacts<'tcx> {
    borrows: FxHashMap<Ty<'tcx>, Mutability>,
}

impl<'tcx> AnalysisDriver<'tcx> {
    pub fn analyze(&mut self, tcx: TyCtxt<'tcx>) {
        // Get the token use sets of each function.
        assert!(!tcx.untracked().definitions.is_frozen());

        // N.B. we use this instead of `iter_local_def_id` to avoid freezing the definition map.
        let id_count = tcx.untracked().definitions.read().def_index_count();
        for i in 0..id_count {
            let local_def = LocalDefId {
                local_def_index: DefIndex::from_usize(i),
            };

            if !matches!(tcx.def_kind(local_def), DefKind::Fn | DefKind::AssocFn) {
                continue;
            }

            if tcx.generics_of(local_def).count() > 0 {
                continue;
            }

            self.analyze_fn_facts(tcx, Instance::mono(tcx, local_def.to_def_id()));
        }

        // Check for undeclared unsizing.
        // TODO

        // Generate shadow functions for each locally-visited function.
        assert!(!tcx.untracked().definitions.is_frozen());

        let mut shadows = Vec::new();

        for (instance, facts) in &mut self.func_facts {
            let facts = facts.as_mut().unwrap();
            let Some(orig_id) = instance.def_id().as_local() else {
                continue;
            };

            // Modify body
            let body = tcx.mir_built(orig_id);
            dbg!(orig_id);
            let mut body = body.borrow().clone();
            // TODO

            // Feed the query system the shadow function's properties.
            let shadow_kind = tcx.def_kind(orig_id);
            let shadow_def = tcx.at(body.span).create_def(
                tcx.local_parent(orig_id),
                Symbol::intern(&format!(
                    "{}_autoken_shadow_{}",
                    tcx.opt_item_name(orig_id.to_def_id())
                        .unwrap_or_else(|| unnamed.get()),
                    self.id_gen,
                )),
                shadow_kind,
            );
            self.id_gen += 1;

            feed::<MirBuiltFeeder>(tcx, shadow_def.def_id(), tcx.alloc_steal_mir(body));
            shadow_def.opt_local_def_id_to_hir_id(Some(tcx.local_def_id_to_hir_id(orig_id)));
            shadow_def.visibility(tcx.visibility(orig_id));

            if shadow_kind == DefKind::AssocFn {
                shadow_def.associated_item(tcx.associated_item(orig_id));
            }

            // ...and queue it up for borrow checking!
            shadows.push(shadow_def);
        }

        // Finally, borrow check everything in a single go to avoid issues with stolen values.
        for shadow in shadows {
            let _ = tcx.mir_borrowck(shadow.def_id());
        }
    }

    fn analyze_fn_facts(&mut self, tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) {
        // Ensure that we don't analyze the same function circularly or redundantly.
        let hash_map::Entry::Vacant(entry) = self.func_facts.entry(instance) else {
            return;
        };

        // If this function has a hardcoded fact set, use those.
        let hardcoded_mut = match tcx.opt_item_name(instance.def_id()) {
            v if v == Some(sym::__autoken_declare_tied_ref.get()) => Some(Mutability::Not),
            v if v == Some(sym::__autoken_declare_tied_mut.get()) => Some(Mutability::Mut),
            _ => None,
        };

        if let Some(hardcoded_mut) = hardcoded_mut {
            entry.insert(Some(FuncFacts {
                borrows: HashMap::from_iter([(instance.args[0].as_type().unwrap(), hardcoded_mut)]),
            }));
            return;
        }

        // Acquire the function body.
        let body_steal;
        let body = match safeishly_grab_instance_mir(tcx, instance.def) {
            MirGrabResult::FoundSteal(body) => {
                body_steal = body.borrow();
                &*body_steal
            }
            MirGrabResult::FoundRef(body) => body,
            _ => return,
        };

        // This is a real function so let's add it to the fact map.
        entry.insert(None);

        // See who the function may call and where.
        let mut borrows = FxHashMap::default();

        for bb in body.basic_blocks.iter() {
            // If the terminator is a call terminator.
            let Some(Terminator {
                kind: TerminatorKind::Call { func, .. },
                ..
            }) = &bb.terminator
            else {
                continue;
            };

            // Concretize the function type.
            let func = func.ty(&body.local_decls, tcx);
            let func = instance.instantiate_mir_and_normalize_erasing_regions(
                tcx,
                ParamEnv::reveal_all(),
                EarlyBinder::bind(func),
            );

            // If the function target is well known...
            let TyKind::FnDef(callee_id, generics) = func.kind() else {
                continue;
            };

            // Analyze it...
            let target_instance =
                Instance::expect_resolve(tcx, ParamEnv::reveal_all(), *callee_id, generics);
            self.analyze_fn_facts(tcx, target_instance);

            // ...and add its borrows to the borrows set.
            let Some(Some(target_facts)) = &self.func_facts.get(&target_instance) else {
                continue;
            };

            for (borrow_key, borrow_mut) in &target_facts.borrows {
                let curr_mut = borrows.entry(*borrow_key).or_insert(*borrow_mut);
                if borrow_mut.is_mut() {
                    *curr_mut = Mutability::Mut;
                }
            }
        }

        self.func_facts
            .insert(instance, Some(FuncFacts { borrows }));
    }
}

#[allow(non_upper_case_globals)]
mod sym {
    use crate::util::mir::CachedSymbol;

    pub static __autoken_declare_tied_ref: CachedSymbol =
        CachedSymbol::new("__autoken_declare_tied_ref");

    pub static __autoken_declare_tied_mut: CachedSymbol =
        CachedSymbol::new("__autoken_declare_tied_mut");

    pub static unnamed: CachedSymbol = CachedSymbol::new("unnamed");
}
