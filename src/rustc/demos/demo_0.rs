fn main() {
    woo();
}

fn woo<'a>() {
    whee()
}

fn whee<'a>() {
    __autoken_declare_tied_ref::<0, u32>();
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}
