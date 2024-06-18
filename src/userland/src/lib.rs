#![allow(rustdoc::redundant_explicit_links)] // (cargo-rdme needs this)
//! A rust-lang compiler tool adding support for zero-cost borrow-aware context passing.
//!
//! ```rust
//! use autoken::{cap, CapTarget};
//!
//! cap! {
//!     pub MyCap = Vec<u32>;
//! }
//!
//! fn main() {
//!     let mut my_vec = vec![1, 2, 3, 4];
//!
//!     MyCap::provide(&mut my_vec, || {
//!         do_something();
//!     });
//! }
//!
//! fn do_something() {
//!     with_indirection();
//! }
//!
//! fn with_indirection() {
//!     let my_vec = cap!(ref MyCap);
//!     let first_three = &my_vec[0..3];
//!     cap!(mut MyCap).push(5);
//!     eprintln!("The first three elements were {first_three:?}");
//! }
//! ```
//!
//! ```plain_text
//! error: conflicting borrows on token MyCap
//!   --> src/main.rs:23:5
//!    |
//! 20 |     let my_vec = cap!(ref MyCap);
//!    |                  --------------- value first borrowed immutably
//! ...
//! 23 |     cap!(mut MyCap).push(5);
//!    |     ^^^^^^^^^^^^^^^ value later borrowed mutably
//!    |
//!    = help: first borrow originates from Borrows::<Mut<MyCap>>::acquire_ref::<'_>
//!    = help: later borrow originates from Borrows::<Mut<MyCap>>::acquire_mut::<'_>
//! ```
//!
//! ## Installation
//!
//! AuToken is both a custom compiler plugin called `cargo-autoken` and a regular cargo crate called
//! `autoken` whose documentation you are currently reading. It is possible to compile projects
//! using `autoken` with a stock `rustc` compiler since `cargo-autoken` only modifies the compiler's
//! validation logic—not its code generation logic. However, this will be terribly unsound since no
//! validation will actually occur!
//!
//! To fix that, let's actually install and use `cargo-autoken`.
//!
//! Currently, AuToken must be installed directly through the [AuToken git repository](https://github.com/radbuglet/autoken).
//! To install the tool, clone the repository, enter its root directory, and run...
//!
//! ```plain_text
//! cargo install -Z bindeps --path src/cargo
//! ```
//!
//! Now that `cargo-autoken` is installed, let's set up a project.
//!
//! ### Project Setup
//!
//! AuToken requires a very specific version of the Rust compiler to work. Let's pin our project to
//! that version by creating a `rust-toolchain.toml` file in your project's root directory.
//!
//! ```toml
//! [toolchain]
//! channel = "nightly-2024-03-10"
//! ```
//!
//! Next, let's install the `autoken` crate from the [AuToken git repository](https://github.com/radbuglet/autoken).
//! We can do this by adding the following line to your `Cargo.toml` file:
//!
//! ```toml
//! autoken = { git = "https://github.com/Radbuglet/autoken.git", rev = "<optional pinned revision>" }
//! ```
//!
//! Finally, let's create a script to compile and run the project. You can any task runner you like,
//! such as `Makefile` or [`just`](https://github.com/casey/just). This example script is a `Makefile`
//! —you can create an equivalent `Justfile` by removing the `.PHONY` directive.
//!
//! ```makefile
//! .PHONY: run, run-unchecked
//!
//! run:
#![doc = "\tcargo autoken check"]
#![doc = "\tcargo run"]
//!
//! run-unchecked:
#![doc = "\tcargo run"]
//! ```
//!
//! And that it! Have fun!
//!
//! ## Basic Usage
//!
//! The easiest way to use AuToken is through the [`cap!`](crate::cap) macro. `cap!` allows users to
//! define, provide, and fetch a new implicitly-passed context item sometimes called a "capability."
//!
//! Let's start by defining our first capability:
//!
//! ```rust
//! autoken::cap! {
//!     pub MyCap = Vec<u32>;
//! }
//! ```
//!
//! From there, we can define functions that depend on that value.
//!
//! ```rust
//! # autoken::cap! {
//! #     pub MyCap = Vec<u32>;
//! # }
//! fn add_number(value: u32) {
//!     autoken::cap!(mut MyCap).push(value);
//!     eprintln!("The list is now {:?}, skip a few, {:?}", first_n_numbers(2), last_number());
//! }
//!
//! fn last_number() -> Option<u32> {
//!     autoken::cap!(ref MyCap).last().copied()
//! }
//!
//! fn first_n_numbers<'a>(count: usize) -> &'a [u32] {
//!     // Declares the fact that `'a` depends on a borrow of `MyCap`.
//!     autoken::tie!('a => ref MyCap);
//!
//!     &autoken::cap!(ref MyCap)[0..count]
//! }
//! ```
//!
//! To be able to call those functions, we need to inject an actual `Vec<u32>` instance into the `MyCap`
//! context. We can do so using one final form of `cap!`:
//!
//! ```rust
//! # autoken::cap! {
//! #     pub MyCap = Vec<u32>;
//! # }
//! #
//! # fn add_number(value: u32) {
//! #     autoken::cap!(mut MyCap).push(value);
//! #     eprintln!("The list is now {:?}, skip a few, {:?}", first_n_numbers(2), last_number());
//! # }
//! #
//! # fn last_number() -> Option<u32> {
//! #     autoken::cap!(ref MyCap).last().copied()
//! # }
//! #
//! # fn first_n_numbers<'a>(count: usize) -> &'a [u32] {
//! #     // Declares the fact that `'a` depends on a borrow of `MyCap`.
//! #     autoken::tie!('a => ref MyCap);
//! #
//! #     &autoken::cap!(ref MyCap)[0..count]
//! # }
//! fn main() {
//!     autoken::cap! {
//!         MyCap: &mut vec![1, 2, 3]
//!     =>
//!         eprintln!("The last number is {:?}", last_number());
//!         add_number(4);
//!         eprintln!("Our four numbers are {:?}", first_n_numbers(4));
//!     }
//! }
//! ```
//!
//! AuToken can inject context through any static call site, even if it's a `trait` method or even
//! an externally-defined function. For example, this works because we're "passing" the `MyCap`
//! reference through the closure every time it's called...
//!
//! ```rust
//! autoken::cap! {
//!     pub MyCap = Vec<u32>;
//! }
//!
//! fn call_two(a: impl FnOnce(), b: impl FnOnce()) {
//!     a();
//!     b();
//! }
//!
//! fn demo() {
//!     call_two(
//!         || autoken::cap!(mut MyCap).push(3),
//!         || autoken::cap!(mut MyCap).push(4),
//!     );
//! }
//! ```
//!
//! ...while this code fails to compile because both closures need to capture `my_cap`:
//!
//! ```rust
//! # fn call_two(a: impl FnOnce(), b: impl FnOnce()) {
//! #     a();
//! #     b();
//! # }
//! fn demo() {
//!     let mut my_values = vec![1, 2];
//!
//!     call_two(
//!         || my_values.push(3),
//!         || my_values.push(4),
//!     );
//! }
//! ```
//!
//! ```plain_text
//! error[E0499]: cannot borrow `my_values` as mutable more than once at a time
//!   --> src/lib.rs:5:9
//!    |
//! 10 |     call_two(
//!    |     -------- first borrow later used by call
//! 11 |         || my_values.push(3),
//!    |         -- --------- first borrow occurs due to use of `my_values` in closure
//!    |         |
//!    |         first mutable borrow occurs here
//! 12 |         || my_values.push(4),
//!    |         ^^ --------- second borrow occurs due to use of `my_values` in closure
//!    |         |
//!    |         second mutable borrow occurs here
//! ```
//!
//! What AuToken cannot inject through, however, is dynamically dispatched function calls. AuToken
//! assumes that every function or trait member which has been unsized depends on nothing from its
//! caller. Hence, if you try to unsize a function which expects a value to be in its context, the
//! line will refuse to compile:
//!
//! ```rust
//! autoken::cap! {
//!     pub MyCap = u32;
//! }
//!
//! fn increment_counter() {
//!     *autoken::cap!(mut MyCap) += 1;
//! }
//!
//! fn demo() {
//!     // Calling `increment_counter` statically is fine, assuming `MyCap` is in the context.
//!     increment_counter();
//!
//!     // ...but unsizing `increment_counter` is not!
//!     let my_func: fn() = increment_counter;
//! }
//! ```
//!
//! ```plain_text
//! error: cannot unsize this function because it borrows unabsorbed tokens
//!   --> src/main.rs:11:25
//!    |
//! 11 |     let my_func: fn() = increment_counter;
//!    |                         ^^^^^^^^^^^^^^^^^
//!    |
//!    = note: uses &mut MyCap.
//!            
//! note: increment_counter was unsized
//!   --> src/main.rs:4:1
//!    |
//! 4  | fn increment_counter() {
//!    | ^^^^^^^^^^^^^^^^^^^^^^
//! ```
//!
//! ## Advanced Usage
//!
//! **To-Do:** Document the lower-level API.
//!
//! ## Limitations
//!
//! **To-Do:** Document tool limitations (i.e. input parameters aren't supported yet, you can't upgrade
//! to newer versions of `rustc`, you probably shouldn't publish crates written with AuToken, the
//! compiler is a bit slow because it duplicates a ton of work).
//!

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
