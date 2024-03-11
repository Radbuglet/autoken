fn main() {}

struct Foo;

impl Foo {
    fn whee(&self) {}
}

impl Default for Foo {
    fn default() -> Self {
        Self
    }
}

fn waz() {
    maz::<i32>();
    __autoken_declare_tied_ref::<str>();
}

fn maz<T>() {
    __autoken_declare_tied_mut::<str>();
    __autoken_declare_tied_ref::<T>();
    maz::<u32>();
}

fn kaz<F: FnOnce()>(f: F) {
    (f)();
}

fn __autoken_declare_tied_ref<T: ?Sized>() {}

fn __autoken_declare_tied_mut<T: ?Sized>() {}
