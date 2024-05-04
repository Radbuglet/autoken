use autoken::Mut;

fn main() {
    unsafe {
        autoken::absorb::<Mut<()>, _>(|| {
            what(());
        });
    }
}

fn what(foo: impl Foo) {
    foo.what();
}

trait Foo: 'static {
    type Output<'a>;

    fn what<'a, 'b>(&'a self) -> &'b Self::Output<'a>;
}

impl Foo for () {
    type Output<'b> = &'b ();

    fn what<'b, 'c>(&'b self) -> &'c &'b () {
        autoken::tie!('b => mut ());
        &&()
    }
}
