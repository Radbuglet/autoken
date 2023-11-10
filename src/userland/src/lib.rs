#![warn(rustdoc::redundant_explicit_links)] // (cargo-rdme needs this)
//! A rust-lang static analysis tool to automatically check for runtime borrow violations.
//!
//! ```rust
//! use autoken::MutableBorrow;
//!
//! fn foo() {
//!     let _my_guard = MutableBorrow::<u32>::new();
//!
//!     // Autoken statically issues a warning here because we attempted to call a function which could
//!     // mutably borrow `u32` while we already have an active mutable borrow from `_my_guard`.
//!     bar();
//! }
//!
//! fn bar() {
//!     let _my_guard_2 = MutableBorrow::<u32>::new();
//! }
//! ```
//!
//! ## Checking Projects
//!
//! AuToken is a framework for adding static analysis of runtime borrowing to your crate. If you are
//! an end-user of a crate with integrations with AuToken and wish to check your project with the
//! tool, this is the section for you! If, instead, you're building a crate and wish to integrate with
//! AuToken, you should read on to the [Integrating AuToken](#integrating-autoken) section.
//!
//! If you wish to install from source, assuming your current working directory is the same as the
//! [repository](https://github.com/radbuglet/autoken)'s README, `cargo-autoken` can be installed
//! like so:
//!
//! ```bash
//! cargo install --path src/cargo
//! ```
//!
//! ...and executed in the crate you wish to validate like so:
//!
//! ```bash
//! cargo autoken check
//! ```
//!
//! Have fun!
//!
//! ## Ignoring False Positives
//!
//! AuToken is, by nature, very conservative. After all, its whole job is to ensure that only one
//! borrow of a given type exists at a given time, even if you're borrowing from several different
//! sources at once!
//!
//! ```rust
//! # use autoken::MutableBorrow;
//! # struct MyCell<T> {
//! #     _ty: core::marker::PhantomData<fn() -> T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(_value: T) -> Self {
//! #         Self { _ty: core::marker::PhantomData }
//! #     }
//! #
//! #     pub fn borrow_mut(&self) -> MutableBorrow<T> {
//! #         MutableBorrow::new()
//! #     }
//! # }
//! let cell_1 = MyCell::new(1u32);
//! let cell_2 = MyCell::new(2u32);
//!
//! let borrow_1 = cell_1.borrow_mut();
//! let borrow_2 = cell_2.borrow_mut();
//! ```
//!
//! ```plain_text
//! warning: called a function expecting at most 0 mutable borrows of type u32 but was called in a scope with at least 1
//!   --> src/main.rs:10:27
//!    |
//! 10 |     let borrow_2 = cell_2.borrow_mut();
//!    |                           ^^^^^^^^^^^^
//! ```
//!
//! If you're sure you're doing something safe, you can ignore these warnings using the
//! [`assume_no_alias`](crate::assume_no_alias) method.
//!
//! ```rust
//! # use autoken::MutableBorrow;
//! # struct MyCell<T> {
//! #     _ty: core::marker::PhantomData<fn() -> T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(_value: T) -> Self {
//! #         Self { _ty: core::marker::PhantomData }
//! #     }
//! #
//! #     pub fn borrow_mut(&self) -> MutableBorrow<T> {
//! #         MutableBorrow::new()
//! #     }
//! # }
//! let cell_1 = MyCell::new(1u32);
//! let cell_2 = MyCell::new(2u32);
//!
//! let borrow_1 = cell_1.borrow_mut();
//! let borrow_2 = autoken::assume_no_alias(|| cell_2.borrow_mut());
//! ```
//!
//! See [`assume_no_alias_in`](crate::assume_no_alias_in) and [`assume_no_alias_in_many`](crate::assume_no_alias_in_many)
//! for more forms of this function.
//!
//! ### Making Sense of Control Flow Errors
//!
//! The weirdest diagnostic message you are likely to encounter while using AuToken is this one:
//!
//! ```
//! # use autoken::MutableBorrow;
//! # struct MyCell<T> {
//! #     _ty: core::marker::PhantomData<fn() -> T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(_value: T) -> Self {
//! #         Self { _ty: core::marker::PhantomData }
//! #     }
//! #
//! #     pub fn borrow_mut(&self) -> MutableBorrow<T> {
//! #         MutableBorrow::new()
//! #     }
//! # }
//! # let some_condition = true;
//! let cell_1 = MyCell::new(1u32);
//!
//! let my_borrow = if some_condition {
//!     Some(cell_1.borrow_mut())
//! } else {
//!     None
//! };
//! ```
//!
//! ```plain_text
//! warning: not all control-flow paths to this statement are guaranteed to borrow the same number of components
//!   --> src/main.rs:9:21
//!   |
//! 9  |       let my_borrow = if some_condition {
//!   |  _____________________^
//! 10 | |         Some(cell_1.borrow_mut())
//! 11 | |     } else {
//! 12 | |         None
//! 13 | |     };
//!   | |_____^
//! ```
//!
//! This error occurs because of a fundamental limitation of AuToken's design. AuToken analyzes your
//! programs by traversing through the control-flow graph `rustc` generates to analyze, among other
//! things, borrow checking. Every time it encounters a call to [`borrow_mutably`](crate::borrow_mutably)
//! or [`borrow_immutably`](crate::borrow_immutably), it increments the theoretical number of mutable
//! or immutable borrows a given control flow block may have and vice versa with
//! [`unborrow_mutably`](crate::unborrow_mutably) and [`unborrow_immutably`](crate::unborrow_immutably).
//! If there's a divergence in control flow as introduced by an `if` statement or a `loop`, AuToken
//! will visit and analyze each path separately.
//!
//! But what happens when those two paths join back together? How many borrows does a user have if
//! one path borrows `u32` mutably and the other doesn't borrow it at all? AuToken doesn't know the
//! answer to this question and just guesses randomly. Because this guess is probably wrong, it emits
//! a warning to tell you that it really can't handle code written like this.
//!
//! So, if this type of code can't be analyzed by AuToken, what can be done? The best solution is to
//! use a method AuToken integration writers are strongly encouraged to implement: `borrow_on_loan`
//! (or `borrow_mut_on_loan`, or `get_mut_on_loan`... just search for `_on_loan` in the docs!). This
//! method ties the borrow to an externally provided [`MutableBorrow`](crate::MutableBorrow) instance,
//! which should be defined outside of all the conditional logic.
//!
//! ```
//! # use autoken::{MutableBorrow, Nothing};
//! # struct MyCell<T> {
//! #     _ty: core::marker::PhantomData<fn() -> T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(_value: T) -> Self {
//! #         Self { _ty: core::marker::PhantomData }
//! #     }
//! #
//! #     pub fn borrow_mut_on_loan<'l>(&self, loaner: &'l mut MutableBorrow<T>) -> MutableBorrow<Nothing<'l>> {
//! #         loaner.loan()
//! #     }
//! # }
//! # let some_condition = true;
//! let cell_1 = MyCell::new(1u32);
//!
//! let mut guard = MutableBorrow::<u32>::new();
//! let my_borrow = if some_condition {
//!     Some(cell_1.borrow_mut_on_loan(&mut guard))
//! } else {
//!     None
//! };
//! ```
//!
//! If this is too hard to manage, you could also strip the token of all static borrow analysis
//! entirely and all the [`strip_lifetime_analysis`](crate::MutableBorrow::strip_lifetime_analysis)
//! method. This is far more dangerous, however, because AuToken essentially forgets about the
//! existence of that borrow and potentially lets invalid borrows slip by.
//!
//! ```
//! # use autoken::MutableBorrow;
//! # struct MyCell<T> {
//! #     _ty: core::marker::PhantomData<fn() -> T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(_value: T) -> Self {
//! #         Self { _ty: core::marker::PhantomData }
//! #     }
//! #
//! #     pub fn borrow_mut(&self) -> MutableBorrow<T> {
//! #         MutableBorrow::new()
//! #     }
//! # }
//! # let some_condition = true;
//! let cell_1 = MyCell::new(1u32);
//!
//! let my_borrow = if some_condition {
//!     Some(cell_1.borrow_mut().strip_lifetime_analysis())
//! } else {
//!     None
//! };
//! ```
//!
//! Finally, if things get *really* bad, you could ignore the entire section with [`assume_black_box`](crate::assume_black_box).
//! This function is, very much, a last resort, because it prevents the static analysis tool from even
//! looking at anything in the called closure. You should read its documentation for details before
//! even thinking about touching it!
//!
//! ## Dealing With Dynamic Dispatches
//!
//! AuToken resolves dynamic dispatches by collecting all possible dispatch targets ahead of time
//! based around what gets unsized to what and assumes that any of those could be called. This
//! can occasionally be overly pessimistic. You can help this along by making the dynamically
//! dispatched traits more fine grained. For example, instead of using an `FnMut(u32, i32, f32)`, you
//! could use an `FnMut(PhantomData<MyHandlers>, u32, i32, f32)`. Likewise, if you have a trait
//! `MyBehavior`, you could parameterize it by a marker generic type to make it even more fine-grained.
//!
//! If something is really wrong, you could, once again, use [`assume_black_box`](crate::assume_black_box)
//! to hide the unsizing coercions that create these dynamic dispatch targets. Once again, this is,
//! very much, a last resort and you should certainly read its documentation for details before even
//! thinking about touching it!
//!
//! ## Dealing With Foreign Code
//!
//! AuToken has no clue how to deal with foreign code and just ignores it. If you have a foreign
//! function that calls back into userland code, you can tell AuToken that the code is, indeed,
//! reachable with something like this:
//!
//! ```
//! # fn my_ffi_call(f: impl FnOnce()) {}
//! # fn my_callback() {}
//! my_ffi_call(my_callback);
//!
//! if false {  // reachability hint to AuToken
//!     my_callback();
//! }
//! ```
//!
//! # Integrating AuToken
//!
//! **TODO:** Write documentation

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
