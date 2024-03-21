#![no_std]
#![feature(tuple_trait)]

// === BorrowsAllExcept === //

pub type BorrowsAllExcept<T = ()> = [borrows_all_except::BorrowsAllExcept<T>; 0];

mod borrows_all_except {
    use core::marker::{PhantomData, Tuple};

    pub struct BorrowsAllExcept<T: Tuple> {
        __autoken_borrows_all_except_field_indicator: PhantomData<fn() -> T>,
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
