trait Whee {
    fn woo();
}

impl<T> Whee for T {
    fn woo() {
        borrow_mutably::<T>();
        borrow_immutably::<T>();
        unborrow_immutably::<T>();
        unborrow_mutably::<T>();
    }
}

fn main() {
    demo::<f32>();
}

fn demo<W: Whee>() {
    W::woo();
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
