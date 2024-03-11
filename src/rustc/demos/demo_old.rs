fn main() {
    let mut foo = Vec::<i32>::new();
    let a = __autoken_tie_mut(&mut foo);

    let mut bar = Vec::<i32>::new();
    let b = __autoken_tie_mut(&mut bar);
    let _ = a;
}

fn whee(hehe: (), v: &mut u32) {
    let foo = __autoken_tie_mut(v);
    let bar = __autoken_tie_mut(v);
    // let _ = foo;
}

fn __autoken_tie_ref_shadow<'a, T: ?Sized>(v: &'a T, _: &'a ()) -> &'a T {
    v
}

fn __autoken_tie_mut_shadow<'a, T: ?Sized>(v: &'a mut T, _: &'a mut ()) -> &'a mut T {
    v
}

fn __autoken_tie_ref<T: ?Sized>(v: &T) -> &T {
    v
}

fn __autoken_tie_mut<T: ?Sized>(v: &mut T) -> &mut T {
    v
}

struct Foo;

impl Foo {
    fn whee(&self) {}
}

impl Default for Foo {
    fn default() -> Self {
        Self
    }
}
