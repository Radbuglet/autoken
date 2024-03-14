fn main() {
    let hehe = whee::<u32>();
    woo();
}

fn woo<'a>() {
    let a = whee::<u32>();
    let b = whee::<f32>();
    let _ = a;
}

fn whee<'a, T>() -> &'a f64 {
    __autoken_declare_tied_mut::<0, T>();
    &4.2
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}

fn __autoken_declare_tied_mut<const LT_ID: u32, T: ?Sized>() {}
