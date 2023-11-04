fn main() {
    let foo: &dyn Demo<_> = &();
    foo.do_something();
}

trait Demo<T> {
    fn do_something(&self) -> T;
}

impl Demo<f32> for () {
    fn do_something(&self) -> f32 {
        kaz();
        3.14
    }
}

fn kaz() {}
