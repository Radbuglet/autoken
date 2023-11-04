fn main() {
    a();
    b();
}

fn a() {
    borrow_mutably::<u32>();
}

fn b() {
    unborrow_mutably::<u32>();
    borrow_mutably::<u32>();
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
