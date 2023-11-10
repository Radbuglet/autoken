use std::{path::PathBuf, process::Command};

fn main() -> anyhow::Result<()> {
    // Don't rebuild this crate when nothing changed.
    println!("cargo:rerun-if-changed=build.rs");

    // Make `AUTOKEN_EXPECTED_RUSTC_VERSION` available to the binary.
    let rustc_exe = PathBuf::from(std::env::var("RUSTC")?);
    let rustc_version =
        String::from_utf8(Command::new(rustc_exe).arg("--version").output()?.stdout)?;

    println!("cargo:rustc-env=AUTOKEN_EXPECTED_RUSTC_VERSION={rustc_version}");

    // Make `AUTOKEN_RUSTC_WRAPPER_BINARY` available to the binary.
    let binary_path = std::env::var("CARGO_BIN_FILE_AUTOKEN_RUSTC")?;
    println!("cargo:rustc-env=AUTOKEN_RUSTC_WRAPPER_BINARY={binary_path}");
    println!(
        "cargo:rustc-env=AUTOKEN_RUSTC_WRAPPER_BINARY_HASH={}",
        sha256::digest(std::fs::read(binary_path)?),
    );

    Ok(())
}
