fn main() {
    woo();
}

fn woo() {
    borrow_mutably::<u32>();
    unborrow_mutably::<u32>();
    war();
}

fn war() {
    woo();
}

fn borrow_mutably<T: ?Sized>() {
    fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}

fn borrow_immutably<T: ?Sized>() {
    fn __autoken_borrow_immutably<T: ?Sized>() {}

    __autoken_borrow_immutably::<T>();
}

fn unborrow_mutably<T: ?Sized>() {
    fn __autoken_unborrow_mutably<T: ?Sized>() {}

    __autoken_unborrow_mutably::<T>();
}

fn unborrow_immutably<T: ?Sized>() {
    fn __autoken_unborrow_immutably<T: ?Sized>() {}

    __autoken_unborrow_immutably::<T>();
}
