fn main() {}

fn works<'a>() -> impl Iterator<Item = &'a ()> {
    let v = (0..1).map(move |_| &());
    v
}

fn previously_broke<'a>() -> impl Iterator<Item = &'a ()> {
    (0..1).map(move |_| &())
}

fn still_breaks<'a>() -> impl Iterator<Item = &'a ()> {
    previously_broke()
}

fn woo() {
    whee(|| {
        tie();
    });
}

fn whee(f: impl FnOnce()) {
    let guard = tie();
    f();
    let _ = guard;
}

fn hehe(_dummy: &u32) -> (&u32, &str) {
    map(tie())
}

fn hehe2<'b, 'c>() -> (&'b u32, &'c str) {
    map(tie())
}

fn map(_v: &()) -> (&u32, &str) {
    (&3, "hi!!")
}

fn tie<'a>() -> &'a () {
    autoken::tie!('a => mut ());
    &()
}
