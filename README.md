# AuToken

> [!NOTE]
> This version is much newer than and very different to the version published on [crates.io](https://crates.io/crates/autoken).

<!--
N.B. This README is generated from the crate documentation of `src/userland` and is mirrored in
`README.md` and `src/userland/README.md`.
-->

<!-- cargo-rdme start -->

A rust-lang compiler tool adding support for automated borrow-aware context passing.

```rust
autoken::cap! {
    pub MyCap = Vec<u32>;
}

fn main() {
    let mut my_vec = vec![1, 2, 3, 4];

    autoken::cap! {
        MyCap: &mut my_vec
    =>
        do_something();
    }
}

fn do_something() {
    with_indirection();
}

fn with_indirection() {
    let my_vec = autoken::cap!(ref MyCap);
    let first_three = &my_vec[0..3];
    add_number(5);
    eprintln!("The first three elements were {first_three:?}");
}

fn add_number(number: u32) {
    autoken::cap!(mut MyCap).push(number);
}
```

```plain_text
error: conflicting borrows on token MyCap
  --> src/main.rs:22:5
   |
20 |     let my_vec = autoken::cap!(ref MyCap);
   |                  ------------------------ value first borrowed immutably
21 |     let first_three = &my_vec[0..3];
22 |     add_number(5);
   |     ^^^^^^^^^^^^^ value later borrowed mutably
   |
   = help: first borrow originates from Borrows::<Mut<MyCap>>::acquire_ref::<'_>
   = help: later borrow originates from add_number
```

## Installation

AuToken is both a custom compiler plugin called `cargo-autoken` and a regular cargo crate called
`autoken` whose documentation you are currently reading. It is possible to compile projects
using `autoken` with a stock `rustc` compiler since `cargo-autoken` only modifies the compiler's
validation logic—not its code generation logic. However, this will be terribly unsound since no
validation will actually occur!

To fix that, let's actually install and use `cargo-autoken`.

Currently, AuToken must be installed directly through the [AuToken git repository](https://github.com/radbuglet/autoken).
To install the tool, clone the repository, enter its root directory, and run...

```plain_text
cargo install -Z bindeps --path src/cargo
```

Now that `cargo-autoken` is installed, let's set up a project.

### Project Setup

AuToken requires a very specific version of the Rust compiler to work. Let's pin our project to
that version by creating a `rust-toolchain.toml` file in your project's root directory.

```toml
[toolchain]
channel = "nightly-2024-03-10"
```

Next, let's install the `autoken` crate from the [AuToken git repository](https://github.com/radbuglet/autoken).
We can do this by adding the following line to your `Cargo.toml` file:

```toml
autoken = { git = "https://github.com/Radbuglet/autoken.git", rev = "<optional pinned revision>" }
```

Finally, let's create a script to compile and run the project. You can any task runner you like,
such as `Makefile` or [`just`](https://github.com/casey/just). This example script is a `Makefile`
—you can create an equivalent `Justfile` by removing the `.PHONY` directive.

```makefile
.PHONY: run, run-unchecked

run:
	cargo autoken check
	cargo run

run-unchecked:
	cargo run
```

And that it! Have fun!

## High-Level Usage

The easiest way to use AuToken is through the [`cap!`](https://docs.rs/autoken/latest/autoken/macro.cap.html) macro. `cap!` allows users to
define, provide, and fetch a new implicitly-passed context item sometimes called a "capability."

Let's start by defining our first capability:

```rust
autoken::cap! {
    pub MyCap = Vec<u32>;
}
```

From there, we can define functions that depend on that value.

```rust
fn add_number(value: u32) {
    // This form of `cap!` fetches a reference to the value from the function call context.
    autoken::cap!(mut MyCap).push(value);

    // You can call other functions depending on this context without having to explicitly
    // forward it.
    eprintln!("The list is now {:?}, skip a few, {:?}", first_n_numbers(2), last_number());
}

fn last_number() -> Option<u32> {
    autoken::cap!(ref MyCap).last().copied()
}

fn first_n_numbers<'a>(count: usize) -> &'a [u32] {
    // This directive, meanwhile, declares the fact that `'a` depends on a borrow of `MyCap`.
    autoken::tie!('a => ref MyCap);

    &autoken::cap!(ref MyCap)[0..count]
}
```

To be able to call those functions, we need to inject an actual `Vec<u32>` instance into the `MyCap`
context. We can do so using one final form of `cap!`:

```rust
fn main() {
    autoken::cap! {
        MyCap: &mut vec![1, 2, 3]
    =>
        eprintln!("The last number is {:?}", last_number());
        add_number(4);
        eprintln!("Our four numbers are {:?}", first_n_numbers(4));
    }
}
```

AuToken can inject context through any static call site, even if it's a `trait` method or even
an externally-defined function. For example, this works because we're "passing" the `MyCap`
reference through the closure every time it's called...

```rust
autoken::cap! {
    pub MyCap = Vec<u32>;
}

fn call_two(a: impl FnOnce(), b: impl FnOnce()) {
    a();
    b();
}

fn demo() {
    call_two(
        || autoken::cap!(mut MyCap).push(3),
        || autoken::cap!(mut MyCap).push(4),
    );
}
```

...while this code fails to compile because both closures need to capture `my_cap`:

```rust
fn demo() {
    let mut my_values = vec![1, 2];

    call_two(
        || my_values.push(3),
        || my_values.push(4),
    );
}
```

```plain_text
error[E0499]: cannot borrow `my_values` as mutable more than once at a time
  --> src/lib.rs:5:9
   |
10 |     call_two(
   |     -------- first borrow later used by call
11 |         || my_values.push(3),
   |         -- --------- first borrow occurs due to use of `my_values` in closure
   |         |
   |         first mutable borrow occurs here
12 |         || my_values.push(4),
   |         ^^ --------- second borrow occurs due to use of `my_values` in closure
   |         |
   |         second mutable borrow occurs here
```

What AuToken cannot inject through, however, is dynamically dispatched function calls. AuToken
assumes that every function or trait member which has been unsized depends on nothing from its
caller. Hence, if you try to unsize a function which expects a value to be in its context, the
line will refuse to compile:

```rust
autoken::cap! {
    pub MyCap = u32;
}

fn increment_counter() {
    *autoken::cap!(mut MyCap) += 1;
}

fn demo() {
    // Calling `increment_counter` statically is fine, assuming `MyCap` is in the context.
    increment_counter();

    // ...but unsizing `increment_counter` is not!
    let my_func: fn() = increment_counter;
}
```

```plain_text
error: cannot unsize this function because it borrows unabsorbed tokens
  --> src/main.rs:11:25
   |
11 |     let my_func: fn() = increment_counter;
   |                         ^^^^^^^^^^^^^^^^^
   |
   = note: uses &mut MyCap.

note: increment_counter was unsized
  --> src/main.rs:4:1
   |
4  | fn increment_counter() {
   | ^^^^^^^^^^^^^^^^^^^^^^
```

If, for some reason, you need to "smuggle" access to a `cap!` past a dynamic dispatch boundary,
you can use the [`Borrows`](crate:Borrows) object and its alias [`BorrowsOne`](crate:BorrowsOne).

`Borrows` is an object representing a borrow of a set of capabilities. If you have an mutable
reference to it, you are effectively borrowing that entire set of capabilities mutably. You can
create a `Borrows` object from the surrounding implicit capability context like so:

```rust
fn demo_1() {
    let borrows = autoken::BorrowsOne::<MyCap>::acquire_mut();
    let mut increment = || {
        borrows.absorb(|| {
            increment_counter();
        });
    };
    let increment_dyn: &mut dyn FnMut() = &mut increment;

    increment_dyn();
}

fn demo_2() {
    let increment = |token: &mut autoken::BorrowsOne<MyCap>| {
        token.absorb(|| {
            increment_counter();
        });
    };
    let increment: fn(&mut autoken::BorrowsOne<MyCap>) = increment;

    increment(autoken::BorrowsOne::<MyCap>::acquire_mut());
}
```

## Low-Level Usage

Internally, [`cap!`](https://docs.rs/autoken/latest/autoken/macro.cap.html) is not a primitive feature of AuToken. Instead, it is built
entirely in-userland using `thread_local!` with the help of two custom analysis intrinsics:
[`tie!`](https://docs.rs/autoken/latest/autoken/macro.tie.html) and [`absorb`](https://docs.rs/autoken/latest/autoken/fn.absorb.html).

`tie!` is a macro which can be used in the body of a function to declare the fact that a lifetime
in the function's return type is tied to a borrow of some global token type. For example, you
could write:

```rust
pub struct MySingleton {
    ...
}

pub fn get_singleton<'a>() -> &'a mut MySingleton {
    autoken::tie!('a => mut MySingleton);

    unimplemented!();
}
```

...and no one would be able to use `get_singleton` to acquire multiple mutable references to
`MySingleton` simultaneously since doing so would require borrowing the `MySingleton` "token"
mutably more than once.

```rust
fn demo() {
    let singleton_1 = get_singleton();
    let singleton_2 = get_singleton();
    let _ = singleton_1;
}
```

```plain_text
error: conflicting borrows on token MySingleton
  --> src/main.rs:11:23
   |
10 |     let singleton_1 = get_singleton();
   |                       --------------- value first borrowed mutably
11 |     let singleton_2 = get_singleton();
   |                       ^^^^^^^^^^^^^^^ value later borrowed mutably
   |
   = help: first borrow originates from get_singleton::<'_>
   = help: later borrow originates from get_singleton::<'_>
```

In effect, you could think of `autoken::tie!` as introducing a new virtual parameter to your
function indicating exclusive/shared access to a given contextual resource. The code above, for
example, could be logically desugared as:

```rust
pub struct MySingleton {
    ...
}

pub fn get_singleton<'a>(access_perms: &'a mut Token<MySingleton>) -> &'a mut MySingleton {
    //                                   ^^ this is what the `'a` means in `tie!('a => mut MySingleton)`
    //                                                ^^^^^^^^^^^ and this is what the `MySingleton` means.
    unimplemented!();
}

fn demo(access_perms: &mut Token<MySingleton>) {
    //                ^^^^^^^^^^^^^^^^^^^^^^^ The existence of this parameter is inferred from the
    //                                        call graph. Note that its lifetime is anonymous since
    //                                        it wasn't explicitly tied to anything.
    let singleton_1 = get_singleton(access_perms);
    let singleton_2 = get_singleton(access_perms);
    let _ = singleton_1;
}
```

```plain_text
error[E0499]: cannot borrow `*access_perms` as mutable more than once at a time
  --> src/main.rs:18:37
   |
17 |     let singleton_1 = get_singleton(access_perms);
   |                                     ------------ first mutable borrow occurs here
18 |     let singleton_2 = get_singleton(access_perms);
   |                                     ^^^^^^^^^^^^ second mutable borrow occurs here
19 |     let _ = singleton_1;
   |             ----------- first borrow later used here
```

But how do we ensure that these tokens actually come from somewhere like a `cap!` block? This
is where the second mechanism comes in: `absorb`.

If `absorb` didn't exist, any attempt at running code involving a `tie!` directive would end in
this compile-time error:

```rust
struct MySingleton {}

fn get_singleton<'a>() -> &'a mut MySingleton {
    autoken::tie!('a => mut MySingleton);
    unimplemented!();
}

fn main() {
    get_singleton();
}
```

```plain_text
error: cannot use this main function because it borrows unabsorbed tokens
 --> src/main.rs:8:1
  |
8 | fn main() {
  | ^^^^^^^^^
  |
  = note: uses &mut MySingleton.
```

This is because `main` functions, `extern` functions, and unsized functions/methods are not
permitted to request any tokens. Hence, in order to call a function with a `tie!` directive, we
must somehow get rid of that request for a token. We can do that with `absorb`.

`absorb` absorbs the existence of a token's borrow, hiding it from its caller. In this case, if
we wanted to give our main function the ability to call `get_singleton`, we could wrap the call
in an `absorb` call like so:

```rust
struct MySingleton {}

fn get_singleton<'a>() -> &'a mut MySingleton {
    autoken::tie!('a => mut MySingleton);
    unimplemented!();
}

fn main() {
    unsafe {
        autoken::absorb::<autoken::Mut<MySingleton>, ()>(|| {
            get_singleton();
        });
    }
}
```

This function is obviously `unsafe` since the power to hide borrows could easily break other safe
abstractions such as `cap!`:

```rust
autoken::cap! {
    pub MyCap = Vec<u32>;
}

fn demo() {
    let first_three = &autoken::cap!(ref MyCap)[0..3];

    // This compiles because we don't see the mutable borrow of `MyCap` here!
    unsafe {
        autoken::absorb::<autoken::Mut<MyCap>, _>(|| {
            autoken::cap!(mut MyCap).push(3);
        });
    }
    eprintln!("The first three elements are: {first_three:?}");
}
```

These primitives are all that is required to implement a context-passing mechanism like `cap!`:
the fetch form of `cap!` uses `tie!` to declare the fact that the reference it returns is tied
to some context item defined outside of the function and the binding form of `cap!` uses absorb
to indicate that borrows inside its block don't affect its caller. Feel free to read the macro's
source code for all the gory details!

## Semantics of Generics

AuToken takes a ["substitution failure is not an error"](https://en.wikipedia.org/wiki/Substitution_failure_is_not_an_error)
approach to handling generics. That is, rather than checking that all possible substitutions for
a generic parameter are valid as the function is first defined, it checks only the substitutions
that are actually used. As an example, this function, by itself, passes AuToken's validation:

```rust
fn my_func<T, V>() {
    let a = autoken::BorrowsOne::<T>::acquire_mut();
    let b = autoken::BorrowsOne::<V>::acquire_mut();
    let _ = (a, b);
}
```

In most scenarios, this function type-checks:

```rust
fn demo_works() {
    my_func::<u32, i32>();  // Ok!
}
```

However, if you happen to substitute `T` and `V` such that `T = V`...

```rust
fn demo_breaks() {
    my_func::<u32, u32>();
}
```

...a scary compiler error pops out!

```plain_text
error: conflicting borrows on token u32
 --> src/main.rs:5:13
  |
4 |     let a = autoken::BorrowsOne::<T>::acquire_mut();
  |             --------------------------------------- value first borrowed mutably
5 |     let b = autoken::BorrowsOne::<V>::acquire_mut();
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ value later borrowed mutably
  |
  = help: first borrow originates from Borrows::<Mut<u32>>::acquire_mut::<'_>
  = help: later borrow originates from Borrows::<Mut<u32>>::acquire_mut::<'_>
```

Generic dispatches, too, have some weird generic behavior. In this case, the body of `my_func`
makes it such that the provided closure cannot borrow the `u32` token mutably.

```rust
fn my_func(f: impl FnOnce()) {
    let v = autoken::BorrowsOne::<u32>::acquire_mut();
    f();
    let _ = v;
}

fn demo_works() {
    my_func(|| {
        let _ = autoken::BorrowsOne::<i32>::acquire_mut();
    });
}

fn demo_breaks() {
    my_func(|| {
        let _ = autoken::BorrowsOne::<u32>::acquire_mut();
    });
}
```

```plain_text
error: conflicting borrows on token u32
 --> src/main.rs:3:5
  |
2 |     let v = autoken::BorrowsOne::<u32>::acquire_mut();
  |             ----------------------------------------- value first borrowed mutably
3 |     f();
  |     ^^^ value later borrowed mutably
  |
  = help: first borrow originates from Borrows::<Mut<u32>>::acquire_mut::<'_>
  = help: later borrow originates from demo_breaks::{closure#0}
```

Even bare generic dispatch can introduce restrictions on type parameters substitutions. In this
case, any implementation of `MyTrait` which ties `'a` to any token mutably will fail to compile
if passed to `my_func`.

```rust
trait MyTrait {
   fn run<'a>(self) -> &'a ();
}

fn my_func(f: impl MyTrait, g: impl MyTrait) {
   let a = f.run();
   let b = g.run();
   let _ = (a, b);
}

fn demo_works() {
    struct Works;

    impl MyTrait for Works {
        fn run<'a>(self) -> &'a () {
            &()
        }
    }

    my_func(Works, Works);
}

fn demo_breaks() {
    struct Breaks;

    impl MyTrait for Breaks {
        fn run<'a>(self) -> &'a () {
            autoken::tie!('a => mut u32);
            &()
        }
    }

    my_func(Breaks, Breaks);
}
```

```plain_text
error: conflicting borrows on token u32
 --> src/main.rs:7:13
  |
6 |     let a = f.run();
  |             ------- value first borrowed mutably
7 |     let b = g.run();
  |             ^^^^^^^ value later borrowed mutably
  |
 = help: first borrow originates from <Breaks as MyTrait>::run::<'_>
 = help: later borrow originates from <Breaks as MyTrait>::run::<'_>
```

Same goes with just unsizing functions. In this case, `my_func`'s unsizing of the provided closure
implies a restriction that the provided closure can't borrow any tokens whatsoever!

```rust
fn my_func(mut f: impl FnMut()) {
    let f: &mut dyn FnMut() = &mut f;
}

fn demo_works() {
    my_func(|| {
        eprintln!("Everything is okay!");
    });
}

fn demo_breaks() {
    my_func(|| {
        eprintln!("Uh oh...");
        autoken::BorrowsOne::<u32>::acquire_mut();
    });
}
```

```plain_text
error: cannot unsize this function because it borrows unabsorbed tokens
  --> src/main.rs:2:31
   |
2  |     let f: &mut dyn FnMut() = &mut f;
   |                               ^^^^^^
   |
   = note: uses &mut u32.

note: demo_breaks::{closure#0} was unsized
  --> src/main.rs:12:13
   |
12 |     my_func(|| {
   |             ^^
```

While the former two are merely semantic versioning foot-guns, the latter are genuine
semantic-versioning *breakers* since they make what used to be non-breaking changes in today's Rust
into breaking changes. At the same time, this level of flexibility with generics allows all
sorts of powerful patterns to be implemented generically in AuToken. Hence, the big open question
in AuToken's design is how to remove these foot-guns without also blunting its expressiveness.

## Neat Recipes

One of the coolest uses of AuToken, in my opinion, is integrating it with the [`generational_arena`](https://docs.rs/generational-arena/latest/generational_arena/)
crate. This crate implements what is essentially a `HashMap` from numeric handles to values but
in a way which is considerably more efficient. Since numeric handles are freely copyable, they
can serve as ad hoc shared mutable references. Their only issue is that, in order to dereference
them, you must carry around a reference to the arena mapping those handles to their values.

This is where AuToken comes in. Since `Deref` implementations can tie their output to a token
borrow, we can implement a version of those handles which acts like a smart pointer like so:

```rust
use generational_arena::Arena;

use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

// Extracts the capability containing the arena used by a given `Pointee`
type PointeeCap<T> = <T as Pointee>::Cap;

// A trait implemented by all objects that have an arena that can be pointed into by a `Handle.`
trait Pointee: Sized {
    type Cap;

    fn arena<'a>() -> &'a Arena<Self>;

    fn arena_mut<'a>() -> &'a mut Arena<Self>;
}

// A smart pointer which is `Copy`, `Deref`, `DerefMut`, and has a `destroy()` method! 🙀
struct Handle<T: Pointee> {
    _ty: PhantomData<fn(T) -> T>,
    handle: generational_arena::Index,
}

impl<T: Pointee> Copy for Handle<T> {}

impl<T: Pointee> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Pointee> Handle<T> {
    pub fn new(value: T) -> Self {
        Self {
            _ty: PhantomData,
            handle: T::arena_mut().insert(value),
        }
    }

    pub fn destroy(self) {
        T::arena_mut().remove(self.handle);
    }
}

impl<T: Pointee> Deref for Handle<T> {
    type Target = T;

    fn deref<'a>(&'a self) -> &'a T {
        // The `unsafe` keyword is admittedly a bit weird. The TLDR is that it's a workaround for
        // a difficult-to-fix analysis bug in AuToken.
        autoken::tie!(unsafe 'a => ref T::Cap);
        &T::arena()[self.handle]
    }
}

impl<T: Pointee> DerefMut for Handle<T> {
    fn deref_mut<'a>(&'a mut self) -> &'a mut T {
        autoken::tie!(unsafe 'a => mut T::Cap);
        &mut T::arena_mut()[self.handle]
    }
}
```

Here's how we can use it!

```rust
// First, let's implement `Pointee` on `Vec<u32>`. This could be turned into a simple decl-macro.
const _: () = {
    autoken::cap! {
        pub Cap = Arena<Vec<u32>>;
    }

    impl Pointee for Vec<u32> {
        type Cap = Cap;

        fn arena<'a>() -> &'a Arena<Self> {
            autoken::tie!('a => ref Cap);
            autoken::cap!(ref Cap)
        }

        fn arena_mut<'a>() -> &'a mut Arena<Self> {
            autoken::tie!('a => mut Cap);
            autoken::cap!(mut Cap)
        }
    }
};

// Now, we can start using the handle as if it were any other smart pointer.
fn do_something(mut f: Handle<Vec<u32>>) {
    f.push(4);
    do_something_else(f);
    f.push(5);
}

fn do_something_else(f: Handle<Vec<u32>>) {
    eprintln!("Values: {:?}", &*f);
}

fn main() {
    // ...all we have to do to call these methods is inject the right arena into the context!
    autoken::cap! {
        PointeeCap<Vec<u32>>: &mut Arena::new()
    =>
        let handle = Handle::new(vec![1, 2, 3]);
        do_something(handle);
        handle.destroy();
    }
}
```

Let's use this newfound power to implement the borrow-checker's arch-nemesis: the linked list.

```rust
// This feature allows us to use the `self: Handle<Self>` syntax, which is convenient but not
// required whatsoever.
#![feature(arbitrary_self_types)]

struct Node {
    value: u32,
    prev: Option<Handle<Self>>,
    next: Option<Handle<Self>>,
}

pointee!(Node);

impl Node {
    pub fn new(value: u32) -> Self {
        Self {
            value,
            prev: None,
            next: None,
        }
    }

    pub fn remove(mut self: Handle<Self>) {
        if let Some(mut prev) = self.prev {
            prev.next = self.next;
        }

        if let Some(mut next) = self.next {
            next.prev = self.prev;
        }
    }

    pub fn insert_right(mut self: Handle<Self>, mut next: Handle<Self>) {
        next.remove();

        if let Some(mut old_next) = self.next {
            old_next.prev = Some(next);
        }

        next.next = self.next;
        next.prev = Some(self);
        self.next = Some(next);
    }

    pub fn iter(self: Handle<Self>) -> impl Iterator<Item = Handle<Self>> {
        let mut state = Some(self);

        std::iter::from_fn(move || {
            let curr = state?;
            state = curr.next;
            Some(curr)
        })
    }
}

fn main() {
    autoken::cap! {
        PointeeCap<Node>: &mut Arena::new()
    =>
        let first = Handle::new(Node::new(1));
        let second = Handle::new(Node::new(2));
        let third = Handle::new(Node::new(3));

        first.insert_right(second);
        second.insert_right(third);

        for node in first.iter() {
            eprintln!("Value: {}", node.value);
        }

        second.remove();

        for node in first.iter() {
            eprintln!("Value: {}", node.value);
        }
    }
}
```

```plain_text
Value: 1
Value: 2
Value: 3
Value: 1
Value: 3
```

Neat, huh?

## Limitations

AuToken is held together with duct-tape and dreams.

<details><summary><i style="cursor: pointer">Somehow don't believe me yet?</i> </summary>

```rust
// HACK: `get_body_with_borrowck_facts` does not use `tcx.local_def_id_to_hir_id(def).owner` to
// determine the origin of the inference context like regular `mir_borrowck` does.
//
// Here's the source of `get_body_with_borrowck_facts`:
//
// ```
// pub fn get_body_with_borrowck_facts(
//     tcx: TyCtxt<'_>,
//     def: LocalDefId,
//     options: ConsumerOptions,
// ) -> BodyWithBorrowckFacts<'_> {
//     let (input_body, promoted) = tcx.mir_promoted(def);
//     let infcx = tcx.infer_ctxt().with_opaque_type_inference(DefiningAnchor::Bind(def)).build();
//     let input_body: &Body<'_> = &input_body.borrow();
//     let promoted: &IndexSlice<_, _> = &promoted.borrow();
//     *super::do_mir_borrowck(&infcx, input_body, promoted, Some(options)).1.unwrap()
// }
// ```
//
// ...and here's the (abridged) source of `mir_borrowck`:
//
// ```
// fn mir_borrowck(tcx: TyCtxt<'_>, def: LocalDefId) -> &BorrowCheckResult<'_> {
//     let (input_body, promoted) = tcx.mir_promoted(def);
//     let input_body: &Body<'_> = &input_body.borrow();
//
//     // (erroneous input rejection here)
//
//     let hir_owner = tcx.local_def_id_to_hir_id(def).owner;
//     let infcx =
//         tcx.infer_ctxt().with_opaque_type_inference(DefiningAnchor::Bind(hir_owner.def_id)).build();
//
//     let promoted: &IndexSlice<_, _> = &promoted.borrow();
//     let opt_closure_req = do_mir_borrowck(&infcx, input_body, promoted, None).0;
//     tcx.arena.alloc(opt_closure_req)
// }
// ```
//
// So long as we can pass the owner's `DefId` to `get_body_with_borrowck_facts` but the shadow's body
// and promoted set, we can emulate the correct behavior of `mir_borrowck`—which is exactly what this
// Abomination To Everything Good does.
pub fn get_body_with_borrowck_facts_but_sinful(
    tcx: TyCtxt<'_>,
    shadow_did: LocalDefId,
    options: ConsumerOptions,
) -> BodyWithBorrowckFacts<'_> {
    // Begin by stealing the `mir_promoted` for our shadow function.
    let (shadow_body, shadow_promoted) = tcx.mir_promoted(shadow_did);

    let shadow_body = shadow_body.steal();
    let shadow_promoted = shadow_promoted.steal();

    // Now, let's determine the `orig_did`.
    let hir_did = tcx.local_def_id_to_hir_id(shadow_did).owner.def_id;

    // Modify the instance MIR in place. This doesn't violate query caching because steal is
    // interior-mutable and stable across queries. We're not breaking caching anywhere else since
    // `get_body_with_borrowck_facts` is just a wrapper around `do_mir_borrowck`.
    let (orig_body, orig_promoted) = tcx.mir_promoted(hir_did);

    let orig_body = unpack_steal(orig_body);
    let orig_promoted = unpack_steal(orig_promoted);

    let old_body = std::mem::replace(&mut *orig_body.write(), Some(shadow_body));
    let _dg1 = scopeguard::guard(old_body, |old_body| {
        *orig_body.write() = old_body;
    });

    let old_promoted = std::mem::replace(&mut *orig_promoted.write(), Some(shadow_promoted));
    let _dg2 = scopeguard::guard(old_promoted, |old_promoted| {
        *orig_promoted.write() = old_promoted;
    });

    // Now, do the actual borrow-check, replacing back the original MIR once the operation is done.
    get_body_with_borrowck_facts(tcx, hir_did, options)
}

fn unpack_steal<T>(steal: &Steal<T>) -> &RdsRwLock<Option<T>> {
    unsafe {
        // Safety: None. This is technically U.B.
        &*(steal as *const Steal<T> as *const RdsRwLock<Option<T>>)
    }
}
```

</details>

Here's what that means to you:

- There are almost certainly many bugs and soundness holes from incorrect use of the `rustc` API.
- The tool is much slower than stock `rustc`. This is mostly a result of my awful serializer and
  the lack of incremental analysis.
- You are stuck with a very specific build of `rustc` and I wouldn't try upgrading it without a
  massive suite of compile-tests to check your work because so much of this tool relies on
  the specific implementation details of the rust compiler for which this tool was built.
- This tool breaks semantic versioning (see the [Semantics of Generics](#semantics-of-generics)
  section for details).
- This tool emits diagnostics which are just plain awful—especially if you work with generic code.
- Tying tokens to lifetimes appearing in the input position is potentially unsound. The tool
  should warn you of most of these cases and there are escape hatches (see: the `unsafe` keyword
  in `tie!` that showed up in the ["Neat Recipes"](#neat-recipes) example) but it's still pretty
  goofy.
- This crate does not support `#[no_std]` environments.

All in all, I would mainly use this tool as just a playground for exploring the design implications
of adding a context passing feature to the Rust programming language since there's no better way
to explore the effects of a potential language extension than to play around with it. You probably
shouldn't be using this in production.

## Special Thanks

I owe so much to the wonderful folks of the [`#dark-arts` channel](https://discord.gg/rust-lang-community)
of the "Rust Programming Language Community" Discord server and of the [rust-lang Zulip chat](https://rust-lang.zulipchat.com/).
Thank you all, so very much, for your help!

<!-- cargo-rdme end -->
