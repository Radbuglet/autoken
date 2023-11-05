use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;
use directories::ProjectDirs;
use rustc_build_sysroot::{SysrootBuilder, SysrootConfig};

fn main() -> anyhow::Result<()> {
    // First, we must ensure that we are being run in the context of cargo.
    let (cargo_exe, cargo_args) = extract_cargo_tool_params()
        .context("this binary must be executed as a cargo tool via `cargo autoken`")?;

    // Next, ensure that cargo's `rustc` version string against which `cargo`'s linker path is
    // provided is appropriate for our rustc binary.
    let mut cargo_rustc_exe = cargo_exe.clone();
    if cfg!(windows) {
        cargo_rustc_exe.set_file_name("rustc.exe");
    } else {
        cargo_rustc_exe.set_file_name("rustc");
    }
    let cargo_rustc_version = rustc_version_str(&cargo_rustc_exe).context(
        "failed to determine version of the rustc binary with which the invoking cargo command was \
         distributed",
    )?;

    #[allow(clippy::comparison_to_empty)] // false positive
    if cargo_rustc_version.lines().next() != env!("AUTOKEN_EXPECTED_RUSTC_VERSION").lines().next() {
        anyhow::bail!(
            "The version of rustc with which cargo was bundled was {:?} but autoken's \
             rustc version was {:?}. Make sure to call cargo with the appropriate toolchain parameter.",
            cargo_rustc_version,
            env!("AUTOKEN_EXPECTED_RUSTC_VERSION"),
        );
    }

    // Now, create a directory to store all our work.
    let app_dir = ProjectDirs::from("me", "radbuglet", "autoken")
        .context("failed to get app-dir for autoken")?;
    let app_dir = app_dir.cache_dir();

    // Now, we need to unpack our custom version of `rustc` into a file where we can execute it.
    let rustc_wrapper_path = {
        let mut path = app_dir.to_path_buf();

        if cfg!(windows) {
            path.push("autoken_rustc_wrapper.exe");
        } else {
            path.push("autoken_rustc_wrapper");
        }
        path
    };
    write_rustc_wrapper_exe(&rustc_wrapper_path).context("failed to create rustc wrapper file")?;

    let rustc_cmd = |skip_analysis: bool, sysroot: Option<&Path>| {
        let mut cmd = Command::new(&rustc_wrapper_path);

        if skip_analysis {
            cmd.env("AUTOKEN_SKIP_ANALYSIS", "yes");
        } else {
            cmd.env_remove("AUTOKEN_SKIP_ANALYSIS");
        }

        if let Some(sysroot) = sysroot {
            cmd.env("AUTOKEN_OVERRIDE_SYSROOT", sysroot);
        } else {
            cmd.env_remove("AUTOKEN_OVERRIDE_SYSROOT");
        }

        cmd
    };

    let cargo_cmd = |rust_cmd: Command| {
        let mut cmd = Command::new(&cargo_exe);
        cmd.env("RUSTC", rust_cmd.get_program());
        cmd.envs(rust_cmd.get_envs().filter_map(|(a, b)| Some((a, b?))));
        cmd
    };

    // Let's also ensure that we have a valid sysroot for this version of `rustc`.
    build_sysroot(
        app_dir,
        // TODO: Determine target dynamically
        "aarch64-apple-darwin",
        rustc_cmd(true, None),
        cargo_cmd(rustc_cmd(true, None)),
    )
    .context("failed to build sysroot")?;

    // Now that all of that is out of the way, we can finally re-dispatch cargo with our custom rustc
    // version.
    std::process::exit(
        cargo_cmd(rustc_cmd(false, Some(app_dir)))
            .args(cargo_args)
            .spawn()
            .context("failed to spawn cargo")?
            .wait_with_output()?
            .status
            .code()
            .unwrap_or(1),
    );
}

fn extract_cargo_tool_params() -> anyhow::Result<(PathBuf, Vec<String>)> {
    let cargo_exe = std::env::var("CARGO").context("`CARGO` environment variable was not set")?;

    let mut cargo_args = std::env::args();

    // Skip the current binary argument
    cargo_args.next();

    // Skip the `autoken` argument in `cargo_autoken autoken <...>`.
    if cargo_args.next().as_deref() != Some("autoken") {
        anyhow::bail!("this binary should be called through cargo via `cargo autoken`");
    }

    Ok((PathBuf::from(cargo_exe), cargo_args.collect()))
}

fn rustc_version_str(rustc: &Path) -> anyhow::Result<String> {
    Ok(String::from_utf8(
        Command::new(rustc).arg("--version").output()?.stdout,
    )?)
}

fn write_rustc_wrapper_exe(path: &Path) -> anyhow::Result<()> {
    // Ensure that the parent directory exists
    fs::create_dir_all(path.parent().unwrap())?;

    // Set the file options
    let mut opts = File::options();
    opts.write(true).create(true).truncate(true);

    // Write the data to the file
    let mut rust_wrapper_file = opts.open(path)?;
    rust_wrapper_file.write_all(rustc_wrapper_data())?;

    #[cfg(unix)]
    rust_wrapper_file.set_permissions({
        use std::os::unix::fs::PermissionsExt;
        let mut perms = rust_wrapper_file.metadata()?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        perms
    })?;

    Ok(())
}

fn rustc_wrapper_data() -> &'static [u8] {
    include_bytes!(env!("AUTOKEN_RUSTC_WRAPPER_BINARY"))
}

fn build_sysroot(
    store_path: &Path,
    target: &str,
    rust_cmd: Command,
    cargo_cmd: Command,
) -> anyhow::Result<()> {
    let sysroot_src_code = rustc_build_sysroot::rustc_sysroot_src(rust_cmd)?;

    if !sysroot_src_code.exists() {
        anyhow::bail!("could not find rust-src for this current toolchain");
    }

    SysrootBuilder::new(store_path, target)
        .cargo(cargo_cmd)
        .sysroot_config(SysrootConfig::WithStd {
            std_features: vec!["panic_unwind".to_string(), "backtrace".to_string()],
        })
        .build_from_source(&sysroot_src_code)?;

    Ok(())
}
