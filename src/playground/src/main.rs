#![feature(arbitrary_self_types)]
#![feature(ptr_metadata)]
#![feature(unsize)]

use std::{collections::HashSet, sync::Arc};

use util::obj::{DynObj, Obj};

pub mod util;

fn main() {
    let comp2 = Obj::new(Component::new(|_| 2));
    let comp = Obj::new(Component::new(move |_| {
        comp2.render();
        eprintln!("Computed!");
        3
    }));

    dbg!(comp.render());
    dbg!(comp.render());
    comp.mark_dep(comp2);
    comp2.mark_dirty();
    dbg!(comp.render());
}

pub trait AnyComponent {
    fn mark_dirty(&self);
}

pub struct Component<T: 'static> {
    dependents: HashSet<DynObj<dyn AnyComponent>>,
    cache: Option<T>,
    renderer: Arc<dyn Fn(Obj<Self>) -> T>,
}

impl<T> Component<T> {
    pub fn new(renderer: impl 'static + Fn(Obj<Self>) -> T) -> Self {
        Self {
            dependents: HashSet::default(),
            cache: None,
            renderer: Arc::new(renderer),
        }
    }

    pub fn render<'autoken_0>(mut self: Obj<Self>) -> &'autoken_0 T {
        autoken::tie!('autoken_0 => ref Self);

        if self.cache.is_none() {
            let rendered = (self.renderer.clone())(self);
            self.cache = Some(rendered);
        }

        Obj::get(self).cache.as_ref().unwrap()
    }

    pub fn mark_dep<V>(self: Obj<Self>, mut other: Obj<Component<V>>) {
        other.dependents.insert(DynObj::new(self));
    }
}

impl<T> AnyComponent for Obj<Component<T>> {
    fn mark_dirty(&self) {
        let mut me = *self;
        me.cache = None;
        for dep in std::mem::take(&mut me.dependents) {
            dep.mark_dirty();
        }
    }
}
