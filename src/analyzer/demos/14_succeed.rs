fn main() {
    let foo: &dyn Demo<_> = &();
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
