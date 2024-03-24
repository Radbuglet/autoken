#![no_std]
#![feature(tuple_trait)]

use core::marker::{PhantomData, Tuple};

// === Absorb === //

pub unsafe fn absorb_borrows_except<T: Tuple, R>(f: impl FnOnce() -> R) -> R {
    #[doc(hidden)]
    #[allow(clippy::extra_unused_type_parameters)]
    pub fn __autoken_absorb_borrows_except<T: Tuple, R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    __autoken_absorb_borrows_except::<T, R>(f)
}

pub fn borrows_all<'a, T: Tuple>() -> BorrowsAllExcept<'a, T> {
    tie!('a => except T);

    BorrowsAllExcept::acquire()
}

pub struct BorrowsAllExcept<'a, T: Tuple = ()> {
    _ty: PhantomData<fn() -> &'a T>,
}

impl<'a, T: Tuple> BorrowsAllExcept<'a, T> {
    pub fn acquire() -> Self {
        tie!('a => except T);

        Self { _ty: PhantomData }
    }

    pub fn absorb<R>(&mut self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_borrows_except::<T, R>(f) }
    }
}

// === Tie === //

#[doc(hidden)]
pub fn __autoken_declare_tied_ref<I, T: ?Sized>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_mut<I, T: ?Sized>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_all_except<I, T: Tuple>() {}

#[doc(hidden)]
pub fn borrow_counterpoint() {
    struct Counterpoint;

    __autoken_declare_tied_mut::<(), Counterpoint>();
}

#[macro_export]
macro_rules! tie {
    ($lt:lifetime => ref $ty:ty) => {{
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::__autoken_declare_tied_ref::<AutokenLifetimeDefiner<'_>, $ty>();
    }};
    ($lt:lifetime => mut $ty:ty) => {
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::__autoken_declare_tied_mut::<AutokenLifetimeDefiner<'_>, $ty>();
    };
    ($lt:lifetime => except $ty:ty) => {
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::borrow_counterpoint();
        $crate::__autoken_declare_tied_all_except::<AutokenLifetimeDefiner<'_>, $ty>();
    };
    (ref $ty:ty) => {{
        $crate::__autoken_declare_tied_ref::<(), $ty>();
    }};
    (mut $ty:ty) => {
        $crate::__autoken_declare_tied_mut::<(), $ty>();
    };
    (except $ty:ty) => {
        $crate::__autoken_declare_tied_all_except::<(), $ty>();
    };
}
