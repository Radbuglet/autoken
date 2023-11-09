fn main() {
    let lock1 = Lock::new();
    let lock2 = assume_no_alias_in_many::<(Lock,), _>(|| Lock::new());
    drop(lock1);
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

pub fn assume_no_alias_in_many<T: ?Sized, Res>(f: impl FnOnce() -> Res) -> Res {
    #[allow(clippy::extra_unused_type_parameters)] // Used by autoken
    fn __autoken_assume_no_alias_in_many<T: ?Sized, Res>(f: impl FnOnce() -> Res) -> Res {
        f()
    }

    __autoken_assume_no_alias_in_many::<T, Res>(f)
}

pub fn assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
    fn __autoken_assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
        f()
    }

    __autoken_assume_no_alias::<Res>(f)
}
