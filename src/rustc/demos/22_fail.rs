fn main() {
    let mut foo = 0;
    while foo < 5 {
        foo += 1;
    }

    if foo == 0 {
        return;
    }

    borrow_mutably::<u32>();
    borrow_mutably::<u32>();
}

pub const fn borrow_mutably<T: ?Sized>() {
    const fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}
