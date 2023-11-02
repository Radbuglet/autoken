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
