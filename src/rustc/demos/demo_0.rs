fn main() {
    let hehe = whee::<u32>();
    woo();
}

fn woo<'a>() {
    let a = whee::<u32>();
    let b = whee::<f32>();
    let hehe = Whee(&4);
    let haha = &*hehe;
    let _ = a;
}

fn whee<'autoken_42, T>() -> &'autoken_42 f64 {
    __autoken_declare_tied_mut::<42, T>();
    &4.2
}

struct Whee<'a>(&'a u32);

impl<'a> std::ops::Deref for Whee<'a> {
    type Target = f64;

    fn deref<'autoken_24>(&'autoken_24 self) -> &'autoken_24 f64 {
        __autoken_declare_tied_mut::<24, f32>();
        &4.2
    }
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}

fn __autoken_declare_tied_mut<const LT_ID: u32, T: ?Sized>() {}
