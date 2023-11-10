# Contribution Guide

We have yet to devise a contribution process for this repository. Just file a PR for now and we'll work from there.

## Rust-Analyzer Setup

In VSCode's distribution of rust-analyzer, you can populate the file `.vscode/settings.json` at the root directory of this project with:

```json
{
    "rust-analyzer.linkedProjects": [
        "src/cargo/Cargo.toml",
        "src/rustc/Cargo.toml",
        "src/userland/Cargo.toml",
    ]
}
```

...to ensure that all projects in this repository can be properly discovered and given IntelliSense.

## Organization

There are three major components to AuToken:

- The `userland` crate people include in their projects to interface with AuToken.
- The `rustc` wrapper, which is a modified version of `rustc` which adds AuToken's analysis phase and configures the `rustc` compiler to always emit the full MIR for every crate it compiles.
- The `cargo` wrapper, which...
  - validates the `cargo` toolchain version to avoid ABI issues
  - compiles a version of the Rust standard library using our custom version of `rustc` for use as a sysroot
  - ...and re-executes the appropriate `cargo` command with `rustc` overwritten to point to our custom wrapper and with the appropriate sysroot specified.

## Executing Cargo

The `cargo-autoken` binary automatically builds its own version of `autoken-rustc` so you can execute the entire stack by calling `cargo run` in the `src/cargo` directory. This tool can be installed onto your local computer with `cargo install --path src/cargo`—no questions asked.

## Executing Rustc

The easiest way to execute `rustc-autoken` directly is through `cargo-autoken`'s `rustc` subcommand.

Alternatively, you could build the custom rustc driver manually. If you want to call it directly without the help of `cargo`, you have to build it with an appropriate rpath:

```bash
RUSTFLAGS="-C link-args=-Wl,-rpath,$(rustc --print sysroot)/lib" cargo build --release
```

Otherwise, you can build it with:

```bash
cargo build --release
```

...but then the wrapper *must* be called by a version of `cargo` which corresponds to the `rustc` toolchain with which the binary was built since, otherwise, you could get a dynamic library mismatch.

Then, just run `cargo` on the desired project with the appropriate custom rustc driver and separate target directory:

```
RUSTC="path/to/autoken_rustc_wrapper" CARGO_TARGET_DIR="target/autoken" cargo +toolchain run -Zbuild-std=core,alloc,std --target $(path/to/autoken_rustc_wrapper -vV | sed -n 's|host: ||p')
```
