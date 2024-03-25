#![no_std]
#![allow(clippy::missing_safety_doc)]
#![feature(tuple_trait)]
#![feature(sync_unsafe_cell)]

use core::{
    cell::SyncUnsafeCell,
    fmt,
    marker::{PhantomData, Tuple},
};

// === TokenCell === //

pub struct TokenCell<T: ?Sized, L: ?Sized = T> {
    _ty: PhantomData<fn(L) -> L>,
    value: SyncUnsafeCell<T>,
}

impl<T: Default, L> Default for TokenCell<T, L> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: ?Sized, L> TokenCell<T, L> {
    pub const fn new(value: T) -> Self
    where
        T: Sized,
    {
        Self {
            _ty: PhantomData,
            value: SyncUnsafeCell::new(value),
        }
    }

    pub fn get<'a>(&'a self) -> &'a T {
        tie!('a => ref L);
        unsafe { &*self.value.get() }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn get_mut<'a>(&'a self) -> &'a mut T {
        tie!('a => mut L);
        unsafe { &mut *self.value.get() }
    }

    pub fn into_inner(self) -> T
    where
        T: Sized,
    {
        self.value.into_inner()
    }
}

// === Absorb === //

pub unsafe fn absorb_only_ref<T: Tuple, R>(f: impl FnOnce() -> R) -> R {
    #[doc(hidden)]
    #[allow(clippy::extra_unused_type_parameters)]
    pub fn __autoken_absorb_only_ref<T: Tuple, R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    __autoken_absorb_only_ref::<T, R>(f)
}

pub unsafe fn absorb_only_mut<T: Tuple, R>(f: impl FnOnce() -> R) -> R {
    #[doc(hidden)]
    #[allow(clippy::extra_unused_type_parameters)]
    pub fn __autoken_absorb_only_ref<T: Tuple, R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    __autoken_absorb_only_ref::<T, R>(f)
}

pub struct BorrowsMut<'a, T: Tuple> {
    _ty: PhantomData<fn() -> (&'a (), T)>,
}

impl<T: Tuple> fmt::Debug for BorrowsMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowsMut").finish_non_exhaustive()
    }
}

impl<'a, T: Tuple> BorrowsMut<'a, T> {
    pub unsafe fn new_unchecked() -> Self {
        Self { _ty: PhantomData }
    }

    pub fn acquire() -> Self {
        tie!('a => mut many T);
        Self { _ty: PhantomData }
    }

    pub fn absorb<R>(&mut self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_only_mut::<T, R>(f) }
    }

    pub fn absorb_ref<R>(&self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_only_ref::<T, R>(f) }
    }
}

pub struct BorrowsRef<'a, T: Tuple> {
    _ty: PhantomData<fn() -> (&'a (), T)>,
}

impl<T: Tuple> fmt::Debug for BorrowsRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowsRef").finish_non_exhaustive()
    }
}

impl<'a, T: Tuple> BorrowsRef<'a, T> {
    pub unsafe fn new_unchecked() -> Self {
        Self { _ty: PhantomData }
    }

    pub fn acquire() -> Self {
        tie!('a => ref many T);
        Self { _ty: PhantomData }
    }

    pub fn absorb<R>(&self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_only_ref::<T, R>(f) }
    }
}

// === Tie === //

#[doc(hidden)]
pub fn __autoken_declare_tied_ref<I, T: Tuple>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_mut<I, T: Tuple>() {}

#[doc(hidden)]
pub fn __autoken_declare_tied_all_except<I, T: Tuple>() {}

#[macro_export]
macro_rules! tie {
    ($lt:lifetime => ref many $ty:ty) => {{
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::__autoken_declare_tied_ref::<AutokenLifetimeDefiner<'_>, $ty>();
    }};
    ($lt:lifetime => mut many $ty:ty) => {
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::__autoken_declare_tied_mut::<AutokenLifetimeDefiner<'_>, $ty>();
    };
    ($lt:lifetime => ref $ty:ty) => {{
        $crate::tie!($lt => ref many ($ty,));
    }};
    ($lt:lifetime => mut $ty:ty) => {
        $crate::tie!($lt => mut many ($ty,));
    };
    (ref many $ty:ty) => {{
        $crate::__autoken_declare_tied_ref::<(), $ty>();
    }};
    (mut many $ty:ty) => {
        $crate::__autoken_declare_tied_mut::<(), $ty>();
    };
    (ref $ty:ty) => {{
        $crate::tie!(ref ($ty,));
    }};
    (mut $ty:ty) => {
        $crate::tie!(mut ($ty,));
    };
}
