fn main() {
    // woo();
}

fn woo<'a>() {
    whee();
}

fn whee<'a>() -> &'a f32 {
    __autoken_declare_tied_ref::<0, f32>();
    &4.2
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}
