#![allow(clippy::missing_safety_doc)]

use std::{
    collections::{HashMap, HashSet},
    fmt,
    mem::{self, ManuallyDrop},
    ops::{Deref, DerefMut},
};

// === Resettable === //

pub unsafe trait Resettable {
    fn reset_erasing_temporaries(&mut self);
}

unsafe impl<T> Resettable for Vec<T> {
    fn reset_erasing_temporaries(&mut self) {
        self.clear();
    }
}

unsafe impl<T, S: 'static> Resettable for HashSet<T, S> {
    fn reset_erasing_temporaries(&mut self) {
        self.clear();
    }
}

unsafe impl<K, V, S: 'static> Resettable for HashMap<K, V, S> {
    fn reset_erasing_temporaries(&mut self) {
        self.clear();
    }
}

// === Pooled === //

pub struct Pooled<T> {
    give_back: unsafe fn(*mut ()),
    value: ManuallyDrop<T>,
}

impl<T> fmt::Debug for Pooled<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pooled").finish_non_exhaustive()
    }
}

impl<T> Pooled<T> {
    pub const unsafe fn new(value: T, give_back: unsafe fn(*mut ())) -> Self {
        Self {
            give_back,
            value: ManuallyDrop::new(value),
        }
    }

    pub fn steal(mut me: Self) -> T {
        let value = unsafe { ManuallyDrop::take(&mut me.value) };
        mem::forget(me);
        value
    }
}

impl<T> Deref for Pooled<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Pooled<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T> Drop for Pooled<T> {
    fn drop(&mut self) {
        unsafe { (self.give_back)(&mut self.value as *mut ManuallyDrop<T> as *mut ()) };
    }
}

// === Macros === //

#[doc(hidden)]
pub mod pool_internals {
    pub use {
        super::{Pooled, Resettable},
        std::{
            cell::UnsafeCell,
            mem::{transmute, ManuallyDrop},
            thread_local,
            vec::Vec,
        },
    };

    pub trait HrtbHelper<I> {
        type Output;
    }

    pub type ChooseLeft<L, R> = <L as Id<R>>::Output;

    pub trait Id<T: ?Sized> {
        type Output: ?Sized;
    }

    impl<T: ?Sized, V: ?Sized> Id<V> for T {
        type Output = T;
    }
}

#[macro_export]
macro_rules! pool {
    ($(
        $(#[$attr:meta])*
        $vis:vis $name:ident $(<$($lt:lifetime),*$(,)?>)? => $ty:ty;
    )*) => {$(
        $(#[$attr])*
        $vis fn $name$(<$($lt),*>)?() -> $crate::util::pool::pool_internals::Pooled<$ty> {
            #[allow(unused)]
            type Erased = <
                // dyn for<'lt1, 'lt2, ...> HrtbHelper<(&'lt1 (), &'lt2 (), ...), Output = Ty<'lt1, 'lt2, ...>>
                dyn $(for<$($lt,)*>)? $crate::util::pool::pool_internals::HrtbHelper<(
                    $($(&$lt (),)*)?
                ), Output = $ty>
                as
                // HrtbHelper<(&'static (), &'static (), ...)>
                $crate::util::pool::pool_internals::HrtbHelper<($($(
                    $crate::util::pool::pool_internals::ChooseLeft<&'static (), for<$lt> fn(&$lt ())>,
                )*)?)>
            >::Output;

            $crate::util::pool::pool_internals::thread_local! {
                static POOL
                    : $crate::util::pool::pool_internals::UnsafeCell<
                        $crate::util::pool::pool_internals::Vec<Erased>
                    >
                = const {
                    $crate::util::pool::pool_internals::UnsafeCell::new(
                        $crate::util::pool::pool_internals::Vec::new(),
                    )
                };
            }

            POOL.with(|pool| {
                let value = unsafe { &mut *pool.get() }.pop().unwrap_or_default();

                unsafe {
                    $crate::util::pool::pool_internals::Pooled::new(
                        // Ty<'static, 'static> -> Ty<'lt1, 'lt2>
                        $crate::util::pool::pool_internals::transmute::<Erased, $ty>(value),
                        |value| {
                            let value = &mut *(
                                value as *mut $crate::util::pool::pool_internals::ManuallyDrop<$ty>
                                as *mut $crate::util::pool::pool_internals::ManuallyDrop<Erased>
                            );
                            let mut value = $crate::util::pool::pool_internals::ManuallyDrop::take(value);

                            $crate::util::pool::pool_internals::Resettable::reset_erasing_temporaries(&mut value);

                            POOL.with(|pool| {
                                (&mut *pool.get()).push(value);
                            });
                        },
                    )
                }
            })
        }
    )*};
}

pub use pool;
