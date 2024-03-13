fn main() {}

fn whee<'a>() {
    __autoken_declare_tied_ref::<{ u32::MAX }, i32>();
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}
