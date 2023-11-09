use std::{fmt::Write, fs, path::PathBuf};

fn main() {
    // Don't rebuild this crate when nothing changed.
    println!("cargo:rerun-if-changed=build.rs");

    // Just export the version.
    let mut p_version_check = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    p_version_check.push("version_check.rs");

    let mut file = String::new();
    writeln!(
        file,
        "#[cfg(all(__autoken_checking_version, not(__autoken_current_version_is_{}_{}_{})))]",
        std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap(),
        std::env::var("CARGO_PKG_VERSION_MINOR").unwrap(),
        std::env::var("CARGO_PKG_VERSION_PATCH").unwrap(),
    )
    .unwrap();

    writeln!(
        file,
        "compile_error!(\"Expected autoken to be validated by cargo-autoken version {}.{}.{} but got \
		 a different version. Run `cargo autoken --version` to get the current version.\");",
        std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap(),
		std::env::var("CARGO_PKG_VERSION_MINOR").unwrap(),
		std::env::var("CARGO_PKG_VERSION_PATCH").unwrap(),
    )
    .unwrap();

    fs::write(p_version_check, file).unwrap();
}
