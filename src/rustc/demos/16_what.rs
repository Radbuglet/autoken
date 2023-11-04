fn main() {
    borrow_mutably::<u32>();
    borrow_mutably::<u32>();
    borrow_mutably::<u32>();
    borrow_mutably::<u32>();
    borrow_mutably::<u32>();
}

fn borrow_mutably<T: ?Sized>() {
    fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}
