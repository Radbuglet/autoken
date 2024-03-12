use rustc_data_structures::sorted_map::SortedMap;
use rustc_hir::{FieldDef, Node, OwnerId, OwnerNodes, Pat, PatKind, VariantData};
use rustc_index::IndexVec;
use rustc_middle::{arena::ArenaAllocatable, ty::TyCtxt};
use scopeguard::{guard, ScopeGuard};

// === rewrite_ref === //

pub trait BindTwoLts<'a, 'b> {}

impl<'a, 'b, T: ?Sized> BindTwoLts<'a, 'b> for T {}

pub fn rewrite_ref<'targ, 'tcx, T, C>(
    tcx: TyCtxt<'tcx>,
    target: &'targ mut &'tcx T,
) -> ScopeGuard<T, impl FnOnce(T) + BindTwoLts<'targ, 'tcx>>
where
    T: Clone + ArenaAllocatable<'tcx, C>,
{
    let mirror = target.clone();
    guard(mirror, move |mirror| {
        *target = tcx.arena.alloc(mirror);
    })
}

// === `clone_hir_rewrite_owners` === //

#[derive(Debug, Copy, Clone)]
pub struct OwnerRewrite {
    pub new_owner: OwnerId,
    pub old_owner: OwnerId,
}

impl OwnerRewrite {
    pub fn apply(self, id: &mut OwnerId) {
        assert_eq!(*id, self.old_owner);
        *id = self.new_owner;
    }
}

// TODO: This is very much *not* finished.
pub fn clone_hir_rewrite_owners<'tcx>(
    tcx: TyCtxt<'tcx>,
    new_owner: OwnerRewrite,
    original: &OwnerNodes<'tcx>,
) -> &'tcx OwnerNodes<'tcx> {
    Box::leak(Box::new(OwnerNodes {
        opt_hash_including_bodies: None,
        nodes: IndexVec::from_iter(original.nodes.iter().map(|node| {
            let mut node = *node;

            match &mut node.node {
                Node::Param(node) => {
                    let mut node = rewrite_ref(tcx, node);
                    new_owner.apply(&mut node.hir_id.owner);
                    rewrite_pat(tcx, new_owner, &mut rewrite_ref(tcx, &mut node.pat));
                }
                Node::Item(node) => new_owner.apply(&mut rewrite_ref(tcx, node).owner_id),
                Node::ForeignItem(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).owner_id);
                }
                Node::TraitItem(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).owner_id);
                }
                Node::ImplItem(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).owner_id);
                }
                Node::Variant(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Field(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::AnonConst(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::ConstBlock(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Expr(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::ExprField(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Stmt(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::PathSegment(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Ty(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::TypeBinding(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::TraitRef(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_ref_id.owner);
                }
                Node::Pat(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::PatField(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Arm(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Block(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Local(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Ctor(node) => match &mut *rewrite_ref(tcx, node) {
                    VariantData::Struct { fields, .. } => {
                        *fields = rewrite_field_defs(tcx, new_owner, fields);
                    }
                    VariantData::Tuple(fields, hir_id, _) => {
                        *fields = rewrite_field_defs(tcx, new_owner, fields);
                        new_owner.apply(&mut hir_id.owner);
                    }
                    VariantData::Unit(hir_id, _) => {
                        new_owner.apply(&mut hir_id.owner);
                    }
                },
                Node::Lifetime(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::GenericParam(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Crate(_node) => {
                    // (this node only contains child IDs)
                }
                Node::Infer(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::WhereBoundPredicate(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::ArrayLenInfer(node) => {
                    new_owner.apply(&mut rewrite_ref(tcx, node).hir_id.owner);
                }
                Node::Err(_node) => {
                    // (this node is just a span)
                }
            }

            node
        })),
        bodies: SortedMap::from_iter(original.bodies.iter().map(|(local_id, body)| {
            let local_id = *local_id;
            let body = *body;

            (local_id, body)
        })),
    }))
}

fn rewrite_field_defs<'tcx>(
    tcx: TyCtxt<'tcx>,
    new_owner: OwnerRewrite,
    original: &[FieldDef<'tcx>],
) -> &'tcx [FieldDef<'tcx>] {
    tcx.arena
        .alloc_from_iter(original.iter().copied().map(|mut field| {
            new_owner.apply(&mut field.hir_id.owner);
            field
        }))
}

fn rewrite_pat<'tcx>(tcx: TyCtxt<'tcx>, new_owner: OwnerRewrite, pat: &mut Pat<'tcx>) {
    new_owner.apply(&mut pat.hir_id.owner);

    match &mut pat.kind {
        PatKind::Wild => {}
        PatKind::Binding(_, hir_id, _, sub_pat) => {
            new_owner.apply(&mut hir_id.owner);

            if let Some(sub_pat) = sub_pat {
                rewrite_pat(tcx, new_owner, &mut rewrite_ref(tcx, sub_pat));
            }
        }
        PatKind::Struct(_, _, _) => todo!(),
        PatKind::TupleStruct(_, _, _) => todo!(),
        PatKind::Or(_) => todo!(),
        PatKind::Never => todo!(),
        PatKind::Path(_) => todo!(),
        PatKind::Tuple(_, _) => todo!(),
        PatKind::Box(_) => todo!(),
        PatKind::Ref(_, _) => todo!(),
        PatKind::Lit(_) => todo!(),
        PatKind::Range(_, _, _) => todo!(),
        PatKind::Slice(_, _, _) => todo!(),
        PatKind::Err(_) => todo!(),
    }
}
