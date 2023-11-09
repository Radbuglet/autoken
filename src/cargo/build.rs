use std::{path::PathBuf, process::Command};

fn main() -> anyhow::Result<()> {
    // Don't rebuild this crate when nothing changed.
    println!("cargo:rerun-if-changed=build.rs");

    // Extract some environment variables.
    let rustc_exe = PathBuf::from(std::env::var("RUSTC")?);
    let cargo_exe = PathBuf::from(std::env::var("CARGO")?);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);

    // Make `AUTOKEN_EXPECTED_RUSTC_VERSION` available to the binary.
    let rustc_version =
        String::from_utf8(Command::new(rustc_exe).arg("--version").output()?.stdout)?;

    println!("cargo:rustc-env=AUTOKEN_EXPECTED_RUSTC_VERSION={rustc_version}");

    // Ensure that our rustc wrapper is also available for embedding.
    let rustc_wrapper_src = {
        // `./../rustc`
        let mut cwd = std::env::current_dir().unwrap().canonicalize()?;
        cwd.pop();
        cwd.push("rustc");
        cwd
    };

    println!(
        "cargo:rerun-if-changed={}",
        rustc_wrapper_src.to_string_lossy()
    );

    let build_result = Command::new(cargo_exe)
        .current_dir(rustc_wrapper_src)
        .arg("build")
        .arg("-Z")
        .arg("unstable-options")
        .arg("--release")
        .arg("--out-dir")
        .arg(&out_dir)
        .spawn()?
        .wait()?;

    anyhow::ensure!(build_result.success(), "failed to build the rustc wrapper");

    let binary_path = {
        let mut out = out_dir.clone();
        out.push("out");
        if cfg!(windows) {
            out.set_file_name("autoken-rustc.exe");
        } else {
            out.set_file_name("autoken-rustc");
        }
        out
    };

    println!(
        "cargo:rustc-env=AUTOKEN_RUSTC_WRAPPER_BINARY={}",
        binary_path.to_string_lossy()
    );

    println!(
        "cargo:rustc-env=AUTOKEN_RUSTC_WRAPPER_BINARY_HASH={}",
        sha256::digest(std::fs::read(binary_path)?),
    );

    Ok(())
}
