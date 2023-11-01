fn main() {
    let lock1 = Lock::new();
    dropeaux(lock1);
    let lock2 = Lock::new();
}

#[non_exhaustive]
pub struct Lock;

impl Lock {
    pub fn new() -> Self {
        borrow_mutably::<Self>();
        Self
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        unborrow_mutably::<Self>();
    }
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

fn dropeaux<T>(v: T) {
    let _ = v;
}
