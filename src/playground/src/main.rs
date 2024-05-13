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

fn hehe<'a>() -> &'a u32 {
    map(tie())
}

fn map(_v: &()) -> &u32 {
    &3
}

fn tie<'a>() -> &'a () {
    autoken::tie!('a => mut ());
    &()
}
