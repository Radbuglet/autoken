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

### Checking Projects

AuToken is a framework for adding static analysis of runtime borrowing to your crate. If you are
an end-user of a crate with integrations with AuToken and wish to check your project with the
tool, this is the section for you! If, instead, you're building a crate and wish to integrate with
AuToken, you should read on to the [Integrating AuToken](#integrating-autoken) section.

If you wish to install from source, assuming your current working directory is the same as the
[repository](https://github.com/radbuglet/autoken)'s README, `cargo-autoken` can be installed
like so:

```bash
cargo install --path src/cargo
```

...and executed in the crate you wish to validate like so:

```bash
cargo autoken check
```

Have fun!

### Ignoring False Positives

AuToken is, by nature, very conservative. After all, its whole job is to ensure that only one
borrow of a given type exists at a given time, even if you're borrowing from several different
sources at once!

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
entirely and all the `strip_lifetime_analysis`
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

### Dealing With Dynamic Dispatches

AuToken resolves dynamic dispatches by collecting all possible dispatch targets ahead of time
based around what gets unsized to what and assumes that any of those could be called. This
can occasionally be overly pessimistic. You can help this along by making the dynamically
dispatched traits more fine grained. For example, instead of using an `FnMut(u32, i32, f32)`, you
could use an `FnMut(PhantomData<MyHandlers>, u32, i32, f32)`. Likewise, if you have a trait
`MyBehavior`, you could parameterize it by a marker generic type to make it even more fine-grained.

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

**TODO:** Write documentation

<!-- cargo-rdme end -->
