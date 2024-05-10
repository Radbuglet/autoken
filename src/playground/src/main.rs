fn main() {}

fn bar() {
    let mut foo = Vec::new();
    loop {
        foo.push(tie());
    }
}

fn baz() {
    let foo = tie();
    let bar = tie();
    let _ = foo;
}

fn tie<'a>() -> &'a () {
    autoken::tie!('a => mut u32);
    &()
}
