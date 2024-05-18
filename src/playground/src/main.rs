fn main() {}

fn tie<'a>() -> &'a () {
    autoken::tie!('a => mut ());
    &()
}

fn huh<'a, R>(f: impl FnOnce(&'a ()) -> R) -> R {
    autoken::tie!('a => mut ());
    f(tie())
}

fn wuh() {
    let a = huh(|v| v);
    let _ = huh(|v| v);
    let _ = a;
}
