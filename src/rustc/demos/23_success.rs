fn main() {
    loop {
        assume_no_alias(borrow_mutably::<u32>);
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

pub fn assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
    fn __autoken_assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
        f()
    }

    __autoken_assume_no_alias::<Res>(f)
}
