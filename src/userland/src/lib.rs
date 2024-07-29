#![allow(rustdoc::redundant_explicit_links)] // (cargo-rdme needs this)
//! A rust-lang compiler tool adding support for zero-cost borrow-aware context passing.
//!
//! ```no_run
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
//! ## High-Level Usage
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
//! ```compile_fail
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
//! If, for some reason, you need to "smuggle" access to a `cap!` past a dynamic dispatch boundary,
//! you can use the [`Borrows`](crate:Borrows) object and its alias [`BorrowsOne`](crate:BorrowsOne).
//!
//! `Borrows` is an object representing a borrow of a set of capabilities. If you have an mutable
//! reference to it, you are effectively borrowing that entire set of capabilities mutably. You can
//! create a `Borrows` object from the surrounding implicit `Borrows` context like so:
//!
//! ```rust
//! # autoken::cap! {
//! #     pub MyCap = u32;
//! # }
//! #
//! # fn increment_counter() {
//! #     *autoken::cap!(mut MyCap) += 1;
//! # }
//! fn demo_1() {
//!     let borrows = autoken::BorrowsOne::<MyCap>::acquire_mut();
//!     let mut increment = || {
//!         borrows.absorb(|| {
//!             increment_counter();
//!         });
//!     };
//!     let increment_dyn: &mut dyn FnMut() = &mut increment;
//!
//!     increment_dyn();
//! }
//!
//! fn demo_2() {
//!     let increment = |token: &mut autoken::BorrowsOne<MyCap>| {
//!         token.absorb(|| {
//!             increment_counter();
//!         });
//!     };
//!     let increment: fn(&mut autoken::BorrowsOne<MyCap>) = increment;
//!
//!     increment(autoken::BorrowsOne::<MyCap>::acquire_mut());
//! }
//! ```
//!
//! ## Low-Level Usage
//!
//! Internally, [`cap!`](crate::cap) is not a primitive feature of AuToken. Instead, it is built
//! entirely in-userland using `thread_local!` with the help of two custom analysis intrinsics:
//! [`tie!`](crate::tie) and [`absorb`](crate::absorb).
//!
//! `tie!` is a macro which can be used in the body of a function to declare the fact that a lifetime
//! in the function's return type is tied to a borrow of some global token type. For example, you
//! could write:
//!
//! ```rust
//! pub struct MySingleton {
//! # /*
//!     ...
//! # */
//! }
//!
//! pub fn get_singleton<'a>() -> &'a mut MySingleton {
//!     autoken::tie!('a => mut MySingleton);
//!
//!     unimplemented!();
//! }
//! ```
//!
//! ...and no one would be able to write a function which acquires multiple mutable references to
//! `MySingleton` simultaneously since doing so would require borrowing the `MySingleton` "token"
//! mutably more than once.
//!
//! ```rust
//! # pub struct MySingleton {}
//! #
//! # pub fn get_singleton<'a>() -> &'a mut MySingleton {
//! #     autoken::tie!('a => mut MySingleton);
//! #
//! #     unimplemented!();
//! # }
//! fn demo() {
//!     let singleton_1 = get_singleton();
//!     let singleton_2 = get_singleton();
//!     let _ = singleton_1;
//! }
//! ```
//!
//! ```plain_text
//! error: conflicting borrows on token MySingleton
//!   --> src/main.rs:11:23
//!    |
//! 10 |     let singleton_1 = get_singleton();
//!    |                       --------------- value first borrowed mutably
//! 11 |     let singleton_2 = get_singleton();
//!    |                       ^^^^^^^^^^^^^^^ value later borrowed mutably
//!    |
//!    = help: first borrow originates from get_singleton::<'_>
//!    = help: later borrow originates from get_singleton::<'_>
//! ```
//!
//! In effect, you could think of `autoken::tie!` as introducing a new virtual parameter to your
//! function indicating exclusive/shared access to a given contextual resource. The code above, for
//! example, could be logically desugared as:
//!
//! ```compile_fail
//! # use std::marker::PhantomData;
//! #
//! # pub struct Token<T: ?Sized>(PhantomData<T>);
//! #
//! pub struct MySingleton {
//! # /*
//!     ...
//! # */
//! }
//!
//! pub fn get_singleton<'a>(access_perms: &'a mut Token<MySingleton>) -> &'a mut MySingleton {
//!     //                                   ^^ this is what the `'a` means in `tie!('a => mut MySingleton)`
//!     //                                                ^^^^^^^^^^^ and this is what the `MySingleton` means.
//!     unimplemented!();
//! }
//!
//! fn demo(access_perms: &mut Token<MySingleton>) {
//!     //                ^^^^^^^^^^^^^^^^^^^^^^^ The existence of this parameter is inferred from the
//!     //                                        call graph. Note that its lifetime is anonymous since
//!     //                                        it wasn't explicitly tied to anything.
//!     let singleton_1 = get_singleton(access_perms);
//!     let singleton_2 = get_singleton(access_perms);
//!     let _ = singleton_1;
//! }
//! ```
//!
//! ```plain_text
//! error[E0499]: cannot borrow `*access_perms` as mutable more than once at a time
//!   --> src/main.rs:18:37
//!    |
//! 17 |     let singleton_1 = get_singleton(access_perms);
//!    |                                     ------------ first mutable borrow occurs here
//! 18 |     let singleton_2 = get_singleton(access_perms);
//!    |                                     ^^^^^^^^^^^^ second mutable borrow occurs here
//! 19 |     let _ = singleton_1;
//!    |             ----------- first borrow later used here
//! ```
//!
//! But how do we ensure that these tokens actually come from somewhere like a `cap!` block? This
//! is where the second mechanism comes in: `absorb`.
//!
//! If `absorb` didn't exist, any attempt at running code involving a `tie!` directive would end in
//! this compile-time error:
//!
//! ```no_run
//! struct MySingleton {}
//!
//! fn get_singleton<'a>() -> &'a mut MySingleton {
//!     autoken::tie!('a => mut MySingleton);
//!     unimplemented!();
//! }
//!
//! fn main() {
//!     get_singleton();
//! }
//! ```
//!
//! ```plain_text
//! error: cannot use this main function because it borrows unabsorbed tokens
//!  --> src/main.rs:8:1
//!   |
//! 8 | fn main() {
//!   | ^^^^^^^^^
//!   |
//!   = note: uses &mut MySingleton.
//! ```
//!
//! This is because `main` functions, `extern` functions, and unsized functions/methods are not
//! permitted to request any tokens. Hence, in order to call a function with a `tie!` directive, we
//! must somehow get rid of that request for a token. We can do that with `absorb`.
//!
//! `absorb` absorbs the existence of a token's borrow, hiding it from its caller. In this case, if
//! we wanted to give our main function the ability to call `get_singleton`, we could wrap the call
//! in an `absorb` call like so:
//!
//! ```no_run
//! struct MySingleton {}
//!
//! fn get_singleton<'a>() -> &'a mut MySingleton {
//!     autoken::tie!('a => mut MySingleton);
//!     unimplemented!();
//! }
//!
//! fn main() {
//!     unsafe {
//!         autoken::absorb::<autoken::Mut<MySingleton>, ()>(|| {
//!             get_singleton();
//!         });
//!     }
//! }
//! ```
//!
//! This function is obviously `unsafe` since the power to hide borrows could easily break other safe
//! abstractions such as `cap!`:
//!
//! ```rust
//! autoken::cap! {
//!     pub MyCap = Vec<u32>;
//! }
//!
//! fn demo() {
//!     let first_three = &autoken::cap!(ref MyCap)[0..3];
//!
//!     // This compiles because we don't see the mutable borrow of `MyCap` here!
//!     unsafe {
//!         autoken::absorb::<autoken::Mut<MyCap>, _>(|| {
//!             autoken::cap!(mut MyCap).push(3);
//!         });
//!     }
//!     eprintln!("The first three elements are: {first_three:?}");
//! }
//! ```
//!
//! These primitives are all that is required to implement a context-passing mechanism like `cap!`:
//! the fetch form of `cap!` uses `tie!` to declare the fact that the reference it returns is tied
//! to some context item defined outside of the function and the binding form of `cap!` uses absorb
//! to indicate that borrows inside its block don't affect its caller. Feel free to read the macro's
//! source code for all the gory details!
//!
//! ## Semantics of Generics
//!
//! AuToken takes a ["substitution failure is not an error"](https://en.wikipedia.org/wiki/Substitution_failure_is_not_an_error)
//! approach to handling generics. That is, rather than checking that all possible substitutions for
//! a generic parameter are valid as the function is first defined, it checks only the substitutions
//! that are actually used. As an example, this function, by itself, passes AuToken's validation:
//!
//! ```rust
//! fn my_func<T, V>() {
//!     let a = autoken::BorrowsOne::<T>::acquire_mut();
//!     let b = autoken::BorrowsOne::<V>::acquire_mut();
//!     let _ = (a, b);
//! }
//! ```
//!
//! In most scenarios, this function type-checks:
//!
//! ```rust
//! # fn my_func<T, V>() {
//! #     let a = autoken::BorrowsOne::<T>::acquire_mut();
//! #     let b = autoken::BorrowsOne::<V>::acquire_mut();
//! #     let _ = (a, b);
//! # }
//! fn demo_works() {
//!     my_func::<u32, i32>();  // Ok!
//! }
//! ```
//!
//! However, if you happen to substitute `T` and `V` such that `T = V`...
//!
//! ```rust
//! # fn my_func<T, V>() {
//! #     let a = autoken::BorrowsOne::<T>::acquire_mut();
//! #     let b = autoken::BorrowsOne::<V>::acquire_mut();
//! #     let _ = (a, b);
//! # }
//! fn demo_breaks() {
//!     my_func::<u32, u32>();
//! }
//! ```
//!
//! ...a scary compiler error pops out!
//!
//! ```plain_text
//! error: conflicting borrows on token u32
//!  --> src/main.rs:5:13
//!   |
//! 4 |     let a = autoken::BorrowsOne::<T>::acquire_mut();
//!   |             --------------------------------------- value first borrowed mutably
//! 5 |     let b = autoken::BorrowsOne::<V>::acquire_mut();
//!   |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ value later borrowed mutably
//!   |
//!   = help: first borrow originates from Borrows::<Mut<u32>>::acquire_mut::<'_>
//!   = help: later borrow originates from Borrows::<Mut<u32>>::acquire_mut::<'_>
//! ```
//!
//! Generic dispatches, too, have some weird generic behavior. In this case, the body of `my_func`
//! makes it such that the provided closure cannot borrow the `u32` token mutably.
//!
//! ```rust
//! fn my_func(f: impl FnOnce()) {
//!     let v = autoken::BorrowsOne::<u32>::acquire_mut();
//!     f();
//!     let _ = v;
//! }
//!
//! fn demo_works() {
//!     my_func(|| {
//!         let _ = autoken::BorrowsOne::<i32>::acquire_mut();
//!     });
//! }
//!
//! fn demo_breaks() {
//!     my_func(|| {
//!         let _ = autoken::BorrowsOne::<u32>::acquire_mut();
//!     });
//! }
//! ```
//!
//! ```plain_text
//! error: conflicting borrows on token u32
//!  --> src/main.rs:3:5
//!   |
//! 2 |     let v = autoken::BorrowsOne::<u32>::acquire_mut();
//!   |             ----------------------------------------- value first borrowed mutably
//! 3 |     f();
//!   |     ^^^ value later borrowed mutably
//!   |
//!   = help: first borrow originates from Borrows::<Mut<u32>>::acquire_mut::<'_>
//!   = help: later borrow originates from demo_breaks::{closure#0}
//! ```
//!
//! Even bare generic dispatch can bite you in some scenarios. In this case, any implementation of
//! `MyTrait` which ties `'a` to any token mutably will fail to compile if passed to `my_func`.
//!
//! ```rust
//! trait MyTrait {
//!    fn run<'a>(self) -> &'a ();
//! }
//!
//! fn my_func(f: impl MyTrait, g: impl MyTrait) {
//!    let a = f.run();
//!    let b = g.run();
//!    let _ = (a, b);
//! }
//!
//! fn demo_works() {
//!     struct Works;
//!
//!     impl MyTrait for Works {
//!         fn run<'a>(self) -> &'a () {
//!             &()
//!         }
//!     }
//!
//!     my_func(Works, Works);
//! }
//!
//! fn demo_breaks() {
//!     struct Breaks;
//!
//!     impl MyTrait for Breaks {
//!         fn run<'a>(self) -> &'a () {
//!             autoken::tie!('a => mut u32);
//!             &()
//!         }
//!     }
//!
//!     my_func(Breaks, Breaks);
//! }
//! ```
//!
//! ```plain_text
//! error: conflicting borrows on token u32
//!  --> src/main.rs:7:13
//!  |
//! 6 |     let a = f.run();
//!  |             ------- value first borrowed mutably
//! 7 |     let b = g.run();
//!  |             ^^^^^^^ value later borrowed mutably
//!  |
//!  = help: first borrow originates from <Breaks as MyTrait>::run::<'_>
//!  = help: later borrow originates from <Breaks as MyTrait>::run::<'_>
//! ```
//!
//! Even just unsizing functions can introduce restrictions on which values can be substituted into
//! a generic parameter. In this case, `my_func`'s unsizing of the provided closure implies a
//! restriction that the provided closure can't borrow any tokens whatsoever!
//!
//! ```rust
//! fn my_func(mut f: impl FnMut()) {
//!     let f: &mut dyn FnMut() = &mut f;
//! }
//!
//! fn demo_works() {
//!     my_func(|| {
//!         eprintln!("Everything is okay!");
//!     });
//! }
//!
//! fn demo_breaks() {
//!     my_func(|| {
//!         eprintln!("Uh oh...");
//!         autoken::BorrowsOne::<u32>::acquire_mut();
//!     });
//! }
//! ```
//!
//! ```plain_text
//! error: cannot unsize this function because it borrows unabsorbed tokens
//!   --> src/main.rs:2:31
//!    |
//! 2  |     let f: &mut dyn FnMut() = &mut f;
//!    |                               ^^^^^^
//!    |
//!    = note: uses &mut u32.
//!
//! note: demo_breaks::{closure#0} was unsized
//!   --> src/main.rs:12:13
//!    |
//! 12 |     my_func(|| {
//!    |             ^^
//! ```
//!
//! These semantic versioning foot-guns are quite scary (and the poor diagnostics for them certainly
//! don't help!) but this level of flexibility with generics also allows all sorts of powerful patterns
//! to be implemented generically in AuToken. The big open question in AuToken's design is how to
//! remove these foot-guns without also blunting AuToken's expressiveness.
//!
//! ## Neat Recipes
//!
//! TODO
//!
//! ## Limitations
//!
//! **To-Do:** Document tool limitations (i.e. input parameters aren't supported yet, you can't upgrade
//! to newer versions of `rustc`, you probably shouldn't publish crates written with AuToken, the
//! compiler is a bit slow because it duplicates a ton of work).
//!
//! **To-Do:** Breaks semver and how we might fix this in an actual language feature.

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
