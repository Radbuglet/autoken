#![no_std]
#![feature(tuple_trait)]

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
