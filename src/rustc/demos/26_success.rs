fn main() {
    for _ in 0..1 {
        borrow_mutably::<u32>();

        for _ in 1..2 {}

        unborrow_mutably::<u32>();
    }
}

pub const fn borrow_mutably<T: ?Sized>() {
    const fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}

pub const fn unborrow_mutably<T: ?Sized>() {
    const fn __autoken_unborrow_mutably<T: ?Sized>() {}

    __autoken_unborrow_mutably::<T>();
}
