use std::{fmt::Write, fs, path::PathBuf};

include!("CURRENT_FORMAT.in");

fn main() {
    // Don't rebuild this crate when nothing changed.
    println!("cargo:rerun-if-changed=build.rs");

    // Fetch the environment variables.
    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap();

    // Just export a version check file.
    let mut p_version_check = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    p_version_check.push("version_check.rs");

    let version_check_file = {
        // Never update these!
        const CFG_AUTOKEN_CHECKING_VERSIONS: &str = "__autoken_checking_versions";
        const CFG_MAJOR_IS_PREFIX: &str = "__autoken_major_is_";
        const CFG_SUPPORTED_MINOR_OR_LESS_IS_PREFIX: &str = "__autoken_supported_minor_or_less_is_";
        const CFG_DEPRECATED_MINOR_OR_LESS_IS_PREFIX: &str =
            "__autoken_deprecated_minor_or_less_is_";

        let mut builder = String::new();

        let userland_mismatch_warn_suffix = format!(
			"Userland crate *internal format* version is {CURRENT_FORMAT_MAJOR}.{CURRENT_FORMAT_MINOR}. \
			The userland crate at fault is running AuToken version {pkg_version}. \
			Run `cargo autoken --version` to see the version of your currently installed static analysis tool."
		);

        let correct_major_cfg = format!("{CFG_MAJOR_IS_PREFIX}{CURRENT_FORMAT_MAJOR}");

        // Major version validation
        writeln!(
            builder,
            "#[cfg(all({CFG_AUTOKEN_CHECKING_VERSIONS}, not({correct_major_cfg})))]"
        )
        .unwrap();

        writeln!(
			builder,
			"compile_error!(\"Major version mismatch in AuToken format: this version of the AuToken \
			userland crate is entirely incompatible with the version of the analyzer. \
			{userland_mismatch_warn_suffix}\");\n"
		)
        .unwrap();

        // Minor version validation
        writeln!(
			builder,
			"#[cfg(all({CFG_AUTOKEN_CHECKING_VERSIONS}, {correct_major_cfg}, not({CFG_SUPPORTED_MINOR_OR_LESS_IS_PREFIX}{CURRENT_FORMAT_MINOR})))]"
		)
		.unwrap();
        writeln!(builder, "const _: () = autoken_minor_version_mismatch();\n").unwrap();

        writeln!(builder, "#[allow(dead_code)]").unwrap();
        writeln!(
			builder,
			"#[deprecated(note = \"Minor version mismatch in AuToken format: this version of the AuToken \
			userland crate is using features only available in a later version of the analyzer. \
			{userland_mismatch_warn_suffix}\")]",
		)
        .unwrap();
        writeln!(builder, "const fn autoken_minor_version_mismatch() {{}}\n").unwrap();

        // Deprecated version validation
        writeln!(
			builder,
			"#[cfg(all({CFG_AUTOKEN_CHECKING_VERSIONS}, {correct_major_cfg}, not({CFG_DEPRECATED_MINOR_OR_LESS_IS_PREFIX}{DEPRECATED_FORMAT_MINOR})))]"
		)
		.unwrap();
        writeln!(
            builder,
            "const _: () = autoken_minor_version_deprecated();\n"
        )
        .unwrap();

        writeln!(builder, "#[allow(dead_code)]").unwrap();
        writeln!(
			builder,
			"#[deprecated(note = \"This minor version of the AuToken format has been deprecated: this version \
			of the AuToken userland crate is severely outdated and will likely soon become incompatible with \
			a future version of the analyzer. You are strongly encouraged to upgrade your userland crate! \
			{userland_mismatch_warn_suffix}\")]",
		)
        .unwrap();
        writeln!(
            builder,
            "const fn autoken_minor_version_deprecated() {{}}\n"
        )
        .unwrap();

        builder
    };

    fs::write(p_version_check, version_check_file).unwrap();
}
