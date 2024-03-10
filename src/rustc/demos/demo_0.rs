fn main() {
    let mut foo = Vec::new();
    let bar = &mut foo;
    let baz = 3;
    bar.push(&baz);
}

fn __autoken_helper_limit_to<'a, T: ?Sized>(v: &'a T, _: &'a impl ?Sized) -> &'a T {
    v
}

fn __autoken_tie_to_token<T: ?Sized>(v: &T) -> &T {
    v
}
