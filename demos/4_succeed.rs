trait Whee {
    fn woo();
}

impl Whee for () {
    fn woo() {
        borrow_mutably();
        unborrow_mutably();
    }
}

fn main() {
    demo::<()>();
}

fn demo<W: Whee>() {
    W::woo();
}

fn borrow_mutably() {
    fn __autoken_borrow_mutably() {}

    __autoken_borrow_mutably();
}

fn borrow_immutably() {
    fn __autoken_borrow_immutably() {}

    __autoken_borrow_immutably();
}

fn unborrow_mutably() {
    fn __autoken_unborrow_mutably() {}

    __autoken_unborrow_mutably();
}

fn unborrow_immutably() {
    fn __autoken_unborrow_immutably() {}

    __autoken_unborrow_immutably();
}
