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
    ],
    "rust-analyzer.cargo.extraArgs": ["-Z", "bindeps"],
}
```

...to ensure that all projects in this repository can be properly discovered and given IntelliSense.

## Organization

There are three major components to AuToken:

- The `userland` crate (called `autoken`) people include in their projects to interface with AuToken.
- The `rustc` wrapper (called `autoken-rustc`), which is a modified version of `rustc` which adds AuToken's analysis phase and configures the `rustc` compiler to always emit the full MIR for every crate it compiles.
- The `cargo` wrapper (called `cargo-autoken`), which...
  - validates the `cargo` toolchain version to avoid ABI issues
  - compiles a version of the Rust standard library using our custom version of `rustc` for use as a sysroot
  - ...and re-executes the appropriate `cargo` command with `rustc` overwritten to point to our custom wrapper and with the appropriate sysroot specified.

## Executing Cargo

The `cargo-autoken` binary automatically builds its own version of `autoken-rustc` so you can execute the entire stack by calling `cargo run` in the `src/cargo` directory. This tool can be installed onto your local computer with `cargo install --path src/cargo -Z bindeps`—no questions asked! The `rust-toolchain.toml` should handle all the nightly shenanigans for you.

## Executing Rustc

The easiest way to execute `autoken-rustc` directly is through `cargo-autoken`'s `rustc` subcommand.

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

## Version Update Checklist

Interface updates:

- [ ] Bump `autoken-rustc`'s version if need be.
- [ ] Bump the peer version requirement in `cargo-autoken`. 
- [ ] Bump `autoken`'s version if need be.
- [ ] Bump `INTERFACE_VERSION.in` in both `src/userland` and `src/cargo` if need be.

Documentation updates:

- [ ] If the toolchain changed, update the toolchain in the `autoken` crate documentation.
- [ ] If the version of `cargo-autoken` changed, update the toolchain in the `autoken` crate
      documentation.
- [ ] Regenerate the repository README from the `autoken`'s crate documentation if need be using
      [`cargo rdme`](https://crates.io/crates/cargo-rdme).
- [ ] Copy that regenerated README from the repository root to `src/userland/README.md` if need be.

Updating repository information:

- [ ] If this repository URL is changed, it needs to be changed in:
  - [ ] All the `Cargo.toml` metadata files.
  - [ ] The rustc driver's ICE issue config.
  - [ ] Cargo's version checking constants.
- [ ] If the project tag-line changes, it needs to be updated in all the `Cargo.toml` metadata files.

Publishing:

- [ ] Publish everything that changed in the following order:
  - [ ] `autoken-rustc`
  - [ ] `cargo-autoken`
  - [ ] `autoken`
