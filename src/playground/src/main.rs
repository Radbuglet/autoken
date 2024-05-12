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
