fn main() {
    woo();
}

fn woo() {
    __autoken_borrow_mutably();
    __autoken_unborrow_mutably();

    war();
}

fn war() {
    __autoken_borrow_immutably();
    woo();
    __autoken_unborrow_immutably();
}

fn __autoken_borrow_mutably() {}

fn __autoken_borrow_immutably() {}

fn __autoken_unborrow_mutably() {}

fn __autoken_unborrow_immutably() {}
