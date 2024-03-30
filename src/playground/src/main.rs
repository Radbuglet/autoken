fn main() {
    whee::<u32>();
}

fn whee<T>() {
    let f = || {};

    f();
    woo::<T>();
}

fn woo<T>() {
    autoken::tie!(ref T);
}
