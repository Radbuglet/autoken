fn main() {
    let a = tie();
    let _ = tie();
    let _ = a;
}

fn bar() {
    let mut foo = Vec::new();
    loop {
        foo.push(tie());
    }
}

fn tie<'a>() -> &'a () {
    autoken::tie!('a => mut ());
    &()
}
