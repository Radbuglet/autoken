fn main() {
    // whee::<u32, i32>();
    whee::<u32, u32>();
}

fn whee<T, V>() {
    //     let f = || {};
    //
    //     f();
    //
    //     let sg = scopeguard::guard((), |()| {
    //         woo::<T>();
    //     });
    //     let foo = woo::<T>();
    //     let _ = woo::<V>();
    //     drop(sg);
    //     let _ = foo;

    let foo = woo::<(T, V)>();
    // gah_wrap::<T>();
    gah(|| {
        woo::<(T, V)>();
        woo::<(V, T)>();
        woo::<f32>();
    });
    let _ = foo;
}

fn gah_wrap<T>() {
    gah(|| {
        woo::<T>();
        woo::<f32>();
    });
}

fn gah<F: FnOnce()>(f: F) {
    f();
}

fn woo<'a, T>() -> &'a () {
    autoken::tie!('a => mut T);
    &()
}
