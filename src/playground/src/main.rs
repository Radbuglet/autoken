#![feature(arbitrary_self_types)]

pub mod util;

use util::obj::Obj;

fn main() {
    let a = Obj::new(LinkedList::new(1));
    let b = Obj::new(LinkedList::new(2));
    let c = Obj::new(LinkedList::new(3));

    a.insert_right(b);
    b.insert_right(c);
    a.iter_right(|val| {
        *val += 1;
        dbg!(*val);
    });
}

pub struct LinkedList<T: 'static> {
    prev: Option<Obj<Self>>,
    next: Option<Obj<Self>>,
    value: T,
}

impl<T: 'static> LinkedList<T> {
    pub fn new(value: T) -> Self {
        Self {
            prev: None,
            next: None,
            value,
        }
    }

    pub fn insert_right(mut self: Obj<Self>, mut node: Obj<Self>) {
        if let Some(mut next) = node.next {
            next.prev = Some(node);
        }
        node.next = self.next;
        node.prev = Some(self);
        self.next = Some(node);
    }

    pub fn iter_right(mut self: Obj<Self>, mut f: impl FnMut(&mut T)) {
        f(&mut self.value);

        while let Some(next) = self.next {
            self = next;
            f(&mut self.value);
        }
    }
}
