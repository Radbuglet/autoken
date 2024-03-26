#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::{cell::UnsafeCell, fmt, marker::PhantomData};

// === TokenCell === //

pub struct TokenCell<T: ?Sized, L: ?Sized = T> {
    _ty: PhantomData<fn(L) -> L>,
    value: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send, L: ?Sized> Send for TokenCell<T, L> {}
unsafe impl<T: ?Sized + Sync, L: ?Sized> Sync for TokenCell<T, L> {}

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
            value: UnsafeCell::new(value),
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

// === TokenSet === //

mod sealed {
    pub trait TokenSet {}
}

pub trait TokenSet: sealed::TokenSet {}

// Ref
pub struct Ref<T: ?Sized> {
    __autoken_ref_ty_marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> TokenSet for Ref<T> {}
impl<T: ?Sized> sealed::TokenSet for Ref<T> {}

// Mut
pub struct Mut<T: ?Sized> {
    __autoken_mut_ty_marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> TokenSet for Mut<T> {}
impl<T: ?Sized> sealed::TokenSet for Mut<T> {}

// DowngradeRef
pub struct DowngradeRef<T: TokenSet> {
    __autoken_downgrade_ty_marker: PhantomData<fn() -> T>,
}

impl<T: TokenSet> TokenSet for DowngradeRef<T> {}
impl<T: TokenSet> sealed::TokenSet for DowngradeRef<T> {}

// Diff
pub struct Diff<A: TokenSet, B: TokenSet> {
    __autoken_diff_ty_marker: PhantomData<fn() -> (A, B)>,
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

pub unsafe fn absorb_only<T: TokenSet, R>(f: impl FnOnce() -> R) -> R {
    #[doc(hidden)]
    #[allow(clippy::extra_unused_type_parameters)]
    pub fn __autoken_absorb_only<T: TokenSet, R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    __autoken_absorb_only::<T, R>(f)
}

pub struct BorrowsMut<'a, T: TokenSet> {
    _ty: PhantomData<fn() -> (&'a (), T)>,
}

impl<T: TokenSet> fmt::Debug for BorrowsMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowsMut").finish_non_exhaustive()
    }
}

impl<'a, T: TokenSet> BorrowsMut<'a, T> {
    pub unsafe fn new_unchecked() -> Self {
        Self { _ty: PhantomData }
    }

    pub fn acquire() -> Self {
        tie!('a => set T);
        Self { _ty: PhantomData }
    }

    pub fn absorb<R>(&mut self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_only::<T, R>(f) }
    }

    pub fn absorb_ref<R>(&self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_only::<DowngradeRef<T>, R>(f) }
    }
}

pub struct BorrowsRef<'a, T: TokenSet> {
    _ty: PhantomData<fn() -> (&'a (), T)>,
}

impl<T: TokenSet> fmt::Debug for BorrowsRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowsRef").finish_non_exhaustive()
    }
}

impl<'a, T: TokenSet> BorrowsRef<'a, T> {
    pub unsafe fn new_unchecked() -> Self {
        Self { _ty: PhantomData }
    }

    pub fn acquire() -> Self {
        tie!('a => set DowngradeRef<T>);
        Self { _ty: PhantomData }
    }

    pub fn absorb<R>(&self, f: impl FnOnce() -> R) -> R {
        unsafe { absorb_only::<DowngradeRef<T>, R>(f) }
    }
}

// === Tie === //

#[doc(hidden)]
pub fn __autoken_declare_tied<I, T: TokenSet>() {}

#[macro_export]
macro_rules! tie {
    ($lt:lifetime => set $ty:ty) => {{
        struct AutokenLifetimeDefiner<$lt> {
            _v: &$lt(),
        }

        let _: &$lt() = &();

        $crate::__autoken_declare_tied::<AutokenLifetimeDefiner<'_>, $ty>();
    }};
    ($lt:lifetime => mut $ty:ty) => {
        $crate::tie!($lt => set $crate::Mut<$ty>);
    };
    ($lt:lifetime => ref $ty:ty) => {
        $crate::tie!($lt => set $crate::Ref<$ty>);
    };
    (set $ty:ty) => {{
        $crate::__autoken_declare_tied::<(), $ty>();
    }};
    (mut $ty:ty) => {
        $crate::tie!(set $crate::Mut<$ty>);
    };
    (ref $ty:ty) => {
        $crate::tie!(set $crate::Ref<$ty>);
    };
}
