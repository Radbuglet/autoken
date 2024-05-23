#![allow(clippy::missing_safety_doc)]

use std::{fmt, marker::PhantomData};

// === TokenSet === //

mod sealed {
    pub trait TokenSet {}
}

pub trait TokenSet: sealed::TokenSet {}

// Ref
pub struct Ref<T: ?Sized> {
    // N.B. we intentionally include `T` as a type in this structure to ensure that it inherits all
    // the auto-traits of the type.
    __autoken_ref_ty_marker: PhantomData<T>,
}

impl<T: ?Sized> TokenSet for Ref<T> {}
impl<T: ?Sized> sealed::TokenSet for Ref<T> {}

// Mut
pub struct Mut<T: ?Sized> {
    // N.B. we intentionally include `T` as a type in this structure to ensure that it inherits all
    // the auto-traits of the type.
    __autoken_mut_ty_marker: PhantomData<T>,
}

impl<T: ?Sized> TokenSet for Mut<T> {}
impl<T: ?Sized> sealed::TokenSet for Mut<T> {}

// DowngradeRef
pub struct DowngradeRef<T: TokenSet> {
    // N.B. we intentionally include `T` as a type in this structure to ensure that it inherits all
    // the auto-traits of the type.
    __autoken_downgrade_ty_marker: PhantomData<T>,
}

impl<T: TokenSet> TokenSet for DowngradeRef<T> {}
impl<T: TokenSet> sealed::TokenSet for DowngradeRef<T> {}

// Diff
pub struct Diff<A: TokenSet, B: TokenSet> {
    // N.B. we intentionally include `T` as a type in this structure to ensure that it inherits all
    // the auto-traits of the type.
    __autoken_diff_ty_marker: PhantomData<(A, B)>,
}

impl<A: TokenSet, B: TokenSet> TokenSet for Diff<A, B> {}
impl<A: TokenSet, B: TokenSet> sealed::TokenSet for Diff<A, B> {}

// Union
impl TokenSet for () {}
impl sealed::TokenSet for () {}

macro_rules! impl_union {
    () => {};
    ($first:ident $($rest:ident)*) => {
        impl<$first: TokenSet $(, $rest: TokenSet)*> TokenSet for ($first, $($rest,)*) {}
        impl<$first: TokenSet $(, $rest: TokenSet)*> sealed::TokenSet for ($first, $($rest,)*) {}

        impl_union!($($rest)*);
    };
}

impl_union!(T1 T2 T3 T4 T5 T6 T7 T8 T9 T10 T11 T12 T13 T14 T15 T16 T17 T18 T19 T20 T21 T22 T23 T24 T25 T26 T27 T28 T29 T30 T31 T32);

// === Absorb === //

pub unsafe fn absorb<T: TokenSet, R>(f: impl FnOnce() -> R) -> R {
    #[doc(hidden)]
    #[allow(clippy::extra_unused_type_parameters)]
    pub fn __autoken_absorb_only<T: TokenSet, R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    __autoken_absorb_only::<T, R>(f)
}

pub type BorrowsOne<T> = Borrows<Mut<T>>;

pub struct Borrows<T: TokenSet> {
    // N.B. we intentionally include `T` as a type in this structure to ensure that it inherits all
    // the auto-traits of the type.
    _ty: PhantomData<T>,
}

impl<T: TokenSet> fmt::Debug for Borrows<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Borrows").finish_non_exhaustive()
    }
}

impl<T: TokenSet> Borrows<T> {
    pub unsafe fn new_unchecked() -> Self {
        Self { _ty: PhantomData }
    }

    pub fn acquire_ref<'a>() -> &'a Self {
        tie!('a => set DowngradeRef<T>);
        &Self { _ty: PhantomData }
    }

    pub fn acquire_mut<'a>() -> &'a mut Self {
        tie!('a => set T);
        unsafe { &mut *(0x1 as *mut Self) }
    }

    pub fn absorb<R>(&mut self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb::<T, R>(f) }
    }

    pub fn absorb_ref<R>(&self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb::<DowngradeRef<T>, R>(f) }
    }
}

// === Tie === //

#[doc(hidden)]
pub mod tie_macro_internals {
    pub fn __autoken_declare_tied<I, T: crate::TokenSet, IsUnsafe>() {}
}

#[macro_export]
macro_rules! tie {
    // Safe variants
    ($lt:lifetime => set $ty:ty) => {{
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::tie_macro_internals::__autoken_declare_tied::<AutokenLifetimeDefiner<'_>, $ty, ()>();
    }};
    ($lt:lifetime => mut $ty:ty) => {
        $crate::tie!($lt => set $crate::Mut<$ty>);
    };
    ($lt:lifetime => ref $ty:ty) => {
        $crate::tie!($lt => set $crate::Ref<$ty>);
    };
    (set $ty:ty) => {{
        $crate::tie_macro_internals::__autoken_declare_tied::<(), $ty, ()>();
    }};
    (mut $ty:ty) => {
        $crate::tie!(set $crate::Mut<$ty>);
    };
    (ref $ty:ty) => {
        $crate::tie!(set $crate::Ref<$ty>);
    };

    // Unsafe variants
    (unsafe $lt:lifetime => set $ty:ty) => {{
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::tie_macro_internals::__autoken_declare_tied::<AutokenLifetimeDefiner<'_>, $ty, ((),)>();
    }};
    (unsafe $lt:lifetime => mut $ty:ty) => {
        $crate::tie!(unsafe $lt => set $crate::Mut<$ty>);
    };
    (unsafe $lt:lifetime => ref $ty:ty) => {
        $crate::tie!(unsafe $lt => set $crate::Ref<$ty>);
    };
    (unsafe set $ty:ty) => {{
        $crate::tie_macro_internals::__autoken_declare_tied::<(), $ty, ((),)>();
    }};
    (unsafe mut $ty:ty) => {
        $crate::tie!(unsafe set $crate::Mut<$ty>);
    };
    (unsafe ref $ty:ty) => {
        $crate::tie!(unsafe set $crate::Ref<$ty>);
    };
}

// === `cap!` === //

#[doc(hidden)]
pub mod cap_macro_internals {
    pub use {
        crate::BorrowsOne,
        std::{cell::Cell, ops::FnOnce, ptr::null_mut, thread::LocalKey, thread_local},
    };

    pub struct CxScope {
        tls: &'static LocalKey<Cell<*mut ()>>,
        prev: *mut (),
    }

    impl CxScope {
        pub fn new(tls: &'static LocalKey<Cell<*mut ()>>, new_ptr: *mut ()) -> Self {
            tls.set(new_ptr);

            Self {
                tls,
                prev: tls.get(),
            }
        }
    }

    impl Drop for CxScope {
        fn drop(&mut self) {
            self.tls.set(self.prev);
        }
    }
}

pub trait CapTarget<T> {
    fn provide<R>(value: T, f: impl FnOnce() -> R) -> R;
}

#[macro_export]
macro_rules! cap {
    ( $($ty:ty: $expr:expr),*$(,)? => $($body:tt)* ) => {{
        #[allow(unused_mut)]
        let mut f = || { $($body)* };

        $(
            #[allow(unused_mut)]
            let mut f = || <$ty as $crate::CapTarget<_>>::provide($expr, f);
        )*

        f()
    }};
    (ref $ty:ty) => {
        <$ty>::get($crate::cap_macro_internals::BorrowsOne::acquire_ref(), |v| v)
    };
    (mut $ty:ty) => {
        <$ty>::get_mut($crate::cap_macro_internals::BorrowsOne::acquire_mut(), |v| v)
    };
    (ref $ty:ty => $name:ident in $out:expr) => {
        <$ty>::get($crate::cap_macro_internals::BorrowsOne::acquire_ref(), |$name| $out)
    };
    (mut $ty:ty => $name:ident in $out:expr) => {
        <$ty>::get_mut($crate::cap_macro_internals::BorrowsOne::acquire_mut(), |$name| $out)
    };
    ($(
        $(#[$attr:meta])*
        $vis:vis $name:ident$(<$($lt:lifetime),* $(,)?>)? = $ty:ty;
    )*) => {$(
        $(#[$attr])*
        #[non_exhaustive]
        $vis struct $name;

        impl $name {
            fn tls() -> &'static $crate::cap_macro_internals::LocalKey<$crate::cap_macro_internals::Cell<*mut ()>> {
                $crate::cap_macro_internals::thread_local! {
                    static VALUE: $crate::cap_macro_internals::Cell<*mut ()> = const {
                        $crate::cap_macro_internals::Cell::new($crate::cap_macro_internals::null_mut())
                    };
                }

                &VALUE
            }

            $vis fn get<'out, R: 'out>(
                _borrows: &'out $crate::cap_macro_internals::BorrowsOne<$name>,
                f: impl $(for<$($lt,)*>)? $crate::cap_macro_internals::FnOnce(&'out $ty) -> R,
            ) -> R {
                f(Self::tls().with(|ptr| unsafe { &*ptr.get().cast() }))
            }

            $vis fn get_mut<'out, R: 'out>(
                _borrows: &'out mut $crate::cap_macro_internals::BorrowsOne<$name>,
                f: impl $(for<$($lt,)*>)? $crate::cap_macro_internals::FnOnce(&'out mut $ty) -> R,
            ) -> R {
                f(Self::tls().with(|ptr| unsafe { &mut *ptr.get().cast() }))
            }
        }

        impl<'out $($(, $lt)*)?> $crate::CapTarget<&'out mut $ty> for $name {
            fn provide<R>(value: &'out mut $ty, f: impl $crate::cap_macro_internals::FnOnce() -> R) -> R {
                let _scope = $crate::cap_macro_internals::CxScope::new(Self::tls(), value as *mut $ty as *mut ());

                unsafe {
                    $crate::absorb::<$crate::Mut<Self>, R>(f)
                }
            }
        }

        impl<'out $($(, $lt)*)?> $crate::CapTarget<&'out $ty> for $name {
            fn provide<R>(value: &'out $ty, f: impl $crate::cap_macro_internals::FnOnce() -> R) -> R {
                let _scope = $crate::cap_macro_internals::CxScope::new(Self::tls(), value as *const $ty as *const () as *mut ());

                fn tier<'a>() -> &'a () {
                    $crate::tie!('a => mut $name);
                    &()
                }

                unsafe {
                    $crate::absorb::<$crate::Mut<Self>, R>(|| {
                        let tier = tier();
                        let res = $crate::absorb::<$crate::Ref<Self>, R>(f);
                        let _ = tier;
                        res
                    })
                }
            }
        }
    )*};
}
