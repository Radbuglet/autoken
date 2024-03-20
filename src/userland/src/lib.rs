#![no_std]
#![feature(tuple_trait)]

use core::marker::{PhantomData, Tuple};

// === BorrowsAllExcept === //

pub struct BorrowsAllExcept<T: Tuple = ()> {
    __autoken_borrows_all_except_field_indicator: PhantomData<fn() -> T>,
}

impl<T: Tuple> BorrowsAllExcept<T> {
    pub const fn new() -> Self {
        Self {
            __autoken_borrows_all_except_field_indicator: PhantomData,
        }
    }
}

impl<T: Tuple> Default for BorrowsAllExcept<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Tuple> Copy for BorrowsAllExcept<T> {}

impl<T: Tuple> Clone for BorrowsAllExcept<T> {
    fn clone(&self) -> Self {
        *self
    }
}

// === Tie === //

#[doc(hidden)]
pub fn __autoken_declare_tied_ref<I, T: ?Sized>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_mut<I, T: ?Sized>() {}

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
    (ref $ty:ty) => {{
        $crate::__autoken_declare_tied_ref::<(), $ty>();
    }};
    (mut $ty:ty) => {
        $crate::__autoken_declare_tied_mut::<(), $ty>();
    };
}
