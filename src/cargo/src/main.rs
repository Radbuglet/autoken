use std::process::Command;

use anyhow::Context;
use directories::ProjectDirs;
use rustc_build_sysroot::{SysrootBuilder, SysrootConfig};

fn main() -> anyhow::Result<()> {
    // We try to stuff two binaries in one: `cargo-autoken` and `autoken-rustc`. This code is what
    // allows us to do that.
    //
    // If the `AUTOKEN_CARGO_ACT_AS_RUSTC` environment variable is set, act as `autoken-rustc`.
    // Otherwise, we're running as `cargo autoken`!
    if std::env::var("AUTOKEN_CARGO_ACT_AS_RUSTC_WRAPPER").is_ok_and(|v| v == "yes") {
        autoken_rustc::analyzer::main_inner(std::env::args().collect()); // This is divergent.
    }

    // Parse the arguments the user wants to send to cargo.
    let in_args = {
        let mut in_args = std::env::args();

        // Skip the current binary argument
        in_args.next();

        // Skip the `autoken` argument in `cargo_autoken autoken <...>`.
        if in_args.next().as_deref() != Some("autoken") {
            anyhow::bail!("this binary should be called through cargo via `cargo autoken`");
        }
        in_args
    };

    // We're going to be calling cargo with a special version of `RUSTC`.
    let cargo_exe = std::env::var("CARGO").context("no cargo binary provided")?;
    let current_exe = std::env::current_exe()?;
    let make_cargo_cmd = || {
        let mut cmd = Command::new(&cargo_exe);
        cmd.env("RUSTC", &current_exe);
        cmd.env("AUTOKEN_CARGO_ACT_AS_RUSTC_WRAPPER", "yes");
        cmd
    };

    // We're about to rerun cargo with some special environment variables but we have to make sure
    // that the cargo binary we're about to execute is appropriate for our toolchain.
    // TODO

    // We need to determine the target with which the user wants us to build this project.
    let target_triple = "aarch64-apple-darwin"; // TODO

    // `autoken-rustc` has special requirements on how the standard library is built so let's create
    // a custom sysroot for that if it doesn't already exist.
    let sysroot_store = ProjectDirs::from("me", "radbuglet", "autoken")
        .context("failed to get sysroot cache directory")?;
    let sysroot_store = sysroot_store.cache_dir();
    {
        // Determine the path to the source code for the sysroot compilation
        let sysroot_src_code = rustc_build_sysroot::rustc_sysroot_src({
            let mut cmd = Command::new(&current_exe);
            cmd.env("AUTOKEN_CARGO_ACT_AS_RUSTC_WRAPPER", "yes");
            cmd
        })?;

        if !sysroot_src_code.exists() {
            anyhow::bail!("could not find rust-src for this current toolchain");
        }

        // Create it!
        SysrootBuilder::new(sysroot_store, target_triple)
            .cargo({
                let mut cmd = make_cargo_cmd();
                cmd.env("AUTOKEN_SKIP_ANALYSIS", "yes");
                cmd
            })
            .sysroot_config(SysrootConfig::WithStd {
                std_features: vec!["panic_unwind".to_string(), "backtrace".to_string()],
            })
            .build_from_source(&sysroot_src_code)?;
    }

    // With all the validation out of the way, we can now run cargo. Our only requirement is that
    // `RUSTC` and the sysroot are appropriately set.
    let mut cmd = make_cargo_cmd();
    cmd.env("RUSTC_AUTOKEN_OVERRIDE_SYSROOT", sysroot_store);
    cmd.args(in_args);
    cmd.spawn()
        .context("failed to spawn final cargo command")?
        .wait()?;

    Ok(())
}
