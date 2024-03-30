fn main() {
    whee::<u32>();
}

fn whee<T>() {
    let f = || {};

    f();

    let foo = woo::<T>();
    let _ = woo::<T>();
    let _ = foo;
}

fn woo<'a, T>() -> &'a () {
    autoken::tie!('a => mut T);
    &()
}
