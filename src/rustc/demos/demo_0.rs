fn main() {
    woo();
}

fn woo<'a>() {
    whee::<f32>();
}

fn whee<'a, T>() -> &'a T {
    __autoken_declare_tied_ref::<0, u32>();
    loop {}
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}
