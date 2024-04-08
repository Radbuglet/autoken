use autoken::{Diff, Mut, Ref};

fn main() {
    let v = woo();
    woo();
    let _ = v;
}

fn woo<'a>() -> &'a () {
    autoken::tie!('a => set Diff<Mut<u32>, Ref<i32>>);
    &()
}
