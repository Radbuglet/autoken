#![allow(rustdoc::redundant_explicit_links)] // (cargo-rdme needs this)
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
//! ```plain_text
//! warning: called a function expecting at most 0 mutable borrows of type u32 but was called in a scope with at least 1
//!  --> src/main.rs:8:5
//!   |
//! 8 |     bar();
//!   |     ^^^^^
//! ````
//!
//! ## Checking Projects
//!
//! AuToken is a framework for adding static analysis of runtime borrowing to your crate. If you are
//! an end-user of a crate with integrations with AuToken and wish to check your project with the
//! tool, this is the section for you! If, instead, you're building a crate and wish to integrate with
//! AuToken, you should skip to the [Integrating AuToken](#integrating-autoken) section.
//!
//! If you wish to install this tool through `cargo`, you should run a command like:
//!
//! ```bash
//! cargo +nightly-2023-09-08 install cargo-autoken -Z bindeps
//! ```
//!
//! This will likely require you to faff around with rustup toolchains. Because this process could
//! vary from user to user, the best instructions for setting up an appropriate toolchain are provided
//! by rustup, cargo, and rust.
//!
//! If you wish to install from source, assuming your current working directory is the same as the
//! [repository](https://github.com/radbuglet/autoken)'s README, `cargo-autoken` can be installed
//! like so:
//!
//! ```bash
//! cargo install --path src/cargo -Z bindeps
//! ```
//!
//! You can run AuToken validation on a target binary crate by running:
//!
//! ```bash
//! cargo autoken check
//! ```
//!
//! ...in its directory.
//!
//! Have fun!
//!
//! ## Ignoring False Positives
//!
//! AuToken is, by nature, very conservative. After all, its whole job is to ensure that only one
//! borrow of a given type exists at a given time, even if you're potentially borrowing from several
//! different sources at once!
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
//! entirely using the [`strip_lifetime_analysis`](crate::MutableBorrow::strip_lifetime_analysis)
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
//! ### Potential Borrows
//!
//! You may occasionally stumble across a fallible borrow method in your local AuToken-integrate crate
//! which takes in a [`PotentialMutableBorrow`](crate::PotentialMutableBorrow) or [`PotentialImmutableBorrow`](crate::PotentialImmutableBorrow)
//! "loaner" guard. The reason for these guards is somewhat similar to why we need loaner
//! guards for other conditionally created borrows with the added caveat that, because these borrow
//! guards are being used with a fallible borrow method, it is assumed that the aliasing with an
//! existing borrow can be handled gracefully at runtime. Because of this assumption,
//! `PotentialMutableBorrows` do not emit a warning if another confounding borrow guard is already
//! in scope.
//!
//! ```rust
//! # use {autoken::MutableBorrow, std::cell::{BorrowMutError, RefCell, RefMut}};
//! # #[derive(Debug)]
//! # struct MyRefMut<'a, T, B = T> {
//! #     token: MutableBorrow<B>,
//! #     sptr: RefMut<'a, T>,
//! # }
//! # use autoken::{Nothing, PotentialMutableBorrow};
//! # #[derive(Debug)]
//! # struct MyCell<T> {
//! #     inner: RefCell<T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(value: T) -> Self {
//! #         Self { inner: RefCell::new(value) }
//! #     }
//! #
//! #     pub fn try_borrow_mut<'l>(
//! #         &self,
//! #         loaner: &'l mut PotentialMutableBorrow<T>
//! #     ) -> Result<MyRefMut<'_, T, Nothing<'l>>, BorrowMutError> {
//! #         self.inner.try_borrow_mut().map(|sptr| MyRefMut {
//! #             token: loaner.loan(),
//! #             sptr,
//! #         })
//! #     }
//! # }
//! let my_cell = MyCell::new(1u32);
//!
//! let mut my_loaner_1 = PotentialMutableBorrow::<u32>::new();
//! let borrow_1 = my_cell.try_borrow_mut(&mut my_loaner_1).unwrap();
//!
//! // This should not trigger a static analysis warning because, if the specific cell is already
//! // borrowed, the function returns an `Err` rather than panicking.
//! let mut my_loaner_2 = PotentialMutableBorrow::<u32>::new();
//! let not_borrow_2 = my_cell.try_borrow_mut(&mut my_loaner_2).unwrap_err();
//! ```
//!
//! If the borrow cannot be handled gracefully, one may create a [`MutableBorrow`](crate::MutableBorrow)
//! or [`ImmutableBorrow`](crate::ImmutableBorrow) guard and [`downgrade`](crate::MutableBorrow::downgrade)
//! it to a `PotentialMutableBorrow` or `PotentialImmutableBorrow` guard so that the static analyzer
//! will start reporting these potentially problematic borrows again.
//!
//! ```no_run
//! # use {autoken::MutableBorrow, std::cell::{BorrowMutError, RefCell, RefMut}};
//! # #[derive(Debug)]
//! # struct MyRefMut<'a, T, B = T> {
//! #     token: MutableBorrow<B>,
//! #     sptr: RefMut<'a, T>,
//! # }
//! # use autoken::{Nothing, PotentialMutableBorrow};
//! # #[derive(Debug)]
//! # struct MyCell<T> {
//! #     inner: RefCell<T>,
//! # }
//! # impl<T> MyCell<T> {
//! #     pub fn new(value: T) -> Self {
//! #         Self { inner: RefCell::new(value) }
//! #     }
//! #
//! #     pub fn try_borrow_mut<'l>(
//! #         &self,
//! #         loaner: &'l mut PotentialMutableBorrow<T>
//! #     ) -> Result<MyRefMut<'_, T, Nothing<'l>>, BorrowMutError> {
//! #         self.inner.try_borrow_mut().map(|sptr| MyRefMut {
//! #             token: loaner.loan(),
//! #             sptr,
//! #         })
//! #     }
//! # }
//! let my_cell = MyCell::new(1u32);
//!
//! let mut my_loaner_1 = PotentialMutableBorrow::<u32>::new();
//! let borrow_1 = my_cell.try_borrow_mut(&mut my_loaner_1).unwrap();
//!
//! // Unlike the previous example, this code cannot handle aliasing borrows gracefully, so we should
//! // create a `MutableBorrow` first to get the alias check and then downgrade it for use in the
//! // fallible borrowing method.
//! let mut my_loaner_2 = MutableBorrow::<u32>::new().downgrade();
//! let not_borrow_2 = my_cell.try_borrow_mut(&mut my_loaner_2).unwrap();
//! ```
//!
//! ## Dealing With Dynamic Dispatches
//!
//! AuToken resolves dynamic dispatches by collecting all possible dispatch targets ahead of time
//! based around what gets unsized to what and assumes that any of those concrete types could be
//! called by an invocation of a given unsized type. This can occasionally be overly pessimistic.
//! You can help this along by making the dynamically dispatched traits more fine grained. For
//! example, instead of using an `FnMut(u32, i32, f32)`, you could use an
//! `FnMut(PhantomData<MyHandlers>, u32, i32, f32)`. Likewise, if you have a trait `MyBehavior`, you
//! could parameterize it by a marker generic type to make it even more fine-grained.
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
//! This section is for crate developers wishing to add static analysis to their dynamic borrowing
//! schemes. If you're interested in using one of those crates, see the [checking projects](#checking-projects)
//! section.
//!
//! There are four primitive borrowing functions offered by this library:
//!
//! - [`borrow_mutably<T>`](crate::borrow_mutably)
//! - [`borrow_immutably<T>`](crate::borrow_immutably)
//! - [`unborrow_mutably<T>`](crate::unborrow_mutably)
//! - [`unborrow_immutably<T>`](crate::unborrow_immutably)
//!
//! These functions, in reality, do absolutely nothing and are compiled away. However, when checked
//! by the custom AuToken rustc wrapper, they virtually "borrow" and "unborrow" a global token of
//! the type `T` and raise a warning if it is possible to violate the XOR mutability rules of that
//! virtual global token.
//!
//! Usually, these functions aren't called directly and are instead called indirectly through their
//! RAII'd counterparts [`MutableBorrow`](crate::MutableBorrow) and [`ImmutableBorrow`](crate::ImmutableBorrow).
//!
//! These primitives can be used to introduce additional compile-time safety to dynamically checked
//! borrowing and locking schemes. Here's a couple of examples:
//!
//! You could make a safe wrapper around a `RefCell`...
//!
//! ```no_run
//! use autoken::MutableBorrow;
//! use std::cell::{RefCell, RefMut};
//!
//! struct MyRefCell<T> {
//!     inner: RefCell<T>,
//! }
//!
//! impl<T> MyRefCell<T> {
//!     pub fn new(value: T) -> Self {
//!         Self { inner: RefCell::new(value) }
//!     }
//!
//!     pub fn borrow_mut(&self) -> MyRefMut<'_, T> {
//!         MyRefMut {
//!             token: MutableBorrow::new(),
//!             sptr: self.inner.borrow_mut(),
//!         }
//!     }
//! }
//!
//! struct MyRefMut<'a, T> {
//!     token: MutableBorrow<T>,
//!     sptr: RefMut<'a, T>,
//! }
//!
//! let my_cell = MyRefCell::new(1u32);
//! let _a = my_cell.borrow_mut();
//!
//! // This second mutable borrow results in an AuToken warning.
//! let _b = my_cell.borrow_mut();
//! ```
//!
//! ```plain_text
//! warning: called a function expecting at most 0 mutable borrows of type u32 but was called in a scope with at least 1
//!   --> src/main.rs:33:22
//!    |
//! 33 |     let _b = my_cell.borrow_mut();
//!    |                      ^^^^^^^^^^^^
//! ````
//!
//! You could make a reentrancy-protected function...
//!
//! ```rust
//! fn do_not_reenter(f: impl FnOnce()) {
//!     struct ISaidDoNotReenter;
//!
//!     let _guard = autoken::MutableBorrow::<ISaidDoNotReenter>::new();
//!     f();
//! }
//!
//! do_not_reenter(|| {
//!     // Whoops!
//!     do_not_reenter(|| {});
//! });
//! ```
//!
//! ```plain_text
//! warning: called a function expecting at most 0 mutable borrows of type main::do_not_reenter::ISaidDoNotReenter but was called in a scope with at least 1
//!  --> src/main.rs:6:9
//!   |
//! 6 |         f();
//!   |         ^^^
//! ```
//!
//! You could even deny an entire class of functions where calling them would be dangerous!
//!
//! ```rust
//! use autoken::{ImmutableBorrow, MutableBorrow};
//!
//! struct IsOnMainThread;
//!
//! fn begin_multithreading(f: impl FnOnce()) {
//!     let _guard = MutableBorrow::<IsOnMainThread>::new();
//!     f();
//! }
//!
//! fn only_call_me_on_main_thread() {
//!     let _guard = ImmutableBorrow::<IsOnMainThread>::new();
//!     // ...
//! }
//!
//! begin_multithreading(|| {
//!     // Whoops!
//!     only_call_me_on_main_thread();
//! });
//! ```
//!
//! ```plain_text
//! warning: called a function expecting at most 0 mutable borrows of type main::IsOnMainThread but was called in a scope with at least 1
//!  --> src/main.rs:6:9
//!   |
//! 6 |         f();
//!   |         ^^^
//! ```
//!
//! Pretty neat, huh.
//!
//! ## Dealing with Limitations
//!
//! If you read the [checking projects](#checking-projects) section like I asked you not to, you'd
//! hear about four pretty major limitations of AuToken. While most of these limitations can be overcome
//! by tools provided by AuToken, the second limitation—[Control Flow Errors](#making-sense-of-control-flow-errors)—
//! requires a bit of help from developers wishing to integrate with AuToken. You are strongly
//! encouraged to read that section before this section, since it motivates the necessity for these
//! special method variants.
//!
//! In summary:
//!
//! 1. For every guard object, provide a `strip_lifetime_analysis` function similar to
//!    [`MutableBorrow`](crate::MutableBorrow::strip_lifetime_analysis)'s.
//! 2. For every guard object, provide a way to acquire that object with a "loaner" borrow object. The
//!    recommended suffix for this variant is `on_loan`. The mechanism for doing so is likely very
//!    similar to `MutableBorrow`'s [`loan`](crate::MutableBorrow::loan) method.
//! 3. For conditional borrow methods which check their borrow before performing it, the method should
//!    be made to loan a [`PotentialMutableBorrow`](crate::PotentialMutableBorrow) or [`PotentialImmutableBorrow`](crate::PotentialImmutableBorrow)
//!    instead.
//!
//! All of these methods rely on being able to convert the RAII guard's type from its originally
//! borrowed type to [`Nothing`](crate::Nothing)—a special marker type in AuToken which indicates that
//! the borrow guard isn't actually borrowing anything. Doing this requires you to keep track of the
//! borrowed type at the type level since AuToken lacks the power to analyze runtime mechanisms for
//! doing that. Here's an example of how to accomplish this:
//!
//! ```rust
//! # use {autoken::MutableBorrow, std::cell::RefMut};
//! struct MyRefMut<'a, T, B = T> {
//!     //                 ^ notice the addition of this special parameter?
//!     token: MutableBorrow<B>,
//!     sptr: RefMut<'a, T>,
//! }
//! ```
//!
//! With that additional parameter in place, we can implement the first required method: `strip_lifetime_analysis`.
//! Its implementation is relatively straightforward:
//!
//! ```rust
//! # use {autoken::MutableBorrow, std::cell::{RefCell, RefMut}};
//! # struct MyRefCell<T> {
//! #     inner: RefCell<T>,
//! # }
//! #
//! # impl<T> MyRefCell<T> {
//! #     pub fn new(value: T) -> Self {
//! #         Self { inner: RefCell::new(value) }
//! #     }
//! #
//! #     pub fn borrow_mut(&self) -> MyRefMut<'_, T> {
//! #         MyRefMut {
//! #             token: MutableBorrow::new(),
//! #             sptr: self.inner.borrow_mut(),
//! #         }
//! #     }
//! # }
//! use autoken::Nothing;
//!
//! struct MyRefMut<'a, T, B = T> {
//!     token: MutableBorrow<B>,
//!     sptr: RefMut<'a, T>,
//! }
//!
//! impl<'a, T, B> MyRefMut<'a, T, B> {
//!     pub fn strip_lifetime_analysis(self) -> MyRefMut<'a, T, Nothing<'static>> {
//!         MyRefMut {
//!             token: self.token.strip_lifetime_analysis(),
//!             sptr: self.sptr,
//!         }
//!     }
//! }
//!
//! # let my_condition = true;
//! let my_cell = MyRefCell::new(1u32);
//! let my_guard = if my_condition {
//!     Some(my_cell.borrow_mut().strip_lifetime_analysis())
//! } else {
//!     None
//! };
//! ```
//!
//! The `'static` lifetime in `Nothing` doesn't really mean anything. Indeed, the lifetime in `Nothing`
//! is purely a convenience lifetime whose utility will become more clear when we implement the second
//! required method: `borrow_mut_on_loan`.
//!
//! Writing this method is also relatively straightforward:
//!
//! ```rust
//! # use {autoken::MutableBorrow, std::cell::{RefCell, RefMut}};
//! # struct MyRefMut<'a, T, B = T> {
//! #     token: MutableBorrow<B>,
//! #     sptr: RefMut<'a, T>,
//! # }
//! use autoken::Nothing;
//!
//! struct MyRefCell<T> {
//!     inner: RefCell<T>,
//! }
//!
//! impl<T> MyRefCell<T> {
//!     # pub fn new(value: T) -> Self {
//!     #     Self { inner: RefCell::new(value) }
//!     # }
//!     pub fn borrow_mut_on_loan<'l>(
//!         &self,
//!         loaner: &'l mut MutableBorrow<T>
//!     ) -> MyRefMut<'_, T, Nothing<'l>> {
//!         MyRefMut {
//!             token: loaner.loan(),
//!             sptr: self.inner.borrow_mut(),
//!         }
//!     }
//! }
//!
//! # let my_condition = true;
//! let my_cell = MyRefCell::new(1u32);
//!
//! let mut my_loaner = MutableBorrow::<u32>::new();
//! let my_guard = if my_condition {
//!     Some(my_cell.borrow_mut_on_loan(&mut my_loaner))
//! } else {
//!     None
//! };
//! ```
//!
//! Here, we're using the placeholder lifetime in `Nothing` to limit the lifetime of the loans to
//! the reference to the `loaner`. Pretty convenient.
//!
//! Finally, fallible `borrow` method variants can be implemented in a way almost identical to the
//! previous example's:
//!
//! ```rust
//! # use {autoken::MutableBorrow, std::cell::{BorrowMutError, RefCell, RefMut}};
//! # #[derive(Debug)]
//! # struct MyRefMut<'a, T, B = T> {
//! #     token: MutableBorrow<B>,
//! #     sptr: RefMut<'a, T>,
//! # }
//! use autoken::{Nothing, PotentialMutableBorrow};
//!
//! # #[derive(Debug)]
//! struct MyRefCell<T> {
//!     inner: RefCell<T>,
//! }
//!
//! impl<T> MyRefCell<T> {
//!     # pub fn new(value: T) -> Self {
//!     #     Self { inner: RefCell::new(value) }
//!     # }
//!     pub fn try_borrow_mut<'l>(
//!         &self,
//!         loaner: &'l mut PotentialMutableBorrow<T>
//!     ) -> Result<MyRefMut<'_, T, Nothing<'l>>, BorrowMutError> {
//!         self.inner.try_borrow_mut().map(|sptr| MyRefMut {
//!             token: loaner.loan(),
//!             sptr,
//!         })
//!     }
//! }
//!
//! let my_cell = MyRefCell::new(1u32);
//!
//! let mut my_loaner_1 = PotentialMutableBorrow::<u32>::new();
//! let borrow_1 = my_cell.try_borrow_mut(&mut my_loaner_1).unwrap();
//!
//! let mut my_loaner_2 = PotentialMutableBorrow::<u32>::new();
//! let not_borrow_2 = my_cell.try_borrow_mut(&mut my_loaner_2).unwrap_err();
//! ```
//!
//! How exciting!

#![no_std]

use core::{cmp::Ordering, fmt, marker::PhantomData, mem};

// === Primitives === //

/// Virtually acquires a mutable reference to a global token of type `T`.
///
/// This method is more typically called through the [`MutableBorrow`] guard's constructor.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub const fn borrow_mutably<T: ?Sized>() {
    const fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}

/// Virtually acquires an immutable reference to a global token of type `T`.
///
/// This method is more typically called through the [`ImmutableBorrow`] guard's constructor.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub const fn borrow_immutably<T: ?Sized>() {
    const fn __autoken_borrow_immutably<T: ?Sized>() {}

    __autoken_borrow_immutably::<T>();
}

/// Virtually unacquires a mutable reference to a global token of type `T`.
///
/// This method is more typically called through the [`MutableBorrow`] guard's destructor.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub const fn unborrow_mutably<T: ?Sized>() {
    const fn __autoken_unborrow_mutably<T: ?Sized>() {}

    __autoken_unborrow_mutably::<T>();
}

/// Virtually unacquires an immutable reference to a global token of type `T`.
///
/// This method is more typically called through the [`ImmutableBorrow`] guard's destructor.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub const fn unborrow_immutably<T: ?Sized>() {
    const fn __autoken_unborrow_immutably<T: ?Sized>() {}

    __autoken_unborrow_immutably::<T>();
}

/// Ensures that it is possible to virtually acquire a mutable borrow of the global token of type `T`.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub const fn assert_mutably_borrowable<T: ?Sized>() {
    borrow_mutably::<T>();
    unborrow_mutably::<T>();
}

/// Ensures that it is possible to virtually acquire an immutable borrow of the global token of type `T`.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub const fn assert_immutably_borrowable<T: ?Sized>() {
    borrow_immutably::<T>();
    unborrow_immutably::<T>();
}

/// Asserts that the provided closure's virtual borrows to tokens of type `(T_1, ..., T_n)` will not
/// cause any aliasing issues at runtime.
///
/// `T` must be a tuple of token types to ignore. Its maximum supported arity is 12.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When a component in `T` is [`Nothing`], it is effectively ignored.
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

/// Asserts that the provided closure's virtual borrows to tokens of type `T` will not cause any
/// aliasing issues at runtime.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
///
/// Global token identity is lifetime-erased (i.e. `&'a u32` and `&'b u32` always refer to the same
/// virtual global token). When `T` is [`Nothing`], nothing happens.
pub fn assume_no_alias_in<T: ?Sized, Res>(f: impl FnOnce() -> Res) -> Res {
    assume_no_alias_in_many::<(T,), Res>(f)
}

/// Asserts that the provided closure's virtual borrows to any token will not cause any aliasing
/// issues at runtime.
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
pub fn assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
    fn __autoken_assume_no_alias<Res>(f: impl FnOnce() -> Res) -> Res {
        f()
    }

    __autoken_assume_no_alias::<Res>(f)
}

/// Tells the AuToken static analyzer to entirely ignore the body of the provided closure. This should
/// be a last resort for when nothing else works out.
///
/// This prevents both enumeration of unsizing coercions performed by the closure (which contribute
/// to the set of potential dynamic dispatch targets for a given function pointer or trait type) and
/// detection of the various `borrow` and `unborrow` function calls. This can be particularly tricky
/// if you return a guard from the black-boxed closure since, although the call to [`borrow_mutably`]
/// was ignored, the call to [`unborrow_mutably`] in the destructor is not:
///
/// ```rust
/// use autoken::MutableBorrow;
///
/// // The call to `borrow_mutably` was just ignored.
/// let guard = autoken::assume_black_box(|| MutableBorrow::<u32>::new());
///
/// // ...but the call to `unborrow_mutably` was not!
/// drop(guard);
///
/// // Autoken now thinks that we have -1 mutable borrows in scope!
/// ```
///
/// If we want to fix this, we need to make sure to `strip_lifetime_analysis` on all the guards we
/// return:
///
/// ```rust
/// use autoken::MutableBorrow;
///
/// // The call to `borrow_mutably` was just ignored.
/// let guard = autoken::assume_black_box(|| {
///     MutableBorrow::<u32>::new().strip_lifetime_analysis()
/// });
///
/// // Luckily, in stripping its lifetime analysis, it no longer calls `unborrow_mutably` here.
/// drop(guard);
///
/// // Autoken now has an accurate idea of the number of guards in scope.
/// ```
///
/// It bears repeating that this function is super dangerous. Please consider all the alternatives
/// listed in the crate's [Ignoring False Positives](index.html#ignoring-false-positives) and
/// [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors) section before
/// even thinking about reaching for this function!
///
/// In regular builds, this does nothing, but when AuToken checks a given binary, it uses calls to
/// functions like this to determine whether a program has the possibility of virtually borrowing a
/// global token in a way which violates XOR borrowing rules.
pub fn assume_black_box<T>(f: impl FnOnce() -> T) -> T {
    fn __autoken_assume_black_box<T>(f: impl FnOnce() -> T) -> T {
        f()
    }

    __autoken_assume_black_box::<T>(f)
}

/// A marker type representing a borrow of... "nothing."
///
/// When passed as a parameter to any borrow-adjacent function or method in this crate, this type
/// essentially turns that operation into a no-op. The lifetime is merely a placeholder to help with
/// the common idioms detailed in the [Dealing with Limitations](index.html#dealing-with-limitations)
/// section of the integration guide.
///
/// This is useful for disabling borrows for a given RAII'd AuToken object, as needed by
/// `strip_lifetime_analysis` or for tying an object's borrow to a different [`MutableBorrow`] or
/// [`ImmutableBorrow`] guard.
///
/// An instance of this type cannot be obtained—it is a marker type.
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
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

/// A guard for a virtual mutable borrow of the global token of type `T`.
///
/// These can be thought of as [`Ref`](core::cell::Ref)s to those global tokens except that they have
/// no impact on the runtime and only contribute to AuToken's static analysis of the binary.
///
/// As with [`borrow_mutably`] and friends, setting `T` to [`Nothing`] causes this guard to have no
/// effect on the statically-analyzed borrow counts.
///
/// Developers wishing to integrate their crate with AuToken will likely use this type to represent
/// the static counterpart to a runtime borrow of some cell of type `T` as described in the
/// [Integrating AuToken](index.html#integrating-autoken) section of the crate documentation.
///
/// End-users consuming crates with integrations with AuToken, meanwhile, will likely only use these
/// guards for loaned borrows as described by the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
/// section of the crate documentation.
pub struct MutableBorrow<T: ?Sized> {
    _ty: PhantomData<fn() -> T>,
}

impl<T: ?Sized> MutableBorrow<T> {
    /// Constructs a new `MutableBorrow` guard.
    ///
    /// This function has no runtime cost but will cause the AuToken static analyzer to report
    /// potential virtual borrowing issues with other guards.
    ///
    /// Internally, this function just calls [`borrow_mutably`] with the provided type `T`.
    pub const fn new() -> Self {
        borrow_mutably::<T>();
        Self { _ty: PhantomData }
    }

    /// Transforms this `MutableBorrow` into a [`PotentialMutableBorrow`] of the same type.
    pub fn downgrade(self) -> PotentialMutableBorrow<T> {
        PotentialMutableBorrow(self)
    }

    /// Transforms a reference to `MutableBorrow` into a reference to a [`PotentialMutableBorrow`] of
    /// the same type.
    pub fn downgrade_ref(&self) -> &PotentialMutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    /// Transforms a mutable reference to `MutableBorrow` into a mutable reference to a [`PotentialMutableBorrow`]
    /// of the same type.
    pub fn downgrade_mut(&mut self) -> &mut PotentialMutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    /// Creates a loaned `MutableBorrow` of this guard which has no effect on the static analysis
    /// borrow counters by itself, making it safe to use in conditional code.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on loans.
    pub fn loan(&mut self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    /// Creates a loaned `MutableBorrow` of this guard which has no effect on the static analysis
    /// borrow counters by itself, making it safe to use in conditional code.
    ///
    /// Unlike [`loan`](MutableBorrow::loan), this method takes an immutable reference to the loaning
    /// `MutableBorrow`, which makes it more prone to accidental borrow aliasing.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on loans.
    pub fn assume_no_alias_loan(&self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    /// Clones the current `MutableBorrow` instance and assumes that it is safe to do so.
    pub fn assume_no_alias_clone(&self) -> Self {
        assume_no_alias(|| Self::new())
    }

    /// Transforms the type of `T` into [`Nothing`], effectively making it as if this borrow guard no
    /// longer exists.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on the utility of `strip_lifetime_analysis`.
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

/// A guard for a virtual immutable borrow of the global token of type `T`.
///
/// These can be thought of as [`Ref`](core::cell::Ref)s to those global tokens except that they have
/// no impact on the runtime and only contribute to AuToken's static analysis of the binary.
///
/// As with [`borrow_mutably`] and friends, setting `T` to [`Nothing`] causes this guard to have no
/// effect on the statically-analyzed borrow counts.
///
/// Developers wishing to integrate their crate with AuToken will likely use this type to represent
/// the static counterpart to a runtime borrow of some cell of type `T` as described in the
/// [Integrating AuToken](index.html#integrating-autoken) section of the crate documentation.
///
/// End-users consuming crates with integrations with AuToken, meanwhile, will likely only use these
/// guards for loaned borrows as described by the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
/// section of the crate documentation.
pub struct ImmutableBorrow<T: ?Sized> {
    _ty: PhantomData<fn() -> T>,
}

impl<T: ?Sized> ImmutableBorrow<T> {
    /// Constructs a new `ImmutableBorrow` guard.
    ///
    /// This function has no runtime cost but will cause the AuToken static analyzer to report
    /// potential virtual borrowing issues with other guards.
    ///
    /// Internally, this function just calls [`borrow_immutably`] with the provided type `T`.
    pub const fn new() -> Self {
        borrow_immutably::<T>();
        Self { _ty: PhantomData }
    }

    /// Transforms this `ImmutableBorrow` into a [`PotentialImmutableBorrow`] of the same type.
    pub fn downgrade(self) -> PotentialImmutableBorrow<T> {
        PotentialImmutableBorrow(self)
    }

    /// Transforms a reference to `ImmutableBorrow` into a reference to a [`PotentialImmutableBorrow`]
    /// of the same type.
    pub fn downgrade_ref(&self) -> &PotentialImmutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    /// Transforms a mutable reference to `ImmutableBorrow` into a mutable reference to a [`PotentialImmutableBorrow`]
    /// of the same type.
    pub fn downgrade_mut(&mut self) -> &mut PotentialImmutableBorrow<T> {
        unsafe { mem::transmute(self) }
    }

    /// Creates a loaned `ImmutableBorrow` of this guard which has no effect on the static analysis
    /// borrow counters by itself, making it safe to use in conditional code.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on loans.
    pub const fn loan(&self) -> ImmutableBorrow<Nothing<'_>> {
        ImmutableBorrow::new()
    }

    /// Transforms the type of `T` into [`Nothing`], effectively making it as if this borrow guard no
    /// longer exists.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on the utility of `strip_lifetime_analysis`.
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

/// A variant of [`MutableBorrow`] which represents a mutable borrow which can gracefully recover from
/// borrow errors if they end up occurring.
///
/// Unlike a `MutableBorrow`, this token will not trigger a warning if a confounding borrow is
/// potentially alive at the same time as it since, if the dynamic borrow this borrow guard backs
/// ends up aliasing with something else, the error is assumed to be handled gracefully.
///
/// As with [`borrow_mutably`] and friends, setting `T` to [`Nothing`] causes this guard to have no
/// effect on the statically-analyzed borrow counts.
///
/// If the error cannot be handled gracefully, one may construct a `MutableBorrow` and
/// [`downgrade`](MutableBorrow::downgrade) it to a `PotentialMutableBorrow` so that the static
/// analyzer will start reporting these potentially aliasing borrows again.
///
/// See the [Potential Borrows](index.html#potential-borrows) section of the crate documentation for
/// more details.
#[repr(transparent)]
pub struct PotentialMutableBorrow<T: ?Sized>(MutableBorrow<T>);

impl<T: ?Sized> PotentialMutableBorrow<T> {
    /// Constructs a new `PotentialMutableBorrow` guard.
    ///
    /// This function has no runtime cost but will cause the AuToken static analyzer to increment the
    /// number of mutable borrows of type `T` in scope. Note, however, that it will not cause the
    /// static analyzer to report aliases with potentially confounding borrow tokens. See the structure
    /// documentation for details.
    ///
    /// Internally, this function calls:
    ///
    /// ```rust
    /// # use autoken::MutableBorrow;
    /// # type T = u32;
    /// autoken::assume_no_alias(|| MutableBorrow::<T>::new());
    /// ```
    pub fn new() -> Self {
        assume_no_alias(|| Self(MutableBorrow::new()))
    }

    /// Creates a loaned [`MutableBorrow`] of this guard which has no effect on the static analysis
    /// borrow counters by itself, making it safe to use in conditional code.
    ///
    /// This is typically used to construct the `MutableBorrow` guard for runtime borrow guards which
    /// were successfully created in fallible code.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on loans.
    pub fn loan(&mut self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    /// Creates a loaned `MutableBorrow` of this guard which has no effect on the static analysis
    /// borrow counters by itself, making it safe to use in conditional code.
    ///
    /// This is typically used to construct the `MutableBorrow` guard for runtime borrow guards which
    /// were successfully created in fallible code.
    ///
    /// Unlike [loan](PotentialMutableBorrow::loan), this method takes an immutable reference to the
    /// loaning `PotentialMutableBorrow`, which makes it more prone to accidental borrow aliasing.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on loans.
    pub fn assume_no_alias_loan(&self) -> MutableBorrow<Nothing<'_>> {
        MutableBorrow::new()
    }

    /// Transforms the type of `T` into [`Nothing`], effectively making it as if this borrow guard no
    /// longer exists.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on the utility of `strip_lifetime_analysis`.
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

/// A variant of [`ImmutableBorrow`] which represents an immutable borrow which can gracefully recover from
/// borrow errors if they end up occurring.
///
/// Unlike an `ImmutableBorrow`, this token will not trigger a warning if a confounding borrow is
/// potentially alive at the same time as it since, if the dynamic borrow this borrow guard backs
/// ends up aliasing with something else, the error is assumed to be handled gracefully.
///
/// As with [`borrow_immutably`] and friends, setting `T` to [`Nothing`] causes this guard to have no
/// effect on the statically-analyzed borrow counts.
///
/// If the error cannot be handled gracefully, one may construct an `ImmutableBorrow` and
/// [`downgrade`](ImmutableBorrow::downgrade) it to a `PotentialImmutableBorrow` so that the static
/// analyzer will start reporting these potentially aliasing borrows again.
///
/// See the [Potential Borrows](index.html#potential-borrows) section of the crate documentation for
/// more details.
#[repr(transparent)]
pub struct PotentialImmutableBorrow<T: ?Sized>(ImmutableBorrow<T>);

impl<T: ?Sized> PotentialImmutableBorrow<T> {
    /// Constructs a new `PotentialImmutableBorrow` guard.
    ///
    /// This function has no runtime cost but will cause the AuToken static analyzer to increment the
    /// number of immutable borrows of type `T` in scope. Note, however, that it will not cause the
    /// static analyzer to report aliases with potentially confounding borrow tokens. See the structure
    /// documentation for details.
    ///
    /// Internally, this function calls:
    ///
    /// ```rust
    /// # use autoken::ImmutableBorrow;
    /// # type T = u32;
    /// autoken::assume_no_alias(|| ImmutableBorrow::<T>::new());
    /// ```
    pub fn new() -> Self {
        assume_no_alias(|| Self(ImmutableBorrow::new()))
    }

    /// Creates a loaned [`ImmutableBorrow`] of this guard which has no effect on the static analysis
    /// borrow counters by itself, making it safe to use in conditional code.
    ///
    /// This is typically used to construct the `ImmutableBorrow` guard for runtime borrow guards which
    /// were successfully created in fallible code.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on loans.
    pub const fn loan(&self) -> ImmutableBorrow<Nothing<'_>> {
        ImmutableBorrow::new()
    }

    /// Transforms the type of `T` into [`Nothing`], effectively making it as if this borrow guard no
    /// longer exists.
    ///
    /// See the [Making Sense of Control Flow Errors](index.html#making-sense-of-control-flow-errors)
    /// section of the crate documentation for more details on the utility of `strip_lifetime_analysis`.
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
