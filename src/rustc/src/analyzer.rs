use std::collections::{hash_map, HashMap};

use rustc_data_structures::{steal::Steal, sync::RwLock};
use rustc_hir::{
    def::DefKind,
    def_id::{DefId, DefIndex, LocalDefId},
    HirId, ItemLocalId, OwnerId,
};

use rustc_middle::{
    mir::{
        interpret::Scalar, BasicBlock, Body, BorrowKind, CastKind, Const, ConstOperand, ConstValue,
        LocalDecl, MutBorrowKind, Mutability, Operand, Place, ProjectionElem, Rvalue, SourceInfo,
        SourceScope, Statement, StatementKind, UserTypeProjection,
    },
    ty::{
        BoundRegionKind, BoundVariableKind, CanonicalUserType, CanonicalUserTypeAnnotation,
        Instance, List, ParamEnv, Region, Ty, TyCtxt, TypeAndMut, UniverseIndex, UserType, ValTree,
        Variance,
    },
};
use rustc_span::{Symbol, DUMMY_SP};

use crate::{
    analyzer::sym::unnamed,
    util::{
        feeder::{feed, feeders::MirBuiltFeeder},
        hash::FxHashMap,
        mir::{
            get_static_callee_from_terminator, safeishly_grab_def_id_mir,
            safeishly_grab_instance_mir, MirGrabResult,
        },
    },
};

// === Engine === //

#[derive(Debug, Default)]
pub struct AnalysisDriver<'tcx> {
    func_facts: FxHashMap<Instance<'tcx>, Option<FuncFacts<'tcx>>>,
    local_mirs: FxHashMap<LocalDefId, Body<'tcx>>,
    id_gen: u64,
}

#[derive(Debug)]
struct FuncFacts<'tcx> {
    borrows: FxHashMap<Ty<'tcx>, (Mutability, Option<u32>)>,
}

impl<'tcx> AnalysisDriver<'tcx> {
    pub fn analyze(&mut self, tcx: TyCtxt<'tcx>) {
        let id_count = tcx.untracked().definitions.read().def_index_count();

        // Fetch the MIR for each local definition in case it gets stolen by `safeishly_grab_instance_mir`
        // and `Instance::instantiate_mir_and_normalize_erasing_regions`.
        //
        // N.B. we use this instead of `iter_local_def_id` to avoid freezing the definition map.
        for i in 0..id_count {
            let local_def = LocalDefId {
                local_def_index: DefIndex::from_usize(i),
            };

            let Some(body) = safeishly_grab_def_id_mir(tcx, local_def) else {
                continue;
            };

            // HACK: `mir_built` can call `layout_of`, which can call `eval_to_const_value_raw` and
            //  now all bets are off about getting the MIR. We're just praying this MIR isn't actually
            //  load-bearing but there's no proof that this is actually the case.
            let body = unsafe {
                // Safety: there is none.
                std::mem::transmute::<&'tcx Steal<Body<'tcx>>, &RwLock<Option<Body<'tcx>>>>(body)
            };

            if let Some(borrow) = &*body.read() {
                dbg!(local_def, borrow);
                self.local_mirs.insert(local_def, borrow.clone());
            }
        }

        // Get the token use sets of each function.
        assert!(!tcx.untracked().definitions.is_frozen());

        for i in 0..id_count {
            let local_def = LocalDefId {
                local_def_index: DefIndex::from_usize(i),
            };

            // Ensure that we're analyzing a function...
            if !matches!(tcx.def_kind(local_def), DefKind::Fn | DefKind::AssocFn) {
                continue;
            }

            // ...which can be properly monomorphized.
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

        for (instance, facts) in &self.func_facts {
            let facts = facts.as_ref().unwrap();
            let Some(orig_id) = instance.def_id().as_local() else {
                continue;
            };

            // Modify body
            let Some(mut body) = self.local_mirs.get(&orig_id).cloned() else {
                // HACK: See above comment.
                continue;
            };

            {
                // Create a local for every token.
                let token_ref_not_ty =
                    Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);

                let token_ref_mut_ty =
                    Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);

                let dangling_addr_local_ty = Ty::new_mut_ptr(tcx, tcx.types.unit);
                let dangling_addr_local = body
                    .local_decls
                    .push(LocalDecl::new(dangling_addr_local_ty, DUMMY_SP));

                let token_locals = facts
                    .borrows
                    .iter()
                    .map(|(key, (mutability, _))| {
                        (
                            *key,
                            body.local_decls.push(LocalDecl::new(
                                match mutability {
                                    Mutability::Not => token_ref_not_ty,
                                    Mutability::Mut => token_ref_mut_ty,
                                },
                                DUMMY_SP,
                            )),
                        )
                    })
                    .collect::<FxHashMap<_, _>>();

                // Initialize the tokens and ascribe them their types.
                let source_info = SourceInfo {
                    span: DUMMY_SP,
                    // FIXME: This probably isn't a good idea.
                    scope: SourceScope::from_u32(0),
                };
                let mut start_stmts = Vec::new();
                start_stmts.extend([Statement {
                    source_info,
                    kind: StatementKind::Assign(Box::new((
                        Place {
                            local: dangling_addr_local,
                            projection: List::empty(),
                        },
                        Rvalue::Cast(
                            CastKind::PointerFromExposedAddress,
                            Operand::Constant(Box::new(ConstOperand {
                                span: DUMMY_SP,
                                user_ty: None,
                                const_: Const::Val(
                                    ConstValue::Scalar(Scalar::from_target_usize(
                                        1,
                                        &tcx.data_layout,
                                    )),
                                    tcx.types.usize,
                                ),
                            })),
                            dangling_addr_local_ty,
                        ),
                    ))),
                }]);

                for (key, &local) in &token_locals {
                    let (mutability, lt_id) = &facts.borrows[key];
                    let borrow_kind = match mutability {
                        Mutability::Not => BorrowKind::Shared,
                        Mutability::Mut => BorrowKind::Mut {
                            kind: MutBorrowKind::Default,
                        },
                    };

                    if let Some(lt_id) = lt_id {
                        let late_vars = tcx.late_bound_vars(HirId {
                            owner: OwnerId { def_id: orig_id },
                            local_id: ItemLocalId::from_u32(0),
                        });
                        let BoundVariableKind::Region(BoundRegionKind::BrNamed(did, name)) =
                            late_vars[*lt_id as usize]
                        else {
                            unreachable!();
                        };

                        let annotation =
                            body.user_type_annotations
                                .push(CanonicalUserTypeAnnotation {
                                    user_ty: Box::new(CanonicalUserType {
                                        value: UserType::Ty(Ty::new_ref(
                                            tcx,
                                            Region::new_late_param(
                                                tcx,
                                                orig_id.to_def_id(),
                                                BoundRegionKind::BrNamed(did, name),
                                            ),
                                            TypeAndMut {
                                                mutbl: *mutability,
                                                ty: tcx.types.unit,
                                            },
                                        )),
                                        max_universe: UniverseIndex::ROOT,
                                        variables: List::empty(),
                                    }),
                                    span: DUMMY_SP,
                                    inferred_ty: match mutability {
                                        Mutability::Not => token_ref_not_ty,
                                        Mutability::Mut => token_ref_mut_ty,
                                    },
                                });

                        start_stmts.extend([
                            Statement {
                                source_info,
                                kind: StatementKind::Assign(Box::new((
                                    Place {
                                        local,
                                        projection: List::empty(),
                                    },
                                    Rvalue::Ref(
                                        tcx.lifetimes.re_erased,
                                        borrow_kind,
                                        Place {
                                            local: dangling_addr_local,
                                            projection: tcx
                                                .mk_place_elems(&[ProjectionElem::Deref]),
                                        },
                                    ),
                                ))),
                            },
                            Statement {
                                source_info,
                                kind: StatementKind::AscribeUserType(
                                    Box::new((
                                        Place {
                                            local,
                                            projection: List::empty(),
                                        },
                                        UserTypeProjection {
                                            base: annotation,
                                            projs: Vec::new(),
                                        },
                                    )),
                                    Variance::Invariant,
                                ),
                            },
                        ]);
                    } else {
                        let unit_holder = body
                            .local_decls
                            .push(LocalDecl::new(tcx.types.unit, DUMMY_SP));

                        start_stmts.extend([
                            Statement {
                                source_info,
                                kind: StatementKind::Assign(Box::new((
                                    Place {
                                        local: unit_holder,
                                        projection: List::empty(),
                                    },
                                    Rvalue::Use(Operand::Constant(Box::new(ConstOperand {
                                        span: DUMMY_SP,
                                        user_ty: None,
                                        const_: Const::Val(ConstValue::ZeroSized, tcx.types.unit),
                                    }))),
                                ))),
                            },
                            Statement {
                                source_info,
                                kind: StatementKind::Assign(Box::new((
                                    Place {
                                        local,
                                        projection: List::empty(),
                                    },
                                    Rvalue::Ref(
                                        tcx.lifetimes.re_erased,
                                        borrow_kind,
                                        Place {
                                            local: unit_holder,
                                            projection: List::empty(),
                                        },
                                    ),
                                ))),
                            },
                        ]);
                    }
                }

                body.basic_blocks.as_mut()[BasicBlock::from_u32(0)]
                    .statements
                    .splice(0..0, start_stmts);

                // For every function call...
                for bb in body.basic_blocks.as_mut() {
                    // If it has a concrete callee...
                    let Some(terminator) = &bb.terminator else {
                        continue;
                    };

                    let Some(target_instance) = get_static_callee_from_terminator(
                        tcx,
                        instance,
                        &body.local_decls,
                        terminator,
                    ) else {
                        continue;
                    };

                    // Determine what it borrows
                    let Some(callee_borrows) = &self.func_facts.get(&target_instance) else {
                        // This could happen if the optimized MIR reveals that a given function is
                        // unreachable.
                        continue;
                    };

                    // TODO: Add borrow commands to the MIR here.
                }
            }

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
            dbg!(shadow.def_id(), tcx.mir_built(shadow.def_id()));
            let _ = tcx.mir_borrowck(shadow.def_id());
        }
    }

    fn analyze_fn_facts(&mut self, tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) {
        // Ensure that we don't analyze the same function circularly or redundantly.
        let hash_map::Entry::Vacant(entry) = self.func_facts.entry(instance) else {
            return;
        };

        // If this function has a hardcoded fact set, use those.
        let hardcoded_mut = Self::is_special_func(tcx, instance.def_id());

        if let Some(hardcoded_mut) = hardcoded_mut {
            entry.insert(Some(FuncFacts {
                borrows: HashMap::from_iter([(
                    instance.args[1].as_type().unwrap(),
                    (hardcoded_mut, None),
                )]),
            }));
            return;
        }

        // Acquire the function body.
        let MirGrabResult::Found(body) = safeishly_grab_instance_mir(tcx, instance.def) else {
            return;
        };

        // This is a real function so let's add it to the fact map.
        entry.insert(None);

        // See who the function may call and where.
        let mut borrows = FxHashMap::default();

        for bb in body.basic_blocks.iter() {
            // If the terminator is a call terminator.
            let Some(terminator) = &bb.terminator else {
                continue;
            };
            let Some(target_instance) =
                get_static_callee_from_terminator(tcx, &instance, &body.local_decls, terminator)
            else {
                continue;
            };

            // Recurse into its callee.
            self.analyze_fn_facts(tcx, target_instance);

            // ...and add its borrows to the borrows set.
            let Some(Some(target_facts)) = &self.func_facts.get(&target_instance) else {
                continue;
            };

            let lt_idx =
                Self::is_special_func(tcx, target_instance.def_id()).map(|_| match target_instance
                    .args[0]
                    .as_const()
                    .unwrap()
                    .eval(tcx, ParamEnv::reveal_all(), None)
                    .unwrap()
                {
                    ValTree::Leaf(scalar) => scalar.try_to_u32().unwrap(),
                    _ => unreachable!(),
                });

            for (borrow_key, (borrow_mut, _)) in &target_facts.borrows {
                let (curr_mut, curr_idx) = borrows
                    .entry(*borrow_key)
                    .or_insert((Mutability::Not, None));

                if borrow_mut.is_mut() {
                    *curr_mut = Mutability::Mut;
                }

                if let Some(lt_idx) = lt_idx.filter(|idx| *idx != u32::MAX) {
                    *curr_idx = Some(lt_idx);
                }
            }
        }

        self.func_facts
            .insert(instance, Some(FuncFacts { borrows }));
    }

    fn is_special_func(tcx: TyCtxt<'_>, def_id: DefId) -> Option<Mutability> {
        match tcx.opt_item_name(def_id) {
            v if v == Some(sym::__autoken_declare_tied_ref.get()) => Some(Mutability::Not),
            v if v == Some(sym::__autoken_declare_tied_mut.get()) => Some(Mutability::Mut),
            _ => None,
        }
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
