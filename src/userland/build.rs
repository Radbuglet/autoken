use std::{fs, path::PathBuf};

fn main() {
    // Don't rebuild this crate when nothing changed.
    println!("cargo:rerun-if-changed=build.rs");

    // Ensure that we have an appropriate version string.
    autoken_versions::validate_crate_version_in_build_script();

    // Just export the version.
    let mut p_version_check = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    p_version_check.push("version_check.rs");

    fs::write(p_version_check, autoken_versions::emit_userland_cfg_file()).unwrap();
}
