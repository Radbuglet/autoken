fn main() {
    let foo: &dyn Fn() = &|| {
        borrow_mutably::<u32>();
        borrow_mutably::<u32>();
    };
    foo();
}

pub const fn borrow_mutably<T: ?Sized>() {
    const fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}
