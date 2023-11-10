use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
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
    #[command(about = "Run autoken's version of rustc.")]
    Rustc {
        #[command(flatten)]
        binary_overrides: CliBinaryOverrides,

        #[command(flatten)]
        rustc_overrides: CliRustcOverrides,

        #[command(subcommand)]
        args: CliRustcArgs,
    },
    #[command(about = "Print metadata about this cargo-autoken installation.")]
    Metadata,
    #[command(about = "Clean cargo-autoken's global cache directory.")]
    ClearCache,
    #[command(about = "Emit the embedded rustc wrapper binary into the target path.")]
    EmitRustc {
        #[arg(help = "The path of the binary to be written.")]
        path: PathBuf,
    },
    #[command(
        about = "Build a suitable sysroot for the rustc wrapper binary into the target path."
    )]
    BuildSysroot {
        #[command(flatten)]
        binary_overrides: CliBinaryOverrides,

        #[arg(
            short = 't',
            long = "target",
            help = "Specify the target triple for which the sysroot will be generated.",
            default_value = None,
        )]
        target: Option<String>,

        #[arg(help = "The path of the sysroot to be built.")]
        path: PathBuf,
    },
}

#[derive(Debug, Args)]
struct CliCmdCheck {
    // Binary overrides
    #[command(flatten)]
    binary_overrides: CliBinaryOverrides,

    #[command(flatten)]
    rustc_overrides: CliRustcOverrides,

    #[arg(
        short = 'O',
        long = "target-dir",
        help = "Specify a custom cargo target directory into which the project will be compiled and analyzed.",
        default_value = None,
    )]
    target_dir: Option<PathBuf>,

    #[arg(
        short = 'W',
        long = "old-artifacts",
        help = "Specifies how we should handle cargo target directories generated by other cargo-autoken \
                versions.",
        default_value = "warn"
    )]
    old_artifact_mode: CliOldArtifactMode,

    // Cargo options
    #[command(flatten)]
    manifest: clap_cargo::Manifest,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
enum CliOldArtifactMode {
    Warn,
    Delete,
    Ignore,
}

#[derive(Debug, Args)]
struct CliBinaryOverrides {
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
}

#[derive(Debug, Args)]
struct CliRustcOverrides {
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
    target_triple: Option<String>,
}

#[derive(Debug, Subcommand)]
#[command(disable_help_flag = true)]
enum CliRustcArgs {
    #[command(
        name = "metadata",
        about = "Print metadata about the rustc instance to be run"
    )]
    Metadata,
    #[command(name = "with", about = "Run rustc with the specified arguments")]
    With {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rustc_args: Vec<String>,
    },
}

// === Driver === //

fn main() -> anyhow::Result<()> {
    // Parse CLI
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

    // Get a cache directory for our work.
    let mut app_dir = LazilyComputed::new(|| {
        ProjectDirs::from("me", "radbuglet", "autoken")
            .context("failed to get a cache directory for autoken")
    });

    // Handle CLI
    match cli.cmd {
        CliCmd::Check(args) => {
            // Get the binary collection.
            let bin = BinaryCollection::new(&mut app_dir, &args.binary_overrides)?;

            let (target_triple, rustc_sysroot_path) =
                prepare_rust_wrapper(&mut app_dir, &bin, &args.rustc_overrides)?;

            // Determine the target artifact directory for our compilation.
            let target_dir = match args.target_dir {
                Some(path) => path,
                None => {
                    let meta = args.manifest.metadata().exec().context(
                        "Failed to get cargo metadata. This was performed in order to customize \
                         the cargo target directory and can be skipped by setting it manually \
                         by setting the `target-dir` parameter.",
                    )?;
                    let mut target_dir = PathBuf::from(meta.target_directory);
                    target_dir.push("autoken");

                    // Try to remove the all autoken directories which don't belong to us.
                    if args.old_artifact_mode != CliOldArtifactMode::Ignore {
                        if let Ok(item_list) = fs::read_dir(&target_dir) {
                            for item in item_list.flatten() {
                                if item.file_name() != rustc_wrapper_hash() {
                                    let path = item.path();

                                    if args.old_artifact_mode == CliOldArtifactMode::Warn {
                                        eprintln!(
                                            "The target artifact directory {} was created by a \
                                            different version of cargo-autoken and is likely wasting \
                                            space. If you wish to have these directories automatically \
                                            removed, set the `old-artifacts` parameter to `delete`. \
                                            If you wish to suppress this warning, set the parameter \
                                            to `ignore`.",
                                            path.to_string_lossy(),
                                        );
                                    } else {
                                        let _ = fs::remove_dir_all(path);
                                    }
                                }
                            }
                        }
                    }

                    target_dir.push(rustc_wrapper_hash());
                    target_dir
                }
            };

            // Call out to cargo to do the actual work!
            let mut cmd = bin.cargo_cmd(bin.rustc_cmd(false, Some(rustc_sysroot_path)));
            cmd.arg("check")
                .arg("--target")
                .arg(target_triple)
                .arg("--target-dir")
                .arg(target_dir);

            if let Some(path) = args.manifest.manifest_path {
                cmd.arg("--path").arg(path);
            }

            std::process::exit(
                cmd.spawn()
                    .context("failed to spawn cargo")?
                    .wait_with_output()?
                    .status
                    .code()
                    .unwrap_or(1),
            );
        }
        CliCmd::Rustc {
            binary_overrides,
            rustc_overrides,
            args,
        } => {
            // Get the binary collection.
            let bin = BinaryCollection::new(&mut app_dir, &binary_overrides)?;

            let (target_triple, rustc_sysroot_path) =
                prepare_rust_wrapper(&mut app_dir, &bin, &rustc_overrides)?;

            // Call out to autoken-rustc to do the actual work!
            match args {
                CliRustcArgs::Metadata => {
                    println!(
                        "autoken-rustc-exe: {}",
                        bin.rustc_wrapper_path.to_string_lossy()
                    );
                    println!("autoken-rustc-sysroot-target: {}", target_triple);
                    println!(
                        "autoken-rustc-sysroot-path: {}",
                        rustc_sysroot_path.to_string_lossy()
                    );
                    Ok(())
                }
                CliRustcArgs::With { rustc_args } => std::process::exit(
                    bin.rustc_cmd(false, Some(rustc_sysroot_path))
                        .arg("--target")
                        .arg(target_triple)
                        .args(rustc_args)
                        .spawn()
                        .with_context(|| {
                            format!(
                                "Failed to spawn autoken-rustc (path: {})",
                                bin.rustc_wrapper_path.to_string_lossy()
                            )
                        })?
                        .wait_with_output()?
                        .status
                        .code()
                        .unwrap_or(1),
                ),
            }
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
        CliCmd::BuildSysroot {
            binary_overrides,
            target,
            path,
        } => {
            // Get the binary collection.
            let bin = BinaryCollection::new(&mut app_dir, &binary_overrides)?;

            // Get the target.
            let target = match target {
                Some(target) => target,
                None => get_host_target(bin.rustc_cmd(true, None))
                    .context("failed to determine host target")?,
            };

            // Build the requested sysroot.
            eprintln!(
                "Building sysroot for target {target} in path {}...",
                path.to_string_lossy()
            );

            build_sysroot(
                &path,
                &target,
                bin.rustc_cmd(true, None),
                bin.cargo_cmd(bin.rustc_cmd(true, None)),
            )?;

            Ok(())
        }
    }
}

#[derive(Debug)]
struct BinaryCollection {
    cargo_exe: PathBuf,
    rustc_wrapper_path: PathBuf,
}

impl BinaryCollection {
    pub fn new(
        app_dir: &mut LazilyComputed<'_, ProjectDirs>,
        args: &CliBinaryOverrides,
    ) -> anyhow::Result<Self> {
        // Get a path to cargo.
        let cargo_exe = match &args.custom_cargo {
            Some(path) => path.clone(),
            None => get_calling_cargo().context(
                "Failed to get the cargo binary through which this cargo tool was invoked. If \
                 this tool was not invoked through cargo, consider setting the `custom-cargo` parameter.",
            )?,
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

            let cargo_rustc_version =
                get_rustc_version_str(&cargo_rustc_exe).with_context(|| {
                    format!(
                        "Failed to determine version of the rustc binary with which the invoking \
                     cargo command was distributed (expected path: {}). This is for an integrity \
                     check so, if this cannot be satisfied, you can bypass this check entirely \
                     by setting the `disable-toolchain-checks` flag.",
                        cargo_rustc_exe.to_string_lossy(),
                    )
                })?;

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

        // Get our rustc wrapper.
        let rustc_wrapper_path = match &args.custom_rustc_wrapper {
            Some(path) => path.clone(),
            None => {
                // Determine its path
                let mut path = app_dir
                    .get()
                    .context(
                        "Failed to get a work directory into which we can extract our custom \
                         rustc wrapper. You can specify a path to a custom autoken rustc wrapper \
                         by setting the `custom-rustc-wrapper` flag.",
                    )?
                    .cache_dir()
                    .to_path_buf();

                let file_name = format!("autoken_rustc_wrapper_{}", rustc_wrapper_hash());

                if cfg!(windows) {
                    path.push(&format!("{file_name}.exe"));
                } else {
                    path.push(&file_name);
                }

                // Extract it
                write_rustc_wrapper_exe(&path).with_context(|| {
                    format!(
                        "Failed to extract our autoken rustc wrapper into {}. You can specify a \
                         path to a custom autoken rustc wrapper by setting the `custom-rustc-wrapper`
                         parameter.",
                        path.to_string_lossy(),
                    )
                })?;

                path
            }
        };

        Ok(Self {
            cargo_exe,
            rustc_wrapper_path,
        })
    }

    pub fn cargo_cmd(&self, rustc: Command) -> Command {
        get_cargo_wrapper_cmd_gen(&self.cargo_exe)(rustc)
    }

    pub fn rustc_cmd(&self, skip_analysis: bool, sysroot: Option<&Path>) -> Command {
        get_rustc_wrapper_cmd_gen(&self.rustc_wrapper_path)(skip_analysis, sysroot)
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

fn get_cargo_wrapper_cmd_gen(cargo_exe: &Path) -> impl Fn(Command) -> Command + '_ {
    move |rust_cmd: Command| {
        let mut cmd = Command::new(cargo_exe);
        cmd.env("RUSTC", rust_cmd.get_program());
        cmd.envs(rust_cmd.get_envs().filter_map(|(a, b)| Some((a, b?))));
        cmd
    }
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

fn prepare_rust_wrapper<'a>(
    app_dir: &'a mut LazilyComputed<'_, ProjectDirs>,
    bin: &BinaryCollection,
    args: &'a CliRustcOverrides,
) -> anyhow::Result<(String, &'a Path)> {
    // Get the target.
    let target_triple = match &args.target_triple {
        Some(target) => target.clone(),
        None => get_host_target(bin.rustc_cmd(true, None)).context(
            "Failed to determine host target triple while preparing sysroot. This can be skipped by \
             specifying a target explicitly with the `target` parameter.",
        )?,
    };

    // Get a sysroot for our wrapper.
    let rustc_sysroot_path = match &args.custom_rustc_sysroot {
        Some(path) => path,
        None => {
            let sysroot_dir = app_dir.get()?.cache_dir();

            build_sysroot(
                sysroot_dir,
                &target_triple,
                bin.rustc_cmd(true, None),
                bin.cargo_cmd(bin.rustc_cmd(true, None)),
            ).context(
                "Failed to build sysroot. This can be skipped by specifying a sysroot explicitly with \
                 the `custom-rustc-sysroot` parameter."
            )?;

            sysroot_dir
        }
    };

    Ok((target_triple, rustc_sysroot_path))
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

enum LazilyComputed<'f, V> {
    Ok(V),
    Pending(Option<Box<dyn FnOnce() -> anyhow::Result<V> + 'f>>),
}

impl<'f, V> LazilyComputed<'f, V> {
    pub fn new<F>(f: F) -> Self
    where
        F: 'f + FnOnce() -> anyhow::Result<V>,
    {
        Self::Pending(Some(Box::new(f)))
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
