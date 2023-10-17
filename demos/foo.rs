fn main() {
    let foo = Foo;
    let woo_fn = if dummy() { woo } else { waz };
    woo_fn();

    std::process::exit(1);
}

fn woo() {}

fn waz() {}

fn dummy() -> bool {
    std::hint::black_box(true)
}

struct Foo;

impl Drop for Foo {
    fn drop(&mut self) {
        waz();
    }
}
