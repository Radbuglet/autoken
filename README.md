# AuToken

<!-- cargo-rdme start -->

A rust-lang static analysis tool to automatically check for runtime borrow violations.

```rust
use autoken::MutableBorrow;

fn foo() {
    let _my_guard = MutableBorrow::<u32>::new();

    // Autoken statically issues a warning here because we attempted to call a function which could
    // mutably borrow `u32` while we already have an active mutable borrow from `_my_guard`.
    bar();
}

fn bar() {
    let _my_guard_2 = MutableBorrow::<u32>::new();
}
```

```plain_text
warning: called a function expecting at most 0 mutable borrows of type u32 but was called in a scope with at least 1
 --> src/main.rs:8:5
  |
8 |     bar();
  |     ^^^^^
````

### Checking Projects

AuToken is a framework for adding static analysis of runtime borrowing to your crate. If you are
an end-user of a crate with integrations with AuToken and wish to check your project with the
tool, this is the section for you! If, instead, you're building a crate and wish to integrate with
AuToken, you should skip to the [Integrating AuToken](#integrating-autoken) section.

If you wish to install this tool through `cargo`, you should run a command like:

```bash
cargo +nightly-2023-09-08 install cargo-autoken -Z bindeps
```

This will likely require you to faff around with rustup toolchains. Because this process could
vary from user to user, the best instructions for setting up an appropriate toolchain are provided
by rustup, cargo, and rust.

If you wish to install from source, assuming your current working directory is the same as the
[repository](https://github.com/radbuglet/autoken)'s README, `cargo-autoken` can be installed
like so:

```bash
cargo install --path src/cargo -Z bindeps
```

You can run AuToken validation on a target binary crate by running:

```bash
cargo autoken check
```

...in its directory.

Have fun!

### Ignoring False Positives

AuToken is, by nature, very conservative. After all, its whole job is to ensure that only one
borrow of a given type exists at a given time, even if you're potentially borrowing from several
different sources at once!

```rust
let cell_1 = MyCell::new(1u32);
let cell_2 = MyCell::new(2u32);

let borrow_1 = cell_1.borrow_mut();
let borrow_2 = cell_2.borrow_mut();
```

```plain_text
warning: called a function expecting at most 0 mutable borrows of type u32 but was called in a scope with at least 1
  --> src/main.rs:10:27
   |
10 |     let borrow_2 = cell_2.borrow_mut();
   |                           ^^^^^^^^^^^^
```

If you're sure you're doing something safe, you can ignore these warnings using the
[`assume_no_alias`](https://docs.rs/autoken/latest/autoken/fn.assume_no_alias.html) method.

```rust
let cell_1 = MyCell::new(1u32);
let cell_2 = MyCell::new(2u32);

let borrow_1 = cell_1.borrow_mut();
let borrow_2 = autoken::assume_no_alias(|| cell_2.borrow_mut());
```

See [`assume_no_alias_in`](https://docs.rs/autoken/latest/autoken/fn.assume_no_alias_in.html) and [`assume_no_alias_in_many`](https://docs.rs/autoken/latest/autoken/fn.assume_no_alias_in_many.html)
for more forms of this function.

#### Making Sense of Control Flow Errors

The weirdest diagnostic message you are likely to encounter while using AuToken is this one:

```rust
let cell_1 = MyCell::new(1u32);

let my_borrow = if some_condition {
    Some(cell_1.borrow_mut())
} else {
    None
};
```

```plain_text
warning: not all control-flow paths to this statement are guaranteed to borrow the same number of components
  --> src/main.rs:9:21
  |
9  |       let my_borrow = if some_condition {
  |  _____________________^
10 | |         Some(cell_1.borrow_mut())
11 | |     } else {
12 | |         None
13 | |     };
  | |_____^
```

This error occurs because of a fundamental limitation of AuToken's design. AuToken analyzes your
programs by traversing through the control-flow graph `rustc` generates to analyze, among other
things, borrow checking. Every time it encounters a call to [`borrow_mutably`](https://docs.rs/autoken/latest/autoken/fn.borrow_mutably.html)
or [`borrow_immutably`](https://docs.rs/autoken/latest/autoken/fn.borrow_immutably.html), it increments the theoretical number of mutable
or immutable borrows a given control flow block may have and vice versa with
[`unborrow_mutably`](https://docs.rs/autoken/latest/autoken/fn.unborrow_mutably.html) and [`unborrow_immutably`](https://docs.rs/autoken/latest/autoken/fn.unborrow_immutably.html).
If there's a divergence in control flow as introduced by an `if` statement or a `loop`, AuToken
will visit and analyze each path separately.

But what happens when those two paths join back together? How many borrows does a user have if
one path borrows `u32` mutably and the other doesn't borrow it at all? AuToken doesn't know the
answer to this question and just guesses randomly. Because this guess is probably wrong, it emits
a warning to tell you that it really can't handle code written like this.

So, if this type of code can't be analyzed by AuToken, what can be done? The best solution is to
use a method AuToken integration writers are strongly encouraged to implement: `borrow_on_loan`
(or `borrow_mut_on_loan`, or `get_mut_on_loan`... just search for `_on_loan` in the docs!). This
method ties the borrow to an externally provided [`MutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.MutableBorrow.html) instance,
which should be defined outside of all the conditional logic.

```rust
let cell_1 = MyCell::new(1u32);

let mut guard = MutableBorrow::<u32>::new();
let my_borrow = if some_condition {
    Some(cell_1.borrow_mut_on_loan(&mut guard))
} else {
    None
};
```

If this is too hard to manage, you could also strip the token of all static borrow analysis
entirely using the `strip_lifetime_analysis`
method. This is far more dangerous, however, because AuToken essentially forgets about the
existence of that borrow and potentially lets invalid borrows slip by.

```rust
let cell_1 = MyCell::new(1u32);

let my_borrow = if some_condition {
    Some(cell_1.borrow_mut().strip_lifetime_analysis())
} else {
    None
};
```

Finally, if things get *really* bad, you could ignore the entire section with [`assume_black_box`](https://docs.rs/autoken/latest/autoken/fn.assume_black_box.html).
This function is, very much, a last resort, because it prevents the static analysis tool from even
looking at anything in the called closure. You should read its documentation for details before
even thinking about touching it!

#### Potential Borrows

You may occasionally stumble across a fallible borrow method in your local AuToken-integrate crate
which takes in a [`PotentialMutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.PotentialMutableBorrow.html) or [`PotentialImmutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.PotentialImmutableBorrow.html)
"loaner" guard. The reason for these guards is somewhat similar to why we need loaner
guards for other conditionally created borrows with the added caveat that, because these borrow
guards are being used with a fallible borrow method, it is assumed that the aliasing with an
existing borrow can be handled gracefully at runtime. Because of this assumption,
`PotentialMutableBorrows` do not emit a warning if another confounding borrow guard is already
in scope.

```rust
let my_cell = MyCell::new(1u32);

let mut my_loaner_1 = PotentialMutableBorrow::<u32>::new();
let borrow_1 = my_cell.try_borrow_mut(&mut my_loaner_1).unwrap();

// This should not trigger a static analysis warning because, if the specific cell is already
// borrowed, the function returns an `Err` rather than panicking.
let mut my_loaner_2 = PotentialMutableBorrow::<u32>::new();
let not_borrow_2 = my_cell.try_borrow_mut(&mut my_loaner_2).unwrap_err();
```

If the borrow cannot be handled gracefully, one may create a [`MutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.MutableBorrow.html)
or [`ImmutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.ImmutableBorrow.html) guard and `downgrade`
it to a `PotentialMutableBorrow` or `PotentialImmutableBorrow` guard so that the static analyzer
will start reporting these potentially problematic borrows again.

```rust
let my_cell = MyCell::new(1u32);

let mut my_loaner_1 = PotentialMutableBorrow::<u32>::new();
let borrow_1 = my_cell.try_borrow_mut(&mut my_loaner_1).unwrap();

// Unlike the previous example, this code cannot handle aliasing borrows gracefully, so we should
// create a `MutableBorrow` first to get the alias check and then downgrade it for use in the
// fallible borrowing method.
let mut my_loaner_2 = MutableBorrow::<u32>::new().downgrade();
let not_borrow_2 = my_cell.try_borrow_mut(&mut my_loaner_2).unwrap();
```

### Dealing With Dynamic Dispatches

AuToken resolves dynamic dispatches by collecting all possible dispatch targets ahead of time
based around what gets unsized to what and assumes that any of those concrete types could be
called by an invocation of a given unsized type. This can occasionally be overly pessimistic.
You can help this along by making the dynamically dispatched traits more fine grained. For
example, instead of using an `FnMut(u32, i32, f32)`, you could use an
`FnMut(PhantomData<MyHandlers>, u32, i32, f32)`. Likewise, if you have a trait `MyBehavior`, you
could parameterize it by a marker generic type to make it even more fine-grained.

If something is really wrong, you could, once again, use [`assume_black_box`](https://docs.rs/autoken/latest/autoken/fn.assume_black_box.html)
to hide the unsizing coercions that create these dynamic dispatch targets. Once again, this is,
very much, a last resort and you should certainly read its documentation for details before even
thinking about touching it!

### Dealing With Foreign Code

AuToken has no clue how to deal with foreign code and just ignores it. If you have a foreign
function that calls back into userland code, you can tell AuToken that the code is, indeed,
reachable with something like this:

```rust
my_ffi_call(my_callback);

if false {  // reachability hint to AuToken
    my_callback();
}
```

## Integrating AuToken

This section is for crate developers wishing to add static analysis to their dynamic borrowing
schemes. If you're interested in using one of those crates, see the [checking projects](#checking-projects)
section.

There are four primitive borrowing functions offered by this library:

- [`borrow_mutably<T>`](https://docs.rs/autoken/latest/autoken/fn.borrow_mutably.html)
- [`borrow_immutably<T>`](https://docs.rs/autoken/latest/autoken/fn.borrow_immutably.html)
- [`unborrow_mutably<T>`](https://docs.rs/autoken/latest/autoken/fn.unborrow_mutably.html)
- [`unborrow_immutably<T>`](https://docs.rs/autoken/latest/autoken/fn.unborrow_immutably.html)

These functions, in reality, do absolutely nothing and are compiled away. However, when checked
by the custom AuToken rustc wrapper, they virtually "borrow" and "unborrow" a global token of
the type `T` and raise a warning if it is possible to violate the XOR mutability rules of that
virtual global token.

Usually, these functions aren't called directly and are instead called indirectly through their
RAII'd counterparts [`MutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.MutableBorrow.html) and [`ImmutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.ImmutableBorrow.html).

These primitives can be used to introduce additional compile-time safety to dynamically checked
borrowing and locking schemes. Here's a couple of examples:

You could make a safe wrapper around a `RefCell`...

```rust
use autoken::MutableBorrow;
use std::cell::{RefCell, RefMut};

struct MyRefCell<T> {
    inner: RefCell<T>,
}

impl<T> MyRefCell<T> {
    pub fn new(value: T) -> Self {
        Self { inner: RefCell::new(value) }
    }

    pub fn borrow_mut(&self) -> MyRefMut<'_, T> {
        MyRefMut {
            token: MutableBorrow::new(),
            sptr: self.inner.borrow_mut(),
        }
    }
}

struct MyRefMut<'a, T> {
    token: MutableBorrow<T>,
    sptr: RefMut<'a, T>,
}

let my_cell = MyRefCell::new(1u32);
let _a = my_cell.borrow_mut();

// This second mutable borrow results in an AuToken warning.
let _b = my_cell.borrow_mut();
```

```plain_text
warning: called a function expecting at most 0 mutable borrows of type u32 but was called in a scope with at least 1
  --> src/main.rs:33:22
   |
33 |     let _b = my_cell.borrow_mut();
   |                      ^^^^^^^^^^^^
````

You could make a reentrancy-protected function...

```rust
fn do_not_reenter(f: impl FnOnce()) {
    struct ISaidDoNotReenter;

    let _guard = autoken::MutableBorrow::<ISaidDoNotReenter>::new();
    f();
}

do_not_reenter(|| {
    // Whoops!
    do_not_reenter(|| {});
});
```

```plain_text
warning: called a function expecting at most 0 mutable borrows of type main::do_not_reenter::ISaidDoNotReenter but was called in a scope with at least 1
 --> src/main.rs:6:9
  |
6 |         f();
  |         ^^^
```

You could even deny an entire class of functions where calling them would be dangerous!

```rust
use autoken::{ImmutableBorrow, MutableBorrow};

struct IsOnMainThread;

fn begin_multithreading(f: impl FnOnce()) {
    let _guard = MutableBorrow::<IsOnMainThread>::new();
    f();
}

fn only_call_me_on_main_thread() {
    let _guard = ImmutableBorrow::<IsOnMainThread>::new();
    // ...
}

begin_multithreading(|| {
    // Whoops!
    only_call_me_on_main_thread();
});
```

```plain_text
warning: called a function expecting at most 0 mutable borrows of type main::IsOnMainThread but was called in a scope with at least 1
 --> src/main.rs:6:9
  |
6 |         f();
  |         ^^^
```

Pretty neat, huh.

### Dealing with Limitations

If you read the [checking projects](#checking-projects) section like I asked you not to, you'd
hear about four pretty major limitations of AuToken. While most of these limitations can be overcome
by tools provided by AuToken, the second limitation—[Control Flow Errors](#making-sense-of-control-flow-errors)—
requires a bit of help from developers wishing to integrate with AuToken. You are strongly
encouraged to read that section before this section, since it motivates the necessity for these
special method variants.

In summary:

1. For every guard object, provide a `strip_lifetime_analysis` function similar to
   `MutableBorrow`'s.
2. For every guard object, provide a way to acquire that object with a "loaner" borrow object. The
   recommended suffix for this variant is `on_loan`. The mechanism for doing so is likely very
   similar to `MutableBorrow`'s `loan` method.
3. For conditional borrow methods which check their borrow before performing it, the method should
   be made to loan a [`PotentialMutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.PotentialMutableBorrow.html) or [`PotentialImmutableBorrow`](https://docs.rs/autoken/latest/autoken/struct.PotentialImmutableBorrow.html)
   instead.

All of these methods rely on being able to convert the RAII guard's type from its originally
borrowed type to [`Nothing`](https://docs.rs/autoken/latest/autoken/struct.Nothing.html)—a special marker type in AuToken which indicates that
the borrow guard isn't actually borrowing anything. Doing this requires you to keep track of the
borrowed type at the type level since AuToken lacks the power to analyze runtime mechanisms for
doing that. Here's an example of how to accomplish this:

```rust
struct MyRefMut<'a, T, B = T> {
    //                 ^ notice the addition of this special parameter?
    token: MutableBorrow<B>,
    sptr: RefMut<'a, T>,
}
```

With that additional parameter in place, we can implement the first required method: `strip_lifetime_analysis`.
Its implementation is relatively straightforward:

```rust
use autoken::Nothing;

struct MyRefMut<'a, T, B = T> {
    token: MutableBorrow<B>,
    sptr: RefMut<'a, T>,
}

impl<'a, T, B> MyRefMut<'a, T, B> {
    pub fn strip_lifetime_analysis(self) -> MyRefMut<'a, T, Nothing<'static>> {
        MyRefMut {
            token: self.token.strip_lifetime_analysis(),
            sptr: self.sptr,
        }
    }
}

let my_cell = MyRefCell::new(1u32);
let my_guard = if my_condition {
    Some(my_cell.borrow_mut().strip_lifetime_analysis())
} else {
    None
};
```

The `'static` lifetime in `Nothing` doesn't really mean anything. Indeed, the lifetime in `Nothing`
is purely a convenience lifetime whose utility will become more clear when we implement the second
required method: `borrow_mut_on_loan`.

Writing this method is also relatively straightforward:

```rust
use autoken::Nothing;

struct MyRefCell<T> {
    inner: RefCell<T>,
}

impl<T> MyRefCell<T> {
    pub fn borrow_mut_on_loan<'l>(
        &self,
        loaner: &'l mut MutableBorrow<T>
    ) -> MyRefMut<'_, T, Nothing<'l>> {
        MyRefMut {
            token: loaner.loan(),
            sptr: self.inner.borrow_mut(),
        }
    }
}

let my_cell = MyRefCell::new(1u32);

let mut my_loaner = MutableBorrow::<u32>::new();
let my_guard = if my_condition {
    Some(my_cell.borrow_mut_on_loan(&mut my_loaner))
} else {
    None
};
```

Here, we're using the placeholder lifetime in `Nothing` to limit the lifetime of the loans to
the reference to the `loaner`. Pretty convenient.

Finally, fallible `borrow` method variants can be implemented in a way almost identical to the
previous example's:

```rust
use autoken::{Nothing, PotentialMutableBorrow};

struct MyRefCell<T> {
    inner: RefCell<T>,
}

impl<T> MyRefCell<T> {
    pub fn try_borrow_mut<'l>(
        &self,
        loaner: &'l mut PotentialMutableBorrow<T>
    ) -> Result<MyRefMut<'_, T, Nothing<'l>>, BorrowMutError> {
        self.inner.try_borrow_mut().map(|sptr| MyRefMut {
            token: loaner.loan(),
            sptr,
        })
    }
}

let my_cell = MyRefCell::new(1u32);

let mut my_loaner_1 = PotentialMutableBorrow::<u32>::new();
let borrow_1 = my_cell.try_borrow_mut(&mut my_loaner_1).unwrap();

let mut my_loaner_2 = PotentialMutableBorrow::<u32>::new();
let not_borrow_2 = my_cell.try_borrow_mut(&mut my_loaner_2).unwrap_err();
```

How exciting!

<!-- cargo-rdme end -->
