use autoken::Mut;

fn main() {
    unsafe {
        autoken::absorb::<Mut<()>, _>(|| {
            what(&());
        });
    }
}

fn what(foo: impl Foo) {
    let a = foo.what();
    let _ = foo.what();
    let _ = a;
}

trait Foo {
    type Output;

    fn what(&self) -> Self::Output;
}

impl<'a> Foo for &'a () {
    type Output = &'a ();

    fn what(&self) -> Self {
        autoken::tie!('a => mut ());
        &&()
    }
}
