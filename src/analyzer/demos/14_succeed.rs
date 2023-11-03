fn main() {
    let foo: &dyn Demo = &();
    foo.do_something();

    let bar: Box<dyn Demo> = Box::new(());
    bar.do_something();
}

trait Demo {
    fn do_something(&self) {}
}

impl Demo for () {
    fn do_something(&self) {}
}
