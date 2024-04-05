fn main() {
    whee::<u32, i32>();
    // whee::<u32, u32>();
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
    woo::<(V, T)>();
    gah_wrap::<T>();
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

fn gah_wrap2<F: FnOnce()>(f: F) {
    let hehe = woo::<u32>();
    // gah(f);
    woo::<u32>();
    let _ = hehe;
}

fn hehe() {
    let a = woo::<u32>();
}

fn woo<'a, T>() -> &'a () {
    autoken::tie!('a => mut T);
    &()
}
