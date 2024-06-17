# AuToken

> [!NOTE]
> This version is much newer than and very different to the version published on [crates.io](https://crates.io/crates/autoken).

<!--
N.B. This README is generated from the crate documentation of `src/userland` and is mirrored in
`README.md` and `src/userland/README.md`.
-->

<!-- cargo-rdme start -->

A rust-lang compiler tool adding support for zero-cost borrow-aware context passing.

```rust
use autoken::{cap, CapTarget};

cap! {
    pub MyCap = Vec<u32>;
}

fn main() {
    let mut my_vec = vec![1, 2, 3, 4];

    MyCap::provide(&mut my_vec, || {
        do_something();
    });
}

fn do_something() {
    with_indirection();
}

fn with_indirection() {
    let my_vec = cap!(ref MyCap);
    let first_three = &my_vec[0..3];
    cap!(mut MyCap).push(5);
    eprintln!("The first three elements were {first_three:?}");
}
```

```plain_text
error: conflicting borrows on token MyCap
  --> src/main.rs:23:5
   |
20 |     let my_vec = cap!(ref MyCap);
   |                  --------------- value first borrowed immutably
...
23 |     cap!(mut MyCap).push(5);
   |     ^^^^^^^^^^^^^^^ value later borrowed mutably
   |
   = help: first borrow originates from Borrows::<Mut<MyCap>>::acquire_ref::<'_>
   = help: later borrow originates from Borrows::<Mut<MyCap>>::acquire_mut::<'_>
```

### Installation

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

#### Project Setup

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

### Basic Usage

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
    autoken::cap!(mut MyCap).push(value);
    eprintln!("The list is now {:?}, skip a few, {:?}", first_n_numbers(2), last_number());
}

fn last_number() -> Option<u32> {
    autoken::cap!(ref MyCap).last().copied()
}

fn first_n_numbers<'a>(count: usize) -> &'a [u32] {
    // Declares the fact that `'a` depends on a borrow of `MyCap`.
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

**To-Do:** Document unsizing rules.

### Advanced Usage

**To-Do:** Document the lower-level API.

### Limitations

**To-Do:** Document analysis limitations.

<!-- cargo-rdme end -->
