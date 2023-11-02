# Contribution Guide

TODO: Write this guide

## Rust-Analyzer Setup

In VSCode's distribution of rust-analyzer, you can populate the file `.vscode/settings.json` at the root directory of this project with:

```json
{
    "rust-analyzer.linkedProjects": [
        "src/analyzer/Cargo.toml",
        "src/userland/Cargo.toml",
    ]
}
```

...to ensure that all projects in this repository can be properly discovered and given IntelliSense.

## Manual Execution

Build the custom rustc driver with an appropriate rpath:

```
RUSTFLAGS="-C link-args=-Wl,-rpath,$(rustc --print sysroot)/lib" cargo build --release
```

If that doesn't work, I don't know what to do.

Then, just run `cargo` on the desired project with the appropriate custom rustc wrapper and separate target directory:

```
CARGO_INCREMENTAL=0 RUSTC="path/to/rustc_wrapper/we_just_built" CARGO_TARGET_DIR="target/autoken" cargo build
```
