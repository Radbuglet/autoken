fn main() {
    whee::<u32, i32>();
}

fn whee<T, V>() {
    let f = || {};

    f();

    let sg = scopeguard::guard((), |()| {
        woo::<T>();
    });
    let foo = woo::<T>();
    let _ = woo::<V>();
    drop(sg);
    let _ = foo;
}

fn woo<'a, T>() -> &'a () {
    autoken::tie!('a => mut T);
    &()
}
