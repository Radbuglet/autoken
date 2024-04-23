use rustc_hash::{FxHashMap, FxHashSet};
use rustc_index::IndexVec;
use rustc_middle::{
    mir::{
        interpret::Scalar, AggregateKind, BasicBlock, Body, BorrowKind, CastKind, Const,
        ConstOperand, ConstValue, Local, LocalDecl, MutBorrowKind, Mutability, Operand, Place,
        ProjectionElem, Rvalue, SourceInfo, SourceScope, Statement, StatementKind, Terminator,
        TerminatorKind, UserTypeProjection,
    },
    ty::{
        fold::RegionFolder, BoundRegion, BoundRegionKind, BoundVar, Canonical, CanonicalUserType,
        CanonicalUserTypeAnnotation, CanonicalVarInfo, CanonicalVarKind, DebruijnIndex, List,
        Region, RegionKind, Ty, TyCtxt, TyKind, TypeAndMut, TypeFoldable, UniverseIndex, UserType,
        Variance,
    },
};
use rustc_span::{Symbol, DUMMY_SP};
use rustc_target::abi::FieldIdx;

use crate::util::ty::{
    err_failed_to_find_region, find_region_with_name, get_fn_sig_maybe_closure,
    instantiate_ignoring_regions,
};

type PrependerState<'tcx> = (Vec<Statement<'tcx>>, BasicBlock);

pub struct TokenMirBuilder<'tcx, 'body> {
    tcx: TyCtxt<'tcx>,
    body: &'body mut Body<'tcx>,

    // Caches
    token_ref_imm_ty: Ty<'tcx>,
    token_ref_mut_ty: Ty<'tcx>,
    dangling_addr_local_ty: Ty<'tcx>,
    dangling_addr_local: Local,
    default_source_info: SourceInfo,

    // Addition queues
    preprender: PrependerState<'tcx>,
    tokens: FxHashMap<TokenKey<'tcx>, (Local, FxHashSet<Symbol>)>,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct TokenKey<'tcx>(pub Ty<'tcx>);

impl<'tcx, 'body> TokenMirBuilder<'tcx, 'body> {
    pub fn new(tcx: TyCtxt<'tcx>, body: &'body mut Body<'tcx>) -> Self {
        // token_ref_imm_ty = &'erased ()
        let token_ref_imm_ty = Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);

        // token_ref_mut_ty = &'erased ()
        let token_ref_mut_ty = Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);

        // let dangling_addr_local: *mut ();
        let dangling_addr_local_ty = Ty::new_mut_ptr(tcx, tcx.types.unit);
        let dangling_addr_local = body
            .local_decls
            .push(LocalDecl::new(dangling_addr_local_ty, DUMMY_SP));

        let default_source_scope = SourceInfo {
            span: body.span,
            // FIXME: This probably hurts diagnostics.
            scope: SourceScope::from_u32(0),
        };

        Self {
            tcx,
            body,

            // Caches
            token_ref_imm_ty,
            token_ref_mut_ty,
            dangling_addr_local_ty,
            dangling_addr_local,
            default_source_info: default_source_scope,

            // Addition queues
            preprender: (Vec::new(), BasicBlock::from_u32(0)),
            tokens: FxHashMap::default(),
        }
    }

    pub fn body(&self) -> &Body<'tcx> {
        self.body
    }

    // === Prepending === //

    fn flush_prepended_raw(body: &mut Body<'tcx>, prepender: &mut PrependerState<'tcx>) {
        body.basic_blocks.as_mut_preserves_cfg()[prepender.1]
            .statements
            .splice(0..0, prepender.0.drain(..));
    }

    fn prepend_statement_raw(
        body: &mut Body<'tcx>,
        prepender: &mut PrependerState<'tcx>,
        bb: BasicBlock,
        stmts: impl IntoIterator<Item = Statement<'tcx>>,
    ) {
        if prepender.1 != bb {
            Self::flush_prepended_raw(body, prepender);
            prepender.1 = bb;
        }
        prepender.0.extend(stmts);
    }

    fn prepend_statement(
        &mut self,
        bb: BasicBlock,
        stmts: impl IntoIterator<Item = Statement<'tcx>>,
    ) {
        Self::prepend_statement_raw(self.body, &mut self.preprender, bb, stmts)
    }

    fn flush_prepended(&mut self) {
        Self::flush_prepended_raw(self.body, &mut self.preprender);
    }

    // === Tokens === //

    fn get_token_local(&mut self, key: TokenKey<'tcx>) -> (Local, &mut FxHashSet<Symbol>) {
        let (local, tied) = self.tokens.entry(key).or_insert_with(|| {
            let local = self
                .body
                .local_decls
                .push(LocalDecl::new(self.token_ref_mut_ty, DUMMY_SP));

            (local, FxHashSet::default())
        });

        (*local, tied)
    }

    pub fn tie_token_to_my_return(&mut self, key: TokenKey<'tcx>, region_name: Symbol) {
        self.get_token_local(key).1.insert(region_name);
    }

    fn prepend_token_initializers(&mut self) {
        let init_basic_block = BasicBlock::from_u32(0);

        // dangling_addr_local = 0x1 as *mut ()
        self.prepend_statement(
            init_basic_block,
            [Statement {
                source_info: self.default_source_info,
                kind: StatementKind::Assign(Box::new((
                    Place {
                        local: self.dangling_addr_local,
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
                                    &self.tcx.data_layout,
                                )),
                                self.tcx.types.usize,
                            ),
                        })),
                        self.dangling_addr_local_ty,
                    ),
                ))),
            }],
        );

        for (local, ties) in self.tokens.values() {
            if ties.is_empty() {
                // unit_holder: ()
                let unit_holder = self
                    .body
                    .local_decls
                    .push(LocalDecl::new(self.tcx.types.unit, DUMMY_SP));

                Self::prepend_statement_raw(
                    self.body,
                    &mut self.preprender,
                    init_basic_block,
                    [
                        // unit_holder = ();
                        Statement {
                            source_info: self.default_source_info,
                            kind: StatementKind::Assign(Box::new((
                                Place {
                                    local: unit_holder,
                                    projection: List::empty(),
                                },
                                Rvalue::Use(Operand::Constant(Box::new(ConstOperand {
                                    span: DUMMY_SP,
                                    user_ty: None,
                                    const_: Const::Val(ConstValue::ZeroSized, self.tcx.types.unit),
                                }))),
                            ))),
                        },
                        // <local> = &mut unit_holder;
                        Statement {
                            source_info: self.default_source_info,
                            kind: StatementKind::Assign(Box::new((
                                Place {
                                    local: *local,
                                    projection: List::empty(),
                                },
                                Rvalue::Ref(
                                    self.tcx.lifetimes.re_erased,
                                    BorrowKind::Mut {
                                        kind: MutBorrowKind::Default,
                                    },
                                    Place {
                                        local: unit_holder,
                                        projection: List::empty(),
                                    },
                                ),
                            ))),
                        },
                    ],
                );
            } else {
                // <local> = &mut *dangling_addr_local
                Self::prepend_statement_raw(
                    self.body,
                    &mut self.preprender,
                    init_basic_block,
                    [Statement {
                        source_info: self.default_source_info,
                        kind: StatementKind::Assign(Box::new((
                            Place {
                                local: *local,
                                projection: List::empty(),
                            },
                            Rvalue::Ref(
                                self.tcx.lifetimes.re_erased,
                                BorrowKind::Mut {
                                    kind: MutBorrowKind::Default,
                                },
                                Place {
                                    local: self.dangling_addr_local,
                                    projection: self.tcx.mk_place_elems(&[ProjectionElem::Deref]),
                                },
                            ),
                        ))),
                    }],
                );

                for &tie in ties {
                    let found_region = match find_region_with_name(
                        self.tcx,
                        get_fn_sig_maybe_closure(self.tcx, self.body.source.def_id())
                            .skip_binder()
                            .skip_binder()
                            .output(),
                        tie,
                    ) {
                        Ok(re) => re,
                        Err(re) => {
                            err_failed_to_find_region(self.tcx, self.body.span, tie, &re);
                            continue;
                        }
                    };

                    // annotation => &'<found_region> mut ()
                    let annotation =
                        self.body
                            .user_type_annotations
                            .push(CanonicalUserTypeAnnotation {
                                user_ty: Box::new(CanonicalUserType {
                                    value: UserType::Ty(Ty::new_ref(
                                        self.tcx,
                                        found_region,
                                        TypeAndMut {
                                            mutbl: Mutability::Mut,
                                            ty: self.tcx.types.unit,
                                        },
                                    )),
                                    max_universe: UniverseIndex::ROOT,
                                    variables: List::empty(),
                                }),
                                span: DUMMY_SP,
                                inferred_ty: self.token_ref_mut_ty,
                            });

                    // let <local>: &'<found_region> mut () = <local>;
                    Self::prepend_statement_raw(
                        self.body,
                        &mut self.preprender,
                        init_basic_block,
                        [Statement {
                            source_info: self.default_source_info,
                            kind: StatementKind::AscribeUserType(
                                Box::new((
                                    Place {
                                        local: *local,
                                        projection: List::empty(),
                                    },
                                    UserTypeProjection {
                                        base: annotation,
                                        projs: Vec::new(),
                                    },
                                )),
                                Variance::Invariant,
                            ),
                        }],
                    );
                }
            }
        }
    }

    // === Calls === //

    pub fn ensure_not_borrowed_at(
        &mut self,
        bb: BasicBlock,
        key: TokenKey<'tcx>,
        mutability: Mutability,
    ) {
        let local = self.get_token_local(key).0;
        let dummy_token_holder = match mutability {
            Mutability::Not => self
                .body
                .local_decls
                .push(LocalDecl::new(self.token_ref_imm_ty, DUMMY_SP)),
            Mutability::Mut => self
                .body
                .local_decls
                .push(LocalDecl::new(self.token_ref_mut_ty, DUMMY_SP)),
        };

        self.body.basic_blocks.as_mut_preserves_cfg()[bb]
            .statements
            .push(Statement {
                source_info: self.default_source_info,
                kind: StatementKind::Assign(Box::new((
                    Place {
                        local: dummy_token_holder,
                        projection: List::empty(),
                    },
                    Rvalue::Ref(
                        self.tcx.lifetimes.re_erased,
                        match mutability {
                            Mutability::Not => BorrowKind::Shared,
                            Mutability::Mut => BorrowKind::Mut {
                                kind: MutBorrowKind::Default,
                            },
                        },
                        Place {
                            local,
                            projection: self.tcx.mk_place_elems(&[ProjectionElem::Deref]),
                        },
                    ),
                ))),
            });
    }

    pub fn tie_token_to_its_return(
        &mut self,
        bb: BasicBlock,
        key: TokenKey<'tcx>,
        mutability: Mutability,
        mut is_tied: impl FnMut(Region<'tcx>) -> bool,
    ) {
        // Determine where the function call's return type is stored and the name of the basic block
        // jumped to immediately after the call.
        let Some(Terminator {
            kind:
                TerminatorKind::Call {
                    func,
                    target: Some(target),
                    destination,
                    ..
                },
            ..
        }) = &self.body.basic_blocks.as_mut_preserves_cfg()[bb].terminator
        else {
            unreachable!();
        };

        let call_out_bb = *target;
        let call_out_place = *destination;

        // Determine the instance being called.
        let callee = func.ty(&self.body.local_decls, self.tcx);
        let TyKind::FnDef(callee_id, callee_generics) = callee.kind() else {
            unreachable!();
        };

        // Figure out its return type with all our body's generic parameters substituted in.
        let callee_out = instantiate_ignoring_regions(
            self.tcx,
            get_fn_sig_maybe_closure(self.tcx, *callee_id)
                .skip_binder()
                .skip_binder()
                .output(),
            callee_generics,
        );

        // Remap these regions to inference variables.
        let mut var_assignments = FxHashMap::default();
        let mut var_assignment_count = 1;

        let fn_result =
            callee_out.fold_with(&mut RegionFolder::new(self.tcx, &mut |region, index| {
                match region.kind() {
                    // Mapped regions
                    RegionKind::ReEarlyParam(_) | RegionKind::ReLateParam(_) => {
                        if index == DebruijnIndex::from_u32(0) {
                            let idx = if is_tied(region) {
                                0
                            } else {
                                var_assignment_count
                            };

                            let bound_var = *var_assignments.entry(region).or_insert_with(|| {
                                let bv = BoundVar::from_u32(idx);
                                var_assignment_count += 1;
                                bv
                            });

                            Region::new_bound(
                                self.tcx,
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
                    RegionKind::ReErased => {
                        // TODO: Make this less pessimistic.
                        Region::new_bound(
                            self.tcx,
                            DebruijnIndex::from_u32(0),
                            BoundRegion {
                                kind: BoundRegionKind::BrAnon,
                                var: BoundVar::from_u32(0),
                            },
                        )
                    }

                    // Unaffected regions
                    RegionKind::ReBound(_, _) => region,
                    RegionKind::ReStatic => region,

                    // Non-applicable regions
                    RegionKind::ReVar(_) => unreachable!(),
                    RegionKind::RePlaceholder(_) => unreachable!(),
                    RegionKind::ReError(_) => unreachable!(),
                }
            }));

        let fn_result_inferred = call_out_place.ty(&self.body.local_decls, self.tcx).ty;

        // Create the ascription type from this function.
        let tuple_binder = Ty::new_tup(
            self.tcx,
            &[
                Ty::new_ref(
                    self.tcx,
                    Region::new_bound(
                        self.tcx,
                        DebruijnIndex::from_u32(0),
                        BoundRegion {
                            kind: BoundRegionKind::BrAnon,
                            var: BoundVar::from_u32(0),
                        },
                    ),
                    TypeAndMut {
                        mutbl: mutability,
                        ty: self.tcx.types.unit,
                    },
                ),
                fn_result,
            ],
        );

        let tuple_binder_inferred = Ty::new_tup(
            self.tcx,
            &[
                Ty::new_ref(
                    self.tcx,
                    self.tcx.lifetimes.re_erased,
                    TypeAndMut {
                        mutbl: mutability,
                        ty: self.tcx.types.unit,
                    },
                ),
                fn_result_inferred,
            ],
        );

        // Emit a type ascription statement
        let annotation = self
            .body
            .user_type_annotations
            .push(CanonicalUserTypeAnnotation {
                user_ty: Box::new(Canonical {
                    value: UserType::Ty(tuple_binder),
                    max_universe: UniverseIndex::ROOT,
                    variables: self.tcx.mk_canonical_var_infos(
                        &(0..var_assignment_count)
                            .map(|_| CanonicalVarInfo {
                                kind: CanonicalVarKind::Region(UniverseIndex::ROOT),
                            })
                            .collect::<Vec<_>>(),
                    ),
                }),
                span: DUMMY_SP,
                inferred_ty: tuple_binder_inferred,
            });

        let binder_local = self
            .body
            .local_decls
            .push(LocalDecl::new(tuple_binder_inferred, DUMMY_SP));

        let token_local = self.get_token_local(key).0;
        let token_rb_local = match mutability {
            Mutability::Not => self
                .body
                .local_decls
                .push(LocalDecl::new(self.token_ref_imm_ty, DUMMY_SP)),
            Mutability::Mut => self
                .body
                .local_decls
                .push(LocalDecl::new(self.token_ref_mut_ty, DUMMY_SP)),
        };

        self.body.local_decls[call_out_place.local].mutability = Mutability::Mut;

        self.prepend_statement(
            call_out_bb,
            [
                Statement {
                    source_info: self.default_source_info,
                    kind: StatementKind::Assign(Box::new((
                        Place {
                            local: token_rb_local,
                            projection: List::empty(),
                        },
                        Rvalue::Ref(
                            self.tcx.lifetimes.re_erased,
                            match mutability {
                                Mutability::Not => BorrowKind::Shared,
                                Mutability::Mut => BorrowKind::Mut {
                                    kind: MutBorrowKind::Default,
                                },
                            },
                            Place {
                                local: token_local,
                                projection: self.tcx.mk_place_elems(&[ProjectionElem::Deref]),
                            },
                        ),
                    ))),
                },
                Statement {
                    source_info: self.default_source_info,
                    kind: StatementKind::Assign(Box::new((
                        Place {
                            local: binder_local,
                            projection: List::empty(),
                        },
                        Rvalue::Aggregate(
                            Box::new(AggregateKind::Tuple),
                            IndexVec::from_iter([
                                Operand::Move(Place {
                                    local: token_rb_local,
                                    projection: List::empty(),
                                }),
                                Operand::Move(call_out_place),
                            ]),
                        ),
                    ))),
                },
                Statement {
                    source_info: self.default_source_info,
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
                    source_info: self.default_source_info,
                    kind: StatementKind::Assign(Box::new((
                        call_out_place,
                        Rvalue::Use(Operand::Move(Place {
                            local: binder_local,
                            projection: self.tcx.mk_place_elems(&[ProjectionElem::Field(
                                FieldIdx::from_u32(1),
                                fn_result_inferred,
                            )]),
                        })),
                    ))),
                },
            ],
        );
    }
}

impl Drop for TokenMirBuilder<'_, '_> {
    fn drop(&mut self) {
        self.prepend_token_initializers();
        self.flush_prepended();
    }
}
