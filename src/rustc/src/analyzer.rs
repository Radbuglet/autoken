use std::collections::{hash_map, HashMap};

use rustc_hir::{def::DefKind, HirId, ItemLocalId, Node, OwnerId, OwnerNodes, ParentedNode};

use rustc_middle::{
    mir::{
        BorrowKind, Local, LocalDecl, MutBorrowKind, Mutability, Operand, Place, ProjectionElem,
        Rvalue, SourceInfo, SourceScope, Statement, StatementKind, Terminator, TerminatorKind,
    },
    ty::{EarlyBinder, Instance, List, ParamEnv, Ty, TyCtxt, TyKind},
};
use rustc_span::{source_map::dummy_spanned, Symbol, DUMMY_SP};

use crate::util::{
    feeder::{
        feed,
        feeders::{HirOwnerNode, MirBuiltFeeder},
    },
    hash::FxHashMap,
    mir::{safeishly_grab_instance_mir, MirGrabResult},
};

// === Engine === //

#[derive(Debug, Default)]
pub struct AnalysisDriver<'tcx> {
    func_facts: FxHashMap<Instance<'tcx>, Option<FuncFacts<'tcx>>>,
}

#[derive(Debug)]
struct FuncFacts<'tcx> {
    borrows: FxHashMap<Ty<'tcx>, Mutability>,
}

impl<'tcx> AnalysisDriver<'tcx> {
    pub fn analyze(&mut self, tcx: TyCtxt<'tcx>) {
        // Get the token use sets of each function.
        for local_def in tcx.iter_local_def_id() {
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

        // Generate shadow functions for each visited function.
        // TODO

        // Borrow-check these shadow functions.
        // TODO

        dbg!(self);
    }

    fn analyze_fn_facts(&mut self, tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) {
        // Ensure that we don't analyze the same function circularly or redundantly.
        if let hash_map::Entry::Vacant(entry) = self.func_facts.entry(instance) {
            entry.insert(None);
        } else {
            return;
        }

        // If this function has a hardcoded fact set, use those.
        let hardcoded_mut = match tcx.opt_item_name(instance.def_id()) {
            v if v == Some(sym::__autoken_declare_tied_ref.get()) => Some(Mutability::Not),
            v if v == Some(sym::__autoken_declare_tied_mut.get()) => Some(Mutability::Mut),
            _ => None,
        };

        if let Some(hardcoded_mut) = hardcoded_mut {
            self.func_facts.insert(
                instance,
                Some(FuncFacts {
                    borrows: HashMap::from_iter([(
                        instance.args[0].as_type().unwrap(),
                        hardcoded_mut,
                    )]),
                }),
            );
            return;
        }

        // Acquire the function body.
        let MirGrabResult::Found(body) = safeishly_grab_instance_mir(tcx, instance.def) else {
            return;
        };

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
            let Some(target_facts) = &self.func_facts[&target_instance] else {
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

    pub fn analyze_old(&mut self, tcx: TyCtxt<'tcx>) {
        // Find the main function
        let main_fn = {
            let mut fn_id = None;
            for &item in tcx.hir().root_module().item_ids {
                if tcx.hir().name(item.hir_id()) == Symbol::intern("whee") {
                    fn_id = Some(item.owner_id.def_id);
                }
            }
            fn_id.expect("missing `whee` in crate root")
        };

        // Find helper functions
        let tie_mut_shadow_fn = {
            let mut fn_id = None;
            for &item in tcx.hir().root_module().item_ids {
                if tcx.hir().name(item.hir_id()) == sym::__autoken_tie_mut_shadow.get() {
                    fn_id = Some(item.owner_id.def_id);
                }
            }
            fn_id.expect("missing `__autoken_tie_mut_shadow` in crate root")
        };

        // Get the MIR for the function.
        let body = tcx.mir_built(main_fn);

        // Create the shadow function's MIR.
        let mut body = body.borrow().clone();
        let source_info = SourceInfo {
            scope: SourceScope::from_u32(0),
            span: body.span,
        };

        let token_local = Local::from_u32(1);
        let token_local_ty = Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit);
        body.local_decls.as_mut_slice()[token_local].ty = token_local_ty;

        let token_local_rb = body.local_decls.push(LocalDecl::new(
            Ty::new_mut_ref(tcx, tcx.lifetimes.re_erased, tcx.types.unit),
            source_info.span,
        ));

        for bb in body.basic_blocks.as_mut().iter_mut() {
            let Some(Terminator {
                kind: TerminatorKind::Call { func, args, .. },
                ..
            }) = &mut bb.terminator
            else {
                continue;
            };

            let func_ty = func.ty(&body.local_decls, tcx);
            let TyKind::FnDef(callee_id, generics) = func_ty.kind() else {
                continue;
            };
            let callee_id = *callee_id;

            if tcx.item_name(callee_id) == sym::__autoken_tie_mut.get() {
                *func = Operand::function_handle(
                    tcx,
                    tie_mut_shadow_fn.to_def_id(),
                    *generics,
                    func.span(&body.local_decls),
                );

                args.push(dummy_spanned(Operand::Move(Place {
                    local: token_local_rb,
                    projection: List::empty(),
                })));

                bb.statements.push(Statement {
                    source_info,
                    kind: StatementKind::Assign(Box::new((
                        Place {
                            local: token_local_rb,
                            projection: List::empty(),
                        },
                        Rvalue::Ref(
                            tcx.lifetimes.re_erased,
                            BorrowKind::Mut {
                                kind: MutBorrowKind::Default,
                            },
                            Place {
                                local: token_local,
                                projection: tcx.mk_place_elems(&[ProjectionElem::Deref]),
                            },
                        ),
                    ))),
                });
            }
        }

        // Create the shadow function's DefId.

        //> Reserve its DefId.
        let main_fn_shadow_name = Symbol::intern(&format!(
            "{}_autoken_shadow",
            tcx.item_name(main_fn.to_def_id()),
        ));
        let main_fn_shadow = tcx.at(body.span).create_def(
            tcx.local_parent(main_fn),
            main_fn_shadow_name,
            DefKind::Fn,
        );

        //> Create its HIR
        let main_fn_hir_id = tcx.local_def_id_to_hir_id(main_fn);
        let main_fn_owner_nodes = tcx.hir_owner_nodes(main_fn_hir_id.owner);

        let main_fn_shadow_owner_nodes = {
            Box::leak(Box::new(OwnerNodes {
                opt_hash_including_bodies: None,
                nodes: {
                    let own_owner = OwnerId {
                        def_id: main_fn_shadow.def_id(),
                    };

                    let mut nodes = main_fn_owner_nodes.nodes.clone();

                    // Create a new node to hold the parameter type.
                    let token_local_hir_ty = HirId {
                        owner: own_owner,
                        local_id: ItemLocalId::from_usize(nodes.len()),
                    };
                    let token_local_hir_ty = tcx.arena.alloc(rustc_hir::Ty {
                        hir_id: token_local_hir_ty,
                        // TODO: Rewrite it as a reference
                        kind: rustc_hir::TyKind::Tup(&[]),
                        span: DUMMY_SP,
                    });
                    nodes.push(ParentedNode {
                        parent: ItemLocalId::from_u32(0),
                        node: Node::Ty(token_local_hir_ty),
                    });

                    // Adjust the entry-point's OwnerId and function signature just enough for the
                    // borrow-checker to pass. This is *super* unsound but this is a prototype so
                    // it's fine.
                    let fn_node = &mut nodes[ItemLocalId::from_u32(0)].node;

                    match fn_node {
                        Node::Item(p_item) => {
                            let mut item = **p_item;

                            // Edit 1: owner_id
                            item.owner_id = own_owner;

                            // Edit 2: signature
                            match &mut item.kind {
                                rustc_hir::ItemKind::Fn(sig, _, _) => {
                                    let mut decl = *sig.decl;
                                    let mut inputs = decl.inputs.to_vec();
                                    inputs[0] = *token_local_hir_ty;
                                    decl.inputs = tcx.arena.alloc_from_iter(inputs);
                                    sig.decl = tcx.arena.alloc(decl);
                                }
                                _ => unreachable!(),
                            }

                            *p_item = tcx.arena.alloc(item);
                        }
                        // Node::ImplItem(item) => match &mut item.kind {
                        //     rustc_hir::ImplItemKind::Fn(_, _) => todo!(),
                        //     _ => unreachable!(),
                        // },
                        _ => unreachable!(),
                    }

                    nodes
                },
                bodies: main_fn_owner_nodes.bodies.clone(),
            }))
        };

        //> Feed the query system the shadow function's properties.
        main_fn_shadow.opt_local_def_id_to_hir_id(Some(HirId {
            local_id: main_fn_hir_id.local_id,
            owner: OwnerId {
                def_id: main_fn_shadow.def_id(),
            },
        }));

        feed::<MirBuiltFeeder>(tcx, main_fn_shadow.def_id(), tcx.alloc_steal_mir(body));
        feed::<HirOwnerNode>(tcx, main_fn_shadow.def_id(), main_fn_shadow_owner_nodes);

        let main_fn_shadow = main_fn_shadow.def_id();

        // Borrow check the shadow function
        dbg!(tcx.fn_sig(main_fn_shadow));
        dbg!(&tcx.mir_borrowck(main_fn_shadow));
    }
}

#[allow(non_upper_case_globals)]
mod sym {
    use crate::util::mir::CachedSymbol;

    pub static __autoken_declare_tied_ref: CachedSymbol =
        CachedSymbol::new("__autoken_declare_tied_ref");

    pub static __autoken_declare_tied_mut: CachedSymbol =
        CachedSymbol::new("__autoken_declare_tied_mut");

    // Legacy:
    pub static __autoken_tie_ref: CachedSymbol = CachedSymbol::new("__autoken_tie_ref");

    pub static __autoken_tie_mut: CachedSymbol = CachedSymbol::new("__autoken_tie_mut");

    pub static __autoken_tie_ref_shadow: CachedSymbol =
        CachedSymbol::new("__autoken_tie_ref_shadow");

    pub static __autoken_tie_mut_shadow: CachedSymbol =
        CachedSymbol::new("__autoken_tie_mut_shadow");
}
