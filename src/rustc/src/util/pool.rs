use std::{
    cell::Cell,
    fmt,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};

pub struct Pool<T> {
    free: Cell<Vec<T>>,
}

impl<T> fmt::Debug for Pool<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pool").finish_non_exhaustive()
    }
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Pool<T> {
    pub const fn new() -> Self {
        Self {
            free: Cell::new(Vec::new()),
        }
    }

    pub fn grab(&self) -> PoolGuard<'_, T>
    where
        T: PoolElem,
    {
        let mut free = self.free.take();
        if let Some(value) = free.pop() {
            self.free.set(free);
            PoolGuard {
                pool: self,
                value: ManuallyDrop::new(value),
            }
        } else {
            self.free.set(free);

            PoolGuard {
                pool: self,
                value: ManuallyDrop::new(T::default()),
            }
        }
    }
}

pub trait PoolElem: Default {
    fn reset(&mut self);
}

pub struct PoolGuard<'a, T: PoolElem> {
    pool: &'a Pool<T>,
    value: ManuallyDrop<T>,
}

impl<T: PoolElem> Deref for PoolGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: PoolElem> DerefMut for PoolGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: PoolElem> Drop for PoolGuard<'_, T> {
    fn drop(&mut self) {
        self.value.reset();

        let mut free = self.pool.free.take();
        free.push(unsafe { ManuallyDrop::take(&mut self.value) });
        self.pool.free.set(free);
    }
}
