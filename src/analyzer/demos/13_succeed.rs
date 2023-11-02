fn main() {
    let foo: &dyn Fn() = &|| {};
    foo();
}
