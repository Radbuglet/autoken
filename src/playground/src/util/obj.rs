use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

pub struct Obj<T: 'static> {
    value: &'static UnsafeCell<T>,
}

impl<T> Copy for Obj<T> {}

impl<T> Clone for Obj<T> {
    fn clone(&self) -> Self {
        *self
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
        __autoken_declare_tied_ref::<0, T>();
        unsafe { &*self.value.get() }
    }
}

impl<T> DerefMut for Obj<T> {
    #[allow(clippy::needless_lifetimes)]
    fn deref_mut<'autoken_0>(&'autoken_0 mut self) -> &'autoken_0 mut Self::Target {
        __autoken_declare_tied_mut::<0, T>();
        unsafe { &mut *self.value.get() }
    }
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}

fn __autoken_declare_tied_mut<const LT_ID: u32, T: ?Sized>() {}
