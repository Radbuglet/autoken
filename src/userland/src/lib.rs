#![no_std]

use core::{cmp::Ordering, fmt, marker::PhantomData};

// === Primitives === //

pub const fn borrow_mutably<T: ?Sized>() {
    const fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}

pub const fn borrow_immutably<T: ?Sized>() {
    const fn __autoken_borrow_immutably<T: ?Sized>() {}

    __autoken_borrow_immutably::<T>();
}

pub const fn unborrow_mutably<T: ?Sized>() {
    const fn __autoken_unborrow_mutably<T: ?Sized>() {}

    __autoken_unborrow_mutably::<T>();
}

pub const fn unborrow_immutably<T: ?Sized>() {
    const fn __autoken_unborrow_immutably<T: ?Sized>() {}

    __autoken_unborrow_immutably::<T>();
}

// === RAII === //

// MutableBorrow
pub struct MutableBorrow<T: ?Sized> {
    _ty: PhantomData<fn() -> T>,
}

impl<T: ?Sized> MutableBorrow<T> {
    pub const fn new() -> Self {
        borrow_mutably::<T>();
        Self { _ty: PhantomData }
    }
}

impl<T: ?Sized> Default for MutableBorrow<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> fmt::Debug for MutableBorrow<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutableBorrow").finish_non_exhaustive()
    }
}

impl<T: ?Sized> Eq for MutableBorrow<T> {}

impl<T: ?Sized> PartialEq for MutableBorrow<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T: ?Sized> Ord for MutableBorrow<T> {
    fn cmp(&self, _other: &Self) -> Ordering {
        Ordering::Equal
    }
}

impl<T: ?Sized> PartialOrd for MutableBorrow<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Drop for MutableBorrow<T> {
    fn drop(&mut self) {
        unborrow_mutably::<T>();
    }
}

// ImmutableBorrow
pub struct ImmutableBorrow<T: ?Sized> {
    _ty: PhantomData<fn() -> T>,
}

impl<T: ?Sized> ImmutableBorrow<T> {
    pub const fn new() -> Self {
        borrow_immutably::<T>();
        Self { _ty: PhantomData }
    }
}

impl<T: ?Sized> Default for ImmutableBorrow<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> fmt::Debug for ImmutableBorrow<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImmutableBorrow").finish_non_exhaustive()
    }
}

impl<T: ?Sized> Eq for ImmutableBorrow<T> {}

impl<T: ?Sized> PartialEq for ImmutableBorrow<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T: ?Sized> Ord for ImmutableBorrow<T> {
    fn cmp(&self, _other: &Self) -> Ordering {
        Ordering::Equal
    }
}

impl<T: ?Sized> PartialOrd for ImmutableBorrow<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Clone for ImmutableBorrow<T> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl<T: ?Sized> Drop for ImmutableBorrow<T> {
    fn drop(&mut self) {
        unborrow_immutably::<T>();
    }
}