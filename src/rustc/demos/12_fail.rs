fn main() {
    let foo: fn() = || {
        borrow_mutably::<u32>();
    };
    let bar: fn() = || {
        borrow_mutably::<i32>();
    };

    borrow_mutably::<u32>();
    borrow_mutably::<i32>();

    foo();
}

fn borrow_mutably<T: ?Sized>() {
    fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}
