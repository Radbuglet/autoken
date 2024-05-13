use rustc_index::IndexVec;
use rustc_middle::{
    mir::{
        interpret::Scalar, AggregateKind, BasicBlock, Body, BorrowKind, CastKind, Const,
        ConstOperand, ConstValue, Local, LocalDecl, MutBorrowKind, Mutability, Operand, Place,
        ProjectionElem, Rvalue, SourceInfo, SourceScope, Statement, StatementKind, Terminator,
        TerminatorKind, UserTypeProjection,
    },
    ty::{
        fold::RegionFolder, BoundRegion, BoundRegionKind, BoundVar, Canonical,
        CanonicalUserTypeAnnotation, CanonicalVarInfo, CanonicalVarKind, DebruijnIndex, List,
        ParamEnv, Region, Ty, TyCtxt, TypeAndMut, TypeFoldable, UniverseIndex, UserType, Variance,
    },
};
use rustc_span::DUMMY_SP;
use rustc_target::abi::FieldIdx;

use crate::util::ty::FunctionCallAndRegions;

type PrependerState<'tcx> = (Vec<Statement<'tcx>>, BasicBlock);

pub struct TokenMirBuilder<'tcx, 'body> {
    tcx: TyCtxt<'tcx>,
    param_env_user: ParamEnv<'tcx>,
    body: &'body mut Body<'tcx>,

    // Caches
    token_ref_ty: Ty<'tcx>,
    token_ref_mut_ty: Ty<'tcx>,
    dangling_addr_local: Local,
    default_source_info: SourceInfo,

    // Addition queues
    prepender: PrependerState<'tcx>,
}

impl<'tcx, 'body> TokenMirBuilder<'tcx, 'body> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        param_env_user: ParamEnv<'tcx>,
        body: &'body mut Body<'tcx>,
    ) -> Self {
        // token_ref_ty = &'erased ()
        let token_ref_ty = Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);

        // token_ref_mut_ty = &'erased mut ()
        let token_ref_mut_ty = Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);

        // let dangling_addr_local: *mut ();
        let dangling_addr_local_ty = Ty::new_mut_ptr(tcx, tcx.types.unit);
        let dangling_addr_local = body
            .local_decls
            .push(LocalDecl::new(dangling_addr_local_ty, DUMMY_SP));

        let default_source_info = SourceInfo {
            span: body.span,
            scope: SourceScope::from_u32(0),
        };

        let mut prepender = (Vec::new(), BasicBlock::from_u32(0));

        // Define
        // dangling_addr_local = 0x1 as *mut ()
        Self::prepend_statement_raw(
            body,
            &mut prepender,
            BasicBlock::from_u32(0),
            [Statement {
                source_info: default_source_info,
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
                                ConstValue::Scalar(Scalar::from_target_usize(1, &tcx.data_layout)),
                                tcx.types.usize,
                            ),
                        })),
                        dangling_addr_local_ty,
                    ),
                ))),
            }],
        );

        Self {
            tcx,
            param_env_user,
            body,

            // Caches
            token_ref_ty,
            token_ref_mut_ty,
            dangling_addr_local,
            default_source_info,

            // Addition queues
            prepender,
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

    fn flush_prepended(&mut self) {
        Self::flush_prepended_raw(self.body, &mut self.prepender);
    }

    fn prepend_statement(
        &mut self,
        bb: BasicBlock,
        stmts: impl IntoIterator<Item = Statement<'tcx>>,
    ) {
        Self::prepend_statement_raw(self.body, &mut self.prepender, bb, stmts)
    }

    // === Tokens === //

    #[must_use]
    fn create_token(&mut self) -> (Local, Statement<'tcx>) {
        let local = self
            .body
            .local_decls
            .push(LocalDecl::new(self.token_ref_mut_ty, DUMMY_SP));

        (
            local,
            Statement {
                source_info: self.default_source_info,
                kind: StatementKind::Assign(Box::new((
                    Place {
                        local,
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
            },
        )
    }

    // === Calls === //

    pub fn ensure_not_borrowed_at(&mut self, bb: BasicBlock) -> Local {
        let (local, local_initializer) = self.create_token();

        let dummy_token_holder = self
            .body
            .local_decls
            .push(LocalDecl::new(self.token_ref_ty, DUMMY_SP));

        let source_info = self.body.basic_blocks[bb]
            .terminator
            .as_ref()
            .map_or(self.default_source_info, |sf| sf.source_info);

        self.body.basic_blocks.as_mut_preserves_cfg()[bb]
            .statements
            .extend([
                local_initializer,
                Statement {
                    source_info,
                    kind: StatementKind::Assign(Box::new((
                        Place {
                            local: dummy_token_holder,
                            projection: List::empty(),
                        },
                        Rvalue::Ref(
                            self.tcx.lifetimes.re_erased,
                            BorrowKind::Shared,
                            Place {
                                local,
                                projection: self.tcx.mk_place_elems(&[ProjectionElem::Deref]),
                            },
                        ),
                    ))),
                },
            ]);

        local
    }

    pub fn tie_token_to_function_return(
        &mut self,
        bb: BasicBlock,
        call: FunctionCallAndRegions<'tcx>,
        re_vid: BoundVar,
    ) -> Local {
        // Determine where the function call's return type is stored and the name of the basic block
        // jumped to immediately after the call.
        let Some(Terminator {
            kind:
                TerminatorKind::Call {
                    target: Some(target),
                    destination,
                    ..
                },
            source_info,
            ..
        }) = &mut self.body.basic_blocks.as_mut_preserves_cfg()[bb].terminator
        else {
            unreachable!();
        };

        let source_info = *source_info;
        let call_out_bb = *target;
        let orig_call_out_place = *destination;

        let fn_result = call.generalized.skip_binder();
        let fn_result_erased = self
            .tcx
            .normalize_erasing_regions(self.param_env_user, fn_result)
            .fold_with(&mut RegionFolder::new(self.tcx, &mut |_, _| {
                self.tcx.lifetimes.re_erased
            }));

        // N.B. Assignments in the MIR may occasionally upcast a type from its concrete form to its
        // opaque form. However, our type ascriptions always operate on their concrete form so we
        // need to defer this upcast by creating a temporary local with the concrete type.
        let new_call_out_place = self
            .body
            .local_decls
            .push(LocalDecl::new(fn_result_erased, DUMMY_SP));

        let new_call_out_place = Place {
            local: new_call_out_place,
            projection: List::empty(),
        };

        *destination = new_call_out_place;

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
                            var: re_vid,
                        },
                    ),
                    TypeAndMut {
                        mutbl: Mutability::Not,
                        ty: self.tcx.types.unit,
                    },
                ),
                fn_result,
            ],
        );

        let tuple_binder_erased = Ty::new_tup(
            self.tcx,
            &[
                Ty::new_ref(
                    self.tcx,
                    self.tcx.lifetimes.re_erased,
                    TypeAndMut {
                        mutbl: Mutability::Not,
                        ty: self.tcx.types.unit,
                    },
                ),
                fn_result_erased,
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
                        &(0..call.param_count)
                            .map(|_| CanonicalVarInfo {
                                kind: CanonicalVarKind::Region(UniverseIndex::ROOT),
                            })
                            .collect::<Vec<_>>(),
                    ),
                }),
                span: DUMMY_SP,
                inferred_ty: tuple_binder_erased,
            });

        let binder_local = self
            .body
            .local_decls
            .push(LocalDecl::new(tuple_binder_erased, DUMMY_SP));

        let (token_local, token_initializer) = self.create_token();
        let token_rb_local = self
            .body
            .local_decls
            .push(LocalDecl::new(self.token_ref_ty, DUMMY_SP));

        self.prepend_statement(
            call_out_bb,
            [
                token_initializer,
                Statement {
                    source_info,
                    kind: StatementKind::Assign(Box::new((
                        Place {
                            local: token_rb_local,
                            projection: List::empty(),
                        },
                        Rvalue::Ref(
                            self.tcx.lifetimes.re_erased,
                            BorrowKind::Shared,
                            Place {
                                local: token_local,
                                projection: self.tcx.mk_place_elems(&[ProjectionElem::Deref]),
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
                                    local: token_rb_local,
                                    projection: List::empty(),
                                }),
                                Operand::Move(new_call_out_place),
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
                        orig_call_out_place,
                        Rvalue::Use(Operand::Move(Place {
                            local: binder_local,
                            projection: self.tcx.mk_place_elems(&[ProjectionElem::Field(
                                FieldIdx::from_u32(1),
                                fn_result_erased,
                            )]),
                        })),
                    ))),
                },
            ],
        );
        self.flush_prepended();

        token_local
    }
}

impl Drop for TokenMirBuilder<'_, '_> {
    fn drop(&mut self) {
        self.flush_prepended();
    }
}
