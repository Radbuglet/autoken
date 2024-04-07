fn main() {
    whee::<u32, u32>();
}

fn whee<T, V>() {
    let a = gah::<(T, V)>();
    let b = gah::<(V, T)>();
}

fn gah<'a, T>() -> &'a () {
    autoken::tie!('a => ref T);
    &()
}
