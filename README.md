# AuToken

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

## Getting Started

AuToken has two components: a library crate and a cargo tool. The library crate provides the low-level constructs one could use to add static analysis support to an existing runtime borrowing scheme such as a `RefCell`. If you are an end-consumer of a crate with autoken support, you likely do not have to worry about this component.

The cargo tool, meanwhile, provides the mechanism for actually checking the validity of a program involving autoken constructs. Assuming your current working directory is the same as this README's, it can be installed like so:

```bash
cargo install --path src/cargo
```

...and executed in the crate you wish to validate like so:

```bash
cargo autoken check
```
