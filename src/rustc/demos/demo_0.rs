fn main() {
    woo();
}

fn woo<'a>() {
    let a = whee();
    let b = whee();
    // let _ = a;
}

fn whee<'a>() -> &'a f64 {
    __autoken_declare_tied_mut::<0, f64>();
    &4.2
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}

fn __autoken_declare_tied_mut<const LT_ID: u32, T: ?Sized>() {}
