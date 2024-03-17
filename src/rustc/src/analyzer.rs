use std::collections::{hash_map, HashMap};

use rustc_hir::{
    def::DefKind,
    def_id::{DefId, DefIndex, LocalDefId},
};

use rustc_index::IndexVec;
use rustc_middle::{
    mir::{
        interpret::Scalar, AggregateKind, BasicBlock, BorrowKind, CastKind, Const, ConstOperand,
        ConstValue, LocalDecl, MutBorrowKind, Mutability, Operand, Place, ProjectionElem, Rvalue,
        SourceInfo, SourceScope, Statement, StatementKind, Terminator, TerminatorKind,
        UserTypeProjection,
    },
    ty::{
        fold::RegionFolder, BoundRegion, BoundRegionKind, BoundVar, Canonical, CanonicalUserType,
        CanonicalUserTypeAnnotation, CanonicalVarInfo, CanonicalVarKind, DebruijnIndex,
        GenericArgs, GenericParamDefKind, Instance, List, ParamEnv, Region, RegionKind, Ty, TyCtxt,
        TyKind, TypeAndMut, TypeFoldable, UniverseIndex, UserType, ValTree, Variance,
    },
};
use rustc_span::{Symbol, DUMMY_SP};
use rustc_target::abi::FieldIdx;

use crate::{
    analyzer::sym::unnamed,
    util::{
        feeder::{
            feed,
            feeders::{MirBuiltFeeder, MirBuiltStasher},
            read_feed,
        },
        hash::FxHashMap,
        mir::{
            find_region_with_name, get_static_callee_from_terminator, safeishly_grab_def_id_mir,
            safeishly_grab_instance_mir, MirGrabResult,
        },
        ty::instantiate_ignoring_regions,
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
    borrows: FxHashMap<Ty<'tcx>, (Mutability, Option<Symbol>)>,
}

impl<'tcx> AnalysisDriver<'tcx> {
    pub fn analyze(&mut self, tcx: TyCtxt<'tcx>) {
        let id_count = tcx.untracked().definitions.read().def_index_count();

        // Fetch the MIR for each local definition to populate the `MirBuiltStasher`.
        //
        // N.B. we use this instead of `iter_local_def_id` to avoid freezing the definition map.
        for i in 0..id_count {
            let local_def = LocalDefId {
                local_def_index: DefIndex::from_usize(i),
            };

            if safeishly_grab_def_id_mir(tcx, local_def).is_some() {
                assert!(read_feed::<MirBuiltStasher>(tcx, local_def).is_some());
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
            let mut args_wf = true;
            let args =
                // N.B. we use `for_item` instead of `tcx.generics_of` to ensure that we also iterate
                // over the generic arguments of the parent.
                GenericArgs::for_item(tcx, local_def.to_def_id(), |param, _| match param.kind {
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

            if args_wf {
                self.analyze_fn_facts(tcx, Instance::new(local_def.to_def_id(), args));
            }
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
            let Some(mut body) = read_feed::<MirBuiltStasher>(tcx, orig_id).cloned() else {
                // Some `DefIds` with facts are just shims—not functions with actual MIR.
                continue;
            };

            {
                // Create a local for every token.
                let token_ref_imm_ty =
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
                                    Mutability::Not => token_ref_imm_ty,
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
                        // Find the lifetime
                        let found_region = find_region_with_name(
                            tcx,
                            tcx.fn_sig(orig_id).skip_binder().skip_binder().output(),
                            *lt_id,
                        )
                        .unwrap();

                        let annotation =
                            body.user_type_annotations
                                .push(CanonicalUserTypeAnnotation {
                                    user_ty: Box::new(CanonicalUserType {
                                        value: UserType::Ty(Ty::new_ref(
                                            tcx,
                                            found_region,
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
                                        Mutability::Not => token_ref_imm_ty,
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

                let bbs = body.basic_blocks.as_mut();
                bbs[BasicBlock::from_u32(0)]
                    .statements
                    .splice(0..0, start_stmts);

                // For every function call...
                for bb_idx in 0..bbs.len() {
                    let bb = &mut bbs[BasicBlock::from_usize(bb_idx)];

                    // If it has a concrete callee...
                    let Some(Terminator {
                        kind:
                            TerminatorKind::Call {
                                func: callee,
                                destination,
                                target,
                                ..
                            },
                        ..
                    }) = &bb.terminator
                    else {
                        continue;
                    };

                    let Some(target_instance) =
                        get_static_callee_from_terminator(tcx, instance, &body.local_decls, callee)
                    else {
                        continue;
                    };

                    let target_fn_ty_totally_generic = tcx
                        .type_of(target_instance.def_id())
                        .skip_binder()
                        // FIXME: This might be a closure.
                        .fn_sig(tcx)
                        .skip_binder();

                    let target_fn_out_ty_semi_generic_intact_regions = {
                        let callee = callee.ty(&body.local_decls, tcx);
                        let TyKind::FnDef(_callee_id, generics) = callee.kind() else {
                            unreachable!();
                        };

                        instantiate_ignoring_regions(
                            tcx,
                            target_fn_ty_totally_generic.output(),
                            generics,
                        )
                    };

                    // Determine what it borrows
                    let Some(callee_borrows) = &self.func_facts.get(&target_instance) else {
                        // This could happen if the optimized MIR reveals that a given function is
                        // unreachable.
                        continue;
                    };

                    let callee_borrows = callee_borrows.as_ref().unwrap();

                    // Add borrow directives before the function.
                    for (ty, (mutability, _)) in &callee_borrows.borrows {
                        let dummy_token_holder = match mutability {
                            Mutability::Not => body
                                .local_decls
                                .push(LocalDecl::new(token_ref_imm_ty, DUMMY_SP)),
                            Mutability::Mut => body
                                .local_decls
                                .push(LocalDecl::new(token_ref_mut_ty, DUMMY_SP)),
                        };

                        bb.statements.push(Statement {
                            source_info,
                            kind: StatementKind::Assign(Box::new((
                                Place {
                                    local: dummy_token_holder,
                                    projection: List::empty(),
                                },
                                Rvalue::Ref(
                                    tcx.lifetimes.re_erased,
                                    match mutability {
                                        Mutability::Not => BorrowKind::Shared,
                                        Mutability::Mut => BorrowKind::Mut {
                                            kind: MutBorrowKind::Default,
                                        },
                                    },
                                    Place {
                                        local: token_locals[ty],
                                        projection: tcx.mk_place_elems(&[ProjectionElem::Deref]),
                                    },
                                ),
                            ))),
                        });
                    }

                    // Add ascriptions to the return type after.
                    let Some(target) = target else {
                        continue;
                    };
                    let target = *target;
                    let destination = *destination;

                    let bb = &mut bbs[target];

                    let Some(target_facts) = &self.func_facts.get(&target_instance) else {
                        continue;
                    };
                    let target_facts = target_facts.as_ref().unwrap();

                    let mut prepend_statements = Vec::new();

                    for (ty, (mutability, lt_id)) in &target_facts.borrows {
                        let Some(lt_id) = lt_id else {
                            continue;
                        };

                        // Compute the type as which the function result is going to be bound.
                        let mapped_region = find_region_with_name(
                            tcx,
                            target_fn_ty_totally_generic.output(),
                            *lt_id,
                        )
                        .unwrap();

                        let mut var_assignments = FxHashMap::default();
                        var_assignments.insert(mapped_region, BoundVar::from_usize(0));

                        let fn_result = target_fn_out_ty_semi_generic_intact_regions.fold_with(
                            &mut RegionFolder::new(tcx, &mut |region, index| {
                                match region.kind() {
                                    // Mapped regions
                                    RegionKind::ReEarlyParam(_) | RegionKind::ReLateParam(_) => {
                                        if index == DebruijnIndex::from_u32(0) {
                                            let var_assignments_count =
                                                var_assignments.len() as u32;
                                            let bound_var =
                                                *var_assignments.entry(region).or_insert_with(
                                                    || BoundVar::from_u32(var_assignments_count),
                                                );

                                            Region::new_bound(
                                                tcx,
                                                DebruijnIndex::from_u32(0),
                                                BoundRegion {
                                                    kind: BoundRegionKind::BrAnon,
                                                    var: bound_var,
                                                },
                                            )
                                        } else {
                                            region
                                        }
                                    }

                                    // Unaffected regions
                                    RegionKind::ReBound(_, _) => region,
                                    RegionKind::ReStatic => region,

                                    // Non-applicable regions
                                    RegionKind::ReVar(_) => unreachable!(),
                                    RegionKind::RePlaceholder(_) => unreachable!(),
                                    RegionKind::ReErased => unreachable!(),
                                    RegionKind::ReError(_) => unreachable!(),
                                }
                            }),
                        );

                        let fn_result_inferred = destination.ty(&body.local_decls, tcx).ty;

                        // Create a tuple binder
                        let tuple_binder = Ty::new_tup(
                            tcx,
                            &[
                                Ty::new_ref(
                                    tcx,
                                    Region::new_bound(
                                        tcx,
                                        DebruijnIndex::from_u32(0),
                                        BoundRegion {
                                            kind: BoundRegionKind::BrAnon,
                                            var: BoundVar::from_u32(0),
                                        },
                                    ),
                                    TypeAndMut {
                                        mutbl: *mutability,
                                        ty: tcx.types.unit,
                                    },
                                ),
                                fn_result,
                            ],
                        );

                        let tuple_binder_inferred = Ty::new_tup(
                            tcx,
                            &[
                                Ty::new_ref(
                                    tcx,
                                    tcx.lifetimes.re_erased,
                                    TypeAndMut {
                                        mutbl: *mutability,
                                        ty: tcx.types.unit,
                                    },
                                ),
                                fn_result_inferred,
                            ],
                        );

                        // Emit a type ascription statement
                        let annotation =
                            body.user_type_annotations
                                .push(CanonicalUserTypeAnnotation {
                                    user_ty: Box::new(Canonical {
                                        value: UserType::Ty(tuple_binder),
                                        max_universe: UniverseIndex::ROOT,
                                        variables: tcx.mk_canonical_var_infos(
                                            &var_assignments
                                                .iter()
                                                .map(|_| CanonicalVarInfo {
                                                    kind: CanonicalVarKind::Region(
                                                        UniverseIndex::ROOT,
                                                    ),
                                                })
                                                .collect::<Vec<_>>(),
                                        ),
                                    }),
                                    span: DUMMY_SP,
                                    inferred_ty: tuple_binder_inferred,
                                });

                        let binder_local = body
                            .local_decls
                            .push(LocalDecl::new(tuple_binder_inferred, DUMMY_SP));

                        body.local_decls[destination.local].mutability = Mutability::Mut;

                        let dummy_token_holder = match mutability {
                            Mutability::Not => body
                                .local_decls
                                .push(LocalDecl::new(token_ref_imm_ty, DUMMY_SP)),
                            Mutability::Mut => body
                                .local_decls
                                .push(LocalDecl::new(token_ref_mut_ty, DUMMY_SP)),
                        };

                        prepend_statements.extend([
                            Statement {
                                source_info,
                                kind: StatementKind::Assign(Box::new((
                                    Place {
                                        local: dummy_token_holder,
                                        projection: List::empty(),
                                    },
                                    Rvalue::Ref(
                                        tcx.lifetimes.re_erased,
                                        match mutability {
                                            Mutability::Not => BorrowKind::Shared,
                                            Mutability::Mut => BorrowKind::Mut {
                                                kind: MutBorrowKind::Default,
                                            },
                                        },
                                        Place {
                                            local: token_locals[ty],
                                            projection: tcx
                                                .mk_place_elems(&[ProjectionElem::Deref]),
                                        },
                                    ),
                                ))),
                            },
                            Statement {
                                source_info,
                                kind: StatementKind::Assign(Box::new((
                                    Place {
                                        local: binder_local,
                                        projection: List::empty(),
                                    },
                                    Rvalue::Aggregate(
                                        Box::new(AggregateKind::Tuple),
                                        IndexVec::from_iter([
                                            Operand::Move(Place {
                                                local: dummy_token_holder,
                                                projection: List::empty(),
                                            }),
                                            Operand::Move(destination),
                                        ]),
                                    ),
                                ))),
                            },
                            Statement {
                                source_info,
                                kind: StatementKind::AscribeUserType(
                                    Box::new((
                                        Place {
                                            local: binder_local,
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
                            Statement {
                                source_info,
                                kind: StatementKind::Assign(Box::new((
                                    destination,
                                    Rvalue::Use(Operand::Move(Place {
                                        local: binder_local,
                                        projection: tcx.mk_place_elems(&[ProjectionElem::Field(
                                            FieldIdx::from_u32(1),
                                            fn_result_inferred,
                                        )]),
                                    })),
                                ))),
                            },
                        ]);
                    }

                    bb.statements.splice(0..0, prepend_statements);
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
            // dbg!(shadow.def_id(), tcx.mir_built(shadow.def_id()));
            let _ = tcx.mir_borrowck(shadow.def_id());
        }
    }

    // FIXME: Ensure that facts collected after a self-recursive function was analyzed are also
    //  propagated to it.
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
            let Some(Terminator {
                kind: TerminatorKind::Call { func: callee, .. },
                ..
            }) = &bb.terminator
            else {
                continue;
            };

            let Some(target_instance) =
                get_static_callee_from_terminator(tcx, &instance, &body.local_decls, callee)
            else {
                continue;
            };

            // Recurse into its callee.
            self.analyze_fn_facts(tcx, target_instance);

            // ...and add its borrows to the borrows set.
            let Some(Some(target_facts)) = &self.func_facts.get(&target_instance) else {
                continue;
            };

            let lt_id =
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

                if let Some(lt_id) = lt_id.filter(|idx| *idx != u32::MAX) {
                    *curr_idx = Some(Symbol::intern(&format!("'autoken_{lt_id}")));
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
