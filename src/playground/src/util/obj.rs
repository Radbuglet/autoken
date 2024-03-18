use core::fmt;
use std::{
    cell::UnsafeCell,
    hash,
    marker::Unsize,
    ops::{Deref, DerefMut},
    ptr::{from_raw_parts, metadata, NonNull, Pointee},
};

// === Obj === //

#[repr(transparent)]
pub struct Obj<T: 'static> {
    value: &'static UnsafeCell<T>,
}

impl<T> Obj<T> {
    pub fn get<'autoken_0>(self) -> &'autoken_0 T {
        autoken::tie!('autoken_0 => ref T);
        unsafe { &*self.value.get() }
    }

    pub fn get_mut<'autoken_0>(self) -> &'autoken_0 mut T {
        autoken::tie!('autoken_0 => mut T);
        unsafe { &mut *self.value.get() }
    }
}

impl<T> Copy for Obj<T> {}

impl<T> Clone for Obj<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> hash::Hash for Obj<T> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.value.get().hash(state);
    }
}

impl<T> Eq for Obj<T> {}

impl<T> PartialEq for Obj<T> {
    fn eq(&self, other: &Obj<T>) -> bool {
        self.value.get() == other.value.get()
    }
}

impl<T> Obj<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Box::leak(Box::new(UnsafeCell::new(value))),
        }
    }
}

impl<T> Deref for Obj<T> {
    type Target = T;

    #[allow(clippy::needless_lifetimes)]
    fn deref<'autoken_0>(&'autoken_0 self) -> &'autoken_0 Self::Target {
        autoken::tie!('autoken_0 => ref T);
        unsafe { &*self.value.get() }
    }
}

impl<T> DerefMut for Obj<T> {
    #[allow(clippy::needless_lifetimes)]
    fn deref_mut<'autoken_0>(&'autoken_0 mut self) -> &'autoken_0 mut Self::Target {
        autoken::tie!('autoken_0 => mut T);
        unsafe { &mut *self.value.get() }
    }
}

pub struct DynObj<T: ?Sized> {
    pointee: NonNull<()>,
    metadata: <T as Pointee>::Metadata,
}

impl<T: ?Sized> fmt::Debug for DynObj<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynObj").finish_non_exhaustive()
    }
}

impl<T: ?Sized> Copy for DynObj<T> {}

impl<T: ?Sized> Clone for DynObj<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> hash::Hash for DynObj<T> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.pointee.hash(state);
    }
}

impl<T: ?Sized> Eq for DynObj<T> {}

impl<T: ?Sized> PartialEq for DynObj<T> {
    fn eq(&self, other: &DynObj<T>) -> bool {
        self.pointee == other.pointee
    }
}

impl<T: ?Sized> DynObj<T> {
    pub fn new<V>(value: Obj<V>) -> Self
    where
        Obj<V>: Unsize<T>,
    {
        let metadata = metadata(&value as &T);

        Self {
            pointee: NonNull::from(value.value).cast(),
            metadata,
        }
    }
}

impl<T: ?Sized> Deref for DynObj<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            &*from_raw_parts(
                &self.pointee as *const NonNull<()> as *const (),
                self.metadata,
            )
        }
    }
}
