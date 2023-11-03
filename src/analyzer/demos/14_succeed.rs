fn main() {
    let foo: &dyn Demo = &();
}

trait Demo {
    fn do_something(&self);
}

impl Demo for () {
    fn do_something(&self) {
        kaz();
    }
}

fn kaz() {}
