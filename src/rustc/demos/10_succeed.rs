fn main() {
    wee();
}

fn wee() {
    borrow_immutably::<i32>();
    woo();
    unborrow_immutably::<i32>();
}

fn woo() {
    wee();
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
