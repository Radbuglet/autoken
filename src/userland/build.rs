use semver::{Version, VersionReq};

include!("INTERFACE_VERSION.in");

fn main() {
    // Don't rebuild this crate when nothing changed.
    println!("cargo:rerun-if-changed=build.rs");

    // Get environment variables
    let my_version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let tool_version = get_opt_env("AUTOKEN_ANALYZER_VERSION").unwrap_or_else(|| {
        "<unknown (missing `AUTOKEN_ANALYZER_VERSION` environment variable)>".to_string()
    });

    let my_interface_version = Version::parse(MY_INTERFACE_VERSION).unwrap();

    let supported_range = get_opt_env("AUTOKEN_ANALYZER_SUPPORTED_RANGE");
    let upgrade_message = get_fallback_env(
        "AUTOKEN_ANALYZER_UPGRADE_MESSAGE",
        "No upgrade message was provided by the analyzer.",
    );

    let deprecated_range = get_opt_env("AUTOKEN_ANALYZER_DEPRECATED_RANGE");
    let deprecation_message = get_fallback_env(
        "AUTOKEN_ANALYZER_DEPRECATION_MESSAGE",
        "No upgrade message was provided by the analyzer.",
    );

    // Handle the supported range.
    if let Some(supported_range) = supported_range
        .and_then(|v| parse_semver_req_or_err("AUTOKEN_ANALYZER_SUPPORTED_RANGE", &v))
    {
        if !supported_range.matches(&my_interface_version) {
            println!(
                "cargo:warning=Userland crate `autoken {my_version}` is not compatible with \
				 autoken static analyzer tool version {tool_version} as its interface version \
				 {my_interface_version} does not meet the supported interface version range \
				 {supported_range}. {upgrade_message}"
            );
        }
    }

    // Handle the deprecation range.
    if let Some(deprecated_range) = deprecated_range
        .and_then(|v| parse_semver_req_or_err("AUTOKEN_ANALYZER_DEPRECATED_RANGE", &v))
    {
        if deprecated_range.matches(&my_interface_version) {
            println!(
                "cargo:warning=Userland crate `autoken {my_version}` is deprecated according to \
				 autoken static analyzer tool version {tool_version} as its interface version \
				 {my_interface_version} is in the interface version deprecation range {deprecated_range}. \
				 {deprecation_message}"
            );
        }
    }
}

fn get_opt_env(var: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={var}");
    std::env::var(var).ok()
}

fn get_fallback_env(var: &str, fallback: &str) -> String {
    get_opt_env(var).unwrap_or_else(|| fallback.to_string())
}

fn parse_semver_req_or_err(var: &str, val: &str) -> Option<VersionReq> {
    match VersionReq::parse(val) {
        Ok(req) => Some(req),
        Err(err) => {
            println!(
                "cargo:warn=The environment variable `{var}` is not a valid semver version range: \
				 {err}. Offending input: {val:?}."
            );
            None
        }
    }
}
