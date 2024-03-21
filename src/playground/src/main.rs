#![feature(arbitrary_self_types)]
#![feature(ptr_metadata)]
#![feature(unsize)]

use std::{collections::HashSet, sync::Arc};

use autoken::{borrows_all, BorrowsAllExcept};
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

    comp2.mark_dirty(&mut borrows_all());

    dbg!(comp.render());
}

pub trait AnyComponent {
    fn mark_dirty(&self, token: &mut BorrowsAllExcept<'_>);
}

pub struct Component<T: 'static> {
    dependents: HashSet<DynObj<dyn AnyComponent>>,
    cache: Option<T>,
    renderer: Arc<dyn Fn(&mut BorrowsAllExcept, Obj<Self>) -> T>,
}

impl<T> Component<T> {
    pub fn new(renderer: impl 'static + Fn(Obj<Self>) -> T) -> Self {
        Self {
            dependents: HashSet::default(),
            cache: None,
            renderer: Arc::new(move |token, obj| token.absorb(|| renderer(obj))),
        }
    }

    pub fn render<'a>(mut self: Obj<Self>) -> &'a T {
        autoken::tie!('a => ref Self);

        if self.cache.is_none() {
            let rendered = (self.renderer.clone())(&mut borrows_all(), self);
            self.cache = Some(rendered);
        }

        Obj::get(self).cache.as_ref().unwrap()
    }

    pub fn mark_dep<V>(self: Obj<Self>, mut other: Obj<Component<V>>) {
        other.dependents.insert(DynObj::new(self));
    }
}

impl<T> AnyComponent for Obj<Component<T>> {
    fn mark_dirty(&self, token: &mut BorrowsAllExcept<'_>) {
        token.absorb(|| {
            let mut me = *self;
            me.cache = None;
            for dep in std::mem::take(&mut me.dependents) {
                dep.mark_dirty(&mut BorrowsAllExcept::acquire());
            }
        });
    }
}

pub mod wgpu_bug_repro {
    pub struct Error {}

    pub trait UncapturedErrorHandler: Fn(Error) + Send + 'static {}

    impl<T> UncapturedErrorHandler for T where T: Fn(Error) + Send + 'static {}

    fn default_error_handler(err: Error) {}

    pub fn demo() {
        let foo: Box<dyn UncapturedErrorHandler> = Box::new(default_error_handler);
    }
}
