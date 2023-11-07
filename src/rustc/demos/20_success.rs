fn main() {
    let foo = assume_black_box(|| {
        let f = || loop {
            borrow_mutably::<u32>();
        };
        f as fn()
    });

    foo();

    assume_black_box(|| loop {
        borrow_mutably::<u32>();
    });
}

pub fn assume_black_box<T>(f: impl FnOnce() -> T) -> T {
    fn __autoken_assume_black_box<T>(f: impl FnOnce() -> T) -> T {
        f()
    }

    __autoken_assume_black_box::<T>(f)
}

fn borrow_mutably<T: ?Sized>() {
    fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}
