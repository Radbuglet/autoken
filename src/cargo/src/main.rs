use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use directories::ProjectDirs;
use rustc_build_sysroot::{SysrootBuilder, SysrootConfig};

// === Command-Line Parsing == //

#[derive(Debug, Parser)]
#[command(
    styles = clap_cargo::style::CLAP_STYLING,
    author = env!("CARGO_PKG_AUTHORS"),
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    long_about = None,

    // We strip out the binary name and `autoken` before handing off control to clap.
    no_binary_name = true,
)]
struct Cli {
    #[command(subcommand)]
    cmd: CliCmd,
}

#[derive(Debug, Subcommand)]
enum CliCmd {
    #[command(about = "Analyze the specified program.")]
    Check(CliCmdCheck),
    #[command(about = "Print metadata about this cargo-autoken installation.")]
    Metadata,
    #[command(about = "Clean cargo-autoken's global working directory.")]
    ClearCache,
    #[command(about = "Emit the embedded rustc wrapper binary into the target path.")]
    EmitRustc {
        #[arg(help = "The path of the binary to be written.")]
        path: PathBuf,
    },
    // TODO: Add the ability to build custom sysroots.
    // #[command(
    //     about = "Build a suitable sysroot for the rustc wrapper binary into the target path."
    // )]
    // BuildSysroot {
    //     #[arg(help = "The path of the sysroot to be built.")]
    //     path: PathBuf,
    // },
}

#[derive(Debug, Args)]
struct CliCmdCheck {
    // Binary overrides
    #[arg(
        short = 'I',
        long = "disable-toolchain-checks",
        help = "Disable calling cargo version integrity checks.",
        default_value_t = false
    )]
    disable_toolchain_checks: bool,

    #[arg(
        short = 'C',
        long = "custom-cargo",
        help = "Use a custom Cargo executable to check this project.",
        default_value = None,
    )]
    custom_cargo: Option<PathBuf>,

    #[arg(
        short = 'R',
        long = "custom-rustc-wrapper",
        help = "Use a custom rustc wrapper executable to check this project.",
        default_value = None,
    )]
    custom_rustc_wrapper: Option<PathBuf>,

    #[arg(
        short = 'S',
        long = "custom-rustc-sysroot",
        help = "Use a custom sysroot with our rustc wrapper executable to check this project.",
        default_value = None,
    )]
    custom_rustc_sysroot: Option<PathBuf>,

    // Check options
    #[arg(
        short = 't',
        long = "target",
        help = "Specify a custom target triple against which the project will be compiled and analyzed.",
        default_value = None,
    )]
    target: Option<String>,

    // Cargo options
    #[command(flatten)]
    manifest: clap_cargo::Manifest,
    // TODO: Forward more cargo arguments
    // #[command(flatten)]
    // workspace: clap_cargo::Workspace,
    // #[command(flatten)]
    // features: clap_cargo::Features,
}

// === Driver === //

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse_from({
        let mut args = std::env::args().peekable();

        // Skip executable
        args.next();

        // Skip autoken from `cargo autoken` if necessary
        if args.peek().map(String::as_str) == Some("autoken") {
            args.next();
        }

        args
    });

    match cli.cmd {
        CliCmd::Check(args) => {
            // Get a path to cargo.
            let cargo_exe = match args.custom_cargo {
                Some(path) => path,
                None => get_calling_cargo().context(
                    "Failed to get the cargo binary through which this cargo tool was invoked. If \
                     this tool was not invoked through cargo, consider setting the `custom-cargo` flag",
                )?,
            };

            let cargo_cmd = |rust_cmd: Command| {
                let mut cmd = Command::new(&cargo_exe);
                cmd.env("RUSTC", rust_cmd.get_program());
                cmd.envs(rust_cmd.get_envs().filter_map(|(a, b)| Some((a, b?))));
                cmd
            };

            // Ensure that cargo's `rustc` version string against which `cargo`'s linker path is
            // provided is appropriate for our rustc binary.
            if !args.disable_toolchain_checks {
                let mut cargo_rustc_exe = cargo_exe.clone();
                if cfg!(windows) {
                    cargo_rustc_exe.set_file_name("rustc.exe");
                } else {
                    cargo_rustc_exe.set_file_name("rustc");
                }

                let cargo_rustc_version = get_rustc_version_str(&cargo_rustc_exe).with_context(||
                    format!(
                        "Failed to determine version of the rustc binary with which the invoking \
                         cargo command was distributed (expected path: {}). This is for an integrity \
                         check so, if this cannot be satisfied, you can bypass this check entirely \
                         by setting the `disable-toolchain-checks` flag.",
                        cargo_rustc_exe.to_string_lossy(),
                    )
                )?;

                if cargo_rustc_version.lines().next() != Some(rustc_wrapper_version()) {
                    anyhow::bail!(
                        "The version of rustc (path: {}) with which cargo was bundled was {:?} but \
                         autoken's rustc version was {:?}. Make sure to call cargo with the appropriate \
                         toolchain parameter to avoid dynamic linker errors. If this is a false positive, \
                         you can bypass this check by setting the `disable-toolchain-checks` flag.",
                        cargo_rustc_exe.to_string_lossy(),
                        cargo_rustc_version,
                        env!("AUTOKEN_EXPECTED_RUSTC_VERSION"),
                    );
                }
            }

            // Get a cache directory for our work.
            let mut app_dir = LazilyComputed::new(|| {
                ProjectDirs::from("me", "radbuglet", "autoken")
                    .context("failed to get a cache directory for autoken")
            });

            // Get our rustc wrapper.
            let rustc_wrapper_path = match args.custom_rustc_wrapper {
                Some(path) => path,
                None => {
                    // Determine its path
                    let mut path = app_dir
                        .get()
                        .context(
                            "Failed to get a work directory into which we can extract our custom \
                             rustc wrapper. You can specify a path to a custom autoken rustc wrapper \
                             by setting the `custom-rustc-wrapper` flag."
                        )?
                        .cache_dir()
                        .to_path_buf();

                    if cfg!(windows) {
                        path.push("autoken_rustc_wrapper.exe");
                    } else {
                        path.push("autoken_rustc_wrapper");
                    }

                    // Extract it
                    write_rustc_wrapper_exe(&path).with_context(|| {
                        format!(
                            "Failed to extract our autoken rustc wrapper into {}. You can specify a \
                             path to a custom autoken rustc wrapper by setting the `custom-rustc-wrapper`
                             flag.",
                            path.to_string_lossy(),
                        )
                    })?;

                    path
                }
            };

            let rustc_cmd = get_rustc_wrapper_cmd_gen(&rustc_wrapper_path);

            // Get the target.
            let target = match args.target {
                Some(target) => target,
                None => get_host_target(rustc_cmd(true, None))
                    .context("failed to determine host target")?,
            };

            // Get a sysroot for our wrapper.
            let rustc_sysroot_path = match &args.custom_rustc_sysroot {
                Some(path) => path,
                None => {
                    let sysroot_dir = app_dir.get()?.cache_dir();

                    build_sysroot(
                        sysroot_dir,
                        &target,
                        rustc_cmd(true, None),
                        cargo_cmd(rustc_cmd(true, None)),
                    )?;

                    sysroot_dir
                }
            };

            // Call out to cargo to do the actual work!
            let mut cmd = cargo_cmd(rustc_cmd(false, Some(rustc_sysroot_path)));
            cmd.arg("check").arg("--target").arg(target);

            if let Some(path) = args.manifest.manifest_path {
                cmd.arg("--path").arg(path);
            }

            // TODO: Customize target directory
            // TODO: Allow users to pass their own custom arguments

            std::process::exit(
                cmd.spawn()
                    .context("failed to spawn cargo")?
                    .wait_with_output()?
                    .status
                    .code()
                    .unwrap_or(1),
            );
        }
        CliCmd::Metadata => {
            println!("cargo-autoken-version: {}", env!("CARGO_PKG_VERSION"));
            println!("rustc-wrapper-version: {}", rustc_wrapper_version());
            println!("rustc-wrapper-hash: {}", rustc_wrapper_hash());

            match get_cache_dir() {
                Ok(dir) => println!("rustc-cache-dir: {}", dir.to_string_lossy()),
                Err(err) => println!("rustc-cache-dir is unavailable: {err}"),
            }

            match get_calling_cargo() {
                Ok(dir) => println!("calling-cargo-path: {}", dir.to_string_lossy()),
                Err(err) => println!("calling-cargo-path is unavailable: {err}"),
            }

            Ok(())
        }
        CliCmd::ClearCache => {
            let cache_dir = get_cache_dir().context("failed to get cache directory")?;
            eprintln!("Deleting {}", cache_dir.to_string_lossy());
            std::fs::remove_dir_all(cache_dir).context("failed to delete cache directory")?;

            Ok(())
        }
        CliCmd::EmitRustc { path } => {
            eprintln!("Writing rustc wrapper to {}", path.to_string_lossy());
            write_rustc_wrapper_exe(&path).context("failed to write rustc wrapper")?;
            Ok(())
        }
    }
}

// === Helpers === //

fn get_cache_dir() -> anyhow::Result<PathBuf> {
    let app_dir = ProjectDirs::from("me", "radbuglet", "autoken")
        .context("failed to get app-dir for autoken")?;

    Ok(app_dir.cache_dir().to_path_buf())
}

fn get_calling_cargo() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(
        env::var("CARGO").context("`CARGO` environment variable was not set")?,
    ))
}

fn get_rustc_version_str(rustc: &Path) -> anyhow::Result<String> {
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

fn get_rustc_wrapper_cmd_gen(
    rustc_wrapper_path: &Path,
) -> impl Fn(bool, Option<&Path>) -> Command + '_ {
    move |skip_analysis: bool, sysroot: Option<&Path>| {
        let mut cmd = Command::new(rustc_wrapper_path);

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
    }
}

fn get_host_target(mut rust_cmd: Command) -> anyhow::Result<String> {
    Ok(String::from_utf8(rust_cmd.arg("-vV").output()?.stdout)?
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("failed to find `host: ` line")?
        .to_string())
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

enum LazilyComputed<V, F> {
    Ok(V),
    Pending(Option<F>),
}

impl<V, F> LazilyComputed<V, F>
where
    F: FnOnce() -> anyhow::Result<V>,
{
    pub fn new(f: F) -> Self {
        Self::Pending(Some(f))
    }

    pub fn get(&mut self) -> anyhow::Result<&mut V> {
        match self {
            LazilyComputed::Ok(ok) => Ok(ok),
            LazilyComputed::Pending(value) => {
                #[rustfmt::skip]
                let value = value
                    .take()
                    .expect("LazilyComputed::get() called after an error was already yielded")()?;

                *self = LazilyComputed::Ok(value);

                let LazilyComputed::Ok(value) = self else {
                    unreachable!()
                };

                Ok(value)
            }
        }
    }
}

// === Embedded Data === //

fn rustc_wrapper_version() -> &'static str {
    env!("AUTOKEN_EXPECTED_RUSTC_VERSION")
        .lines()
        .next()
        .unwrap()
}

fn rustc_wrapper_data() -> &'static [u8] {
    include_bytes!(env!("AUTOKEN_RUSTC_WRAPPER_BINARY"))
}

fn rustc_wrapper_hash() -> &'static str {
    env!("AUTOKEN_RUSTC_WRAPPER_BINARY_HASH")
}
