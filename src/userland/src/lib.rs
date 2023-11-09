#![no_std]

use core::{cmp::Ordering, fmt, marker::PhantomData, mem};

// === Version Validation === //

include!(concat!(env!("OUT_DIR"), "/version_check.rs"));

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

pub const fn assert_mutably_borrowable<T: ?Sized>() {
    borrow_mutably::<T>();
    unborrow_mutably::<T>();
}

pub const fn assert_immutably_borrowable<T: ?Sized>() {
    borrow_immutably::<T>();
    unborrow_immutably::<T>();
}

pub fn assume_no_alias_in_many<T, Res>(f: impl FnOnce() -> Res) -> Res
where
    T: ?Sized + tuple_sealed::Tuple,
{
    #[allow(clippy::extra_unused_type_parameters)] // Used by autoken
    fn __autoken_assume_no_alias_in<T: ?Sized, Res>(f: impl FnOnce() -> Res) -> Res {
        f()
    }

    __autoken_assume_no_alias_in::<T, Res>(f)
}

pub fn assume_no_alias_in<T: ?Sized, Res>(f: impl FnOnce() -> Res) -> Res {
    assume_no_alias_in_many::<(T,), Res>(f)
}

pub fn assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
    fn __autoken_assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
        f()
    }

    __autoken_assume_no_alias::<Res>(f)
}

pub fn assume_black_box<T>(f: impl FnOnce() -> T) -> T {
    fn __autoken_assume_black_box<T>(f: impl FnOnce() -> T) -> T {
        f()
    }

    __autoken_assume_black_box::<T>(f)
}

pub struct Nothing<'a> {
    __autoken_nothing_type_field_indicator: PhantomData<&'a ()>,
}

mod tuple_sealed {
    pub trait Tuple {}

    impl<A: ?Sized> Tuple for (A,) {}

    impl<A, B: ?Sized> Tuple for (A, B) {}

    impl<A, B, C: ?Sized> Tuple for (A, B, C) {}

    impl<A, B, C, D: ?Sized> Tuple for (A, B, C, D) {}

    impl<A, B, C, D, E: ?Sized> Tuple for (A, B, C, D, E) {}

    impl<A, B, C, D, E, F: ?Sized> Tuple for (A, B, C, D, E, F) {}

    impl<A, B, C, D, E, F, G: ?Sized> Tuple for (A, B, C, D, E, F, G) {}

    impl<A, B, C, D, E, F, G, H: ?Sized> Tuple for (A, B, C, D, E, F, G, H) {}

    impl<A, B, C, D, E, F, G, H, I: ?Sized> Tuple for (A, B, C, D, E, F, G, H, I) {}

    impl<A, B, C, D, E, F, G, H, I, J: ?Sized> Tuple for (A, B, C, D, E, F, G, H, I, J) {}

    impl<A, B, C, D, E, F, G, H, I, J, K: ?Sized> Tuple for (A, B, C, D, E, F, G, H, I, J, K) {}

    impl<A, B, C, D, E, F, G, H, I, J, K, L: ?Sized> Tuple for (A, B, C, D, E, F, G, H, I, J, K, L) {}
}

// === Guaranteed RAII === //

// MutableBorrow
pub struct MutableBorrow<T: ?Sized> {
    _ty: PhantomData<fn() -> T>,
}

impl<T: ?Sized> MutableBorrow<T> {
    pub const fn new() -> Self {
        borrow_mutably::<T>();
        Self { _ty: PhantomData }
    }

    pub fn downgrade(self) -> PotentialMutableBorrow<T> {
        PotentialMutableBorrow(self)
    }

    pub fn downgrade_ref(&self) -> &PotentialMutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    pub fn downgrade_mut(&mut self) -> &mut PotentialMutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    pub fn loan(&mut self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    pub fn assume_no_alias_loan(&self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    pub fn assume_no_alias_clone(&self) -> Self {
        assume_no_alias(|| Self::new())
    }

    pub fn strip_lifetime_analysis(self) -> MutableBorrow<Nothing<'static>> {
        drop(self);
        MutableBorrow::new()
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

    pub fn downgrade(self) -> PotentialImmutableBorrow<T> {
        PotentialImmutableBorrow(self)
    }

    pub fn downgrade_ref(&self) -> &PotentialImmutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    pub fn downgrade_mut(&mut self) -> &mut PotentialImmutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    pub const fn loan(&self) -> ImmutableBorrow<Nothing<'_>> {
        ImmutableBorrow::new()
    }

    pub fn strip_lifetime_analysis(self) -> ImmutableBorrow<Nothing<'static>> {
        drop(self);
        ImmutableBorrow::new()
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

// === Potential RAII === //

// PotentialMutableBorrow
#[repr(transparent)]
pub struct PotentialMutableBorrow<T: ?Sized>(MutableBorrow<T>);

impl<T: ?Sized> PotentialMutableBorrow<T> {
    pub fn new() -> Self {
        assume_no_alias(|| Self(MutableBorrow::new()))
    }

    pub fn loan(&mut self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    pub fn assume_no_alias_loan(&self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    pub fn strip_lifetime_analysis(self) -> PotentialMutableBorrow<Nothing<'static>> {
        drop(self);
        PotentialMutableBorrow::new()
    }
}

impl<T: ?Sized> Default for PotentialMutableBorrow<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> fmt::Debug for PotentialMutableBorrow<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PotentialMutableBorrow")
            .finish_non_exhaustive()
    }
}

impl<T: ?Sized> Eq for PotentialMutableBorrow<T> {}

impl<T: ?Sized> PartialEq for PotentialMutableBorrow<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T: ?Sized> Ord for PotentialMutableBorrow<T> {
    fn cmp(&self, _other: &Self) -> Ordering {
        Ordering::Equal
    }
}

impl<T: ?Sized> PartialOrd for PotentialMutableBorrow<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Clone for PotentialMutableBorrow<T> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

// PotentialImmutableBorrow
#[repr(transparent)]
pub struct PotentialImmutableBorrow<T: ?Sized>(ImmutableBorrow<T>);

impl<T: ?Sized> PotentialImmutableBorrow<T> {
    pub fn new() -> Self {
        assume_no_alias(|| Self(ImmutableBorrow::new()))
    }

    pub const fn loan(&self) -> ImmutableBorrow<Nothing<'_>> {
        ImmutableBorrow::new()
    }

    pub fn strip_lifetime_analysis(self) -> PotentialImmutableBorrow<Nothing<'static>> {
        drop(self);
        PotentialImmutableBorrow::new()
    }
}

impl<T: ?Sized> Default for PotentialImmutableBorrow<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> fmt::Debug for PotentialImmutableBorrow<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PotentialImmutableBorrow")
            .finish_non_exhaustive()
    }
}

impl<T: ?Sized> Eq for PotentialImmutableBorrow<T> {}

impl<T: ?Sized> PartialEq for PotentialImmutableBorrow<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T: ?Sized> Ord for PotentialImmutableBorrow<T> {
    fn cmp(&self, _other: &Self) -> Ordering {
        Ordering::Equal
    }
}

impl<T: ?Sized> PartialOrd for PotentialImmutableBorrow<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Clone for PotentialImmutableBorrow<T> {
    fn clone(&self) -> Self {
        Self::new()
    }
}
