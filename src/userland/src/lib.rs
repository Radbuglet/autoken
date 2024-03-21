#![no_std]
#![feature(tuple_trait)]

// === Tie === //

use core::marker::Tuple;

#[doc(hidden)]
pub fn __autoken_declare_tied_ref<I, T: ?Sized>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_mut<I, T: ?Sized>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_all_except<I, T: Tuple>() {}

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
