// === Config === //

pub const CURRENT_FORMAT_MAJOR: u32 = 0;
pub const CURRENT_FORMAT_MINOR: u32 = 1;
pub const DEPRECATED_FORMAT_MINOR: u32 = 0;

// === Logic that shall not be touched === //

const CFG_AUTOKEN_CHECKING_VERSIONS: &str = "__autoken_checking_versions";
const CFG_MAJOR_IS_PREFIX: &str = "__autoken_major_is_";
const CFG_SUPPORTED_MINOR_OR_LESS_IS_PREFIX: &str = "__autoken_supported_minor_or_less_is_";
const CFG_DEPRECATED_MINOR_OR_LESS_IS_PREFIX: &str = "__autoken_deprecated_minor_or_less_is_";

const GLOBAL_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn validate_crate_version_in_build_script() {
    assert_eq!(
        std::env::var("CARGO_PKG_VERSION").as_deref(),
        Ok(GLOBAL_VERSION),
    );
}

pub fn emit_userland_cfg_file() -> String {
    use std::fmt::Write;

    let mut builder = String::new();

    let userland_mismatch_warn_suffix = format!(
        "Userland crate *internal format* version is {CURRENT_FORMAT_MAJOR}.{CURRENT_FORMAT_MINOR}. \
        The userland crate at fault is running AuToken version {GLOBAL_VERSION}. \
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
}

pub fn set_analyzer_cfgs(mut set: impl FnMut(&str)) {
    set(CFG_AUTOKEN_CHECKING_VERSIONS);
    set(&format!("{CFG_MAJOR_IS_PREFIX}{CURRENT_FORMAT_MAJOR}"));

    for v in (DEPRECATED_FORMAT_MINOR + 1)..=CURRENT_FORMAT_MINOR {
        set(&format!("{CFG_SUPPORTED_MINOR_OR_LESS_IS_PREFIX}{v}"));
    }

    for v in 0..=DEPRECATED_FORMAT_MINOR {
        set(&format!("{CFG_DEPRECATED_MINOR_OR_LESS_IS_PREFIX}{v}"));
    }
}
