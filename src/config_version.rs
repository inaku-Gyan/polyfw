use std::path::Path;

use semver::Version;

use crate::Result;

pub(crate) const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Raise only when the current implementation can no longer preserve older contracts.
pub(crate) const MIN_SUPPORTED_CONFIG_VERSION: &str = "0.2.0";
/// Conservative authoring guidance; raise when new configurations need a newer CLI.
pub(crate) const MIN_REQUIRED_CLI_VERSION: &str = "0.2.0";

pub(crate) fn preflight(source: &str, path: &Path) -> Result<()> {
    let document: toml::Table = toml::from_str(source)
        .map_err(|error| format!("invalid registry {}: {error}", path.display()))?;
    let declaration_error = |reason: String| -> Box<dyn std::error::Error + Send + Sync> {
        format!(
                "{}: {reason}; this CLI is {CLI_VERSION}. Review the configuration contract before \
                 adding config-version and min-polyfw-version; the current authoring baseline \
                 is config-version = \"{CLI_VERSION}\", min-polyfw-version = \
                 \"{MIN_REQUIRED_CLI_VERSION}\". See \
                 https://github.com/inaku-Gyan/polyfw/blob/v{CLI_VERSION}/docs/configuration.md#configuration-compatibility",
                path.display()
            ).into()
    };
    let declared = |key: &str| -> Result<&str> {
        match document.get(key) {
            Some(toml::Value::String(value)) => Ok(value),
            Some(_) => Err(declaration_error(format!(
                "{key} must be a string containing a full SemVer release version"
            ))),
            None => Err(declaration_error(format!(
                "missing required top-level {key}"
            ))),
        }
    };
    check(
        declared("config-version")?,
        declared("min-polyfw-version")?,
        path,
    )?;
    Ok(())
}

pub(crate) fn check(config: &str, minimum_cli: &str, path: &Path) -> Result<()> {
    check_against(
        config,
        minimum_cli,
        path,
        CLI_VERSION,
        MIN_SUPPORTED_CONFIG_VERSION,
    )
}

fn check_against(
    config: &str,
    minimum_cli: &str,
    path: &Path,
    cli: &str,
    oldest_config: &str,
) -> Result<()> {
    let cli_version = Version::parse(cli).expect("CLI version must be valid SemVer");
    let oldest = Version::parse(oldest_config).expect("configuration floor must be valid SemVer");
    let parse = |key: &str, value: &str| -> Result<Version> {
        Version::parse(value).map_err(|error| {
            format!(
                "{}: {key} = {value:?} is not a full SemVer release version: {error}; \
                 this CLI is {cli} (oldest supported configuration: {oldest_config})",
                path.display()
            )
            .into()
        })
    };
    let config_version = parse("config-version", config)?;
    let minimum = parse("min-polyfw-version", minimum_cli)?;
    if minimum.cmp_precedence(&config_version).is_gt() {
        return Err(format!(
            "{}: min-polyfw-version = {minimum_cli:?} exceeds config-version = {config:?}; \
             this CLI is {cli}. Correct the declarations to match the configuration's contract",
            path.display()
        )
        .into());
    }
    if config_version.cmp_precedence(&oldest).is_lt() {
        return Err(format!(
            "{}: config-version = {config:?} is too old for polyfw {cli}; oldest supported \
             configuration: {oldest_config}. Use a CLI compatible with this contract or \
             manually migrate the configuration; changing version declarations alone is not \
             a migration",
            path.display()
        )
        .into());
    }
    if minimum.cmp_precedence(&cli_version).is_gt() {
        return Err(format!(
            "{}: min-polyfw-version = {minimum_cli:?} requires polyfw >= {minimum_cli}, \
             but this CLI is {cli} (config-version = {config:?}). Upgrade the CLI",
            path.display()
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) const TEST_HEADER: &str = concat!(
    "config-version = \"",
    env!("CARGO_PKG_VERSION"),
    "\"\n",
    "min-polyfw-version = \"",
    env!("CARGO_PKG_VERSION"),
    "\"\n",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_is_directional_and_old_support_can_be_dropped() {
        let path = Path::new("project/polyfw.toml");
        for (config, minimum, cli, floor, expected) in [
            ("0.2.0", "0.1.0", "0.2.0", "0.1.0", None),
            ("0.1.0", "0.1.0", "0.2.0", "0.1.0", None),
            ("0.2.0", "0.1.0", "0.1.0", "0.1.0", None),
            ("0.2.0", "0.2.0", "0.1.0", "0.1.0", Some("Upgrade the CLI")),
            ("0.1.0", "0.1.0", "0.3.0", "0.2.0", Some("manually migrate")),
            ("0.2.0", "0.2.0", "0.3.0", "0.2.0", None),
            (
                "0.1.0",
                "0.2.0",
                "0.2.0",
                "0.1.0",
                Some("exceeds config-version"),
            ),
            (
                "0.2",
                "0.1.0",
                "0.2.0",
                "0.1.0",
                Some("config-version = \"0.2\""),
            ),
            (
                "0.2.0",
                ">=0.1.0",
                "0.2.0",
                "0.1.0",
                Some("min-polyfw-version"),
            ),
            (
                "0.2.0-rc.2",
                "0.2.0-rc.2",
                "0.2.0-rc.10",
                "0.2.0-rc.2",
                None,
            ),
            (
                "0.2.0",
                "0.2.0",
                "0.2.0-rc.10",
                "0.1.0",
                Some("Upgrade the CLI"),
            ),
            ("0.2.0+a", "0.2.0+z", "0.2.0+b", "0.2.0+y", None),
        ] {
            let result = check_against(config, minimum, path, cli, floor);
            if let Some(expected) = expected {
                let error = result.unwrap_err().to_string();
                assert!(error.contains(expected), "{error}");
                assert!(error.contains("project/polyfw.toml"), "{error}");
                assert!(error.contains(cli), "{error}");
            } else {
                result.unwrap();
            }
        }
    }

    #[test]
    fn release_baselines_are_consistent() {
        let cli = Version::parse(CLI_VERSION).unwrap();
        let floor = Version::parse(MIN_SUPPORTED_CONFIG_VERSION).unwrap();
        let minimum = Version::parse(MIN_REQUIRED_CLI_VERSION).unwrap();
        assert!(!floor.cmp_precedence(&cli).is_gt());
        assert!(!minimum.cmp_precedence(&cli).is_gt());
        check(
            CLI_VERSION,
            MIN_REQUIRED_CLI_VERSION,
            Path::new("polyfw.toml"),
        )
        .unwrap();
    }

    #[test]
    fn preflight_accepts_uninterpreted_fields_but_still_requires_valid_toml() {
        let path = Path::new("polyfw.toml");
        let source = format!("{TEST_HEADER}[future]\nsetting = 42\n");
        preflight(&source, path).unwrap();
        let error = preflight(&format!("{source}broken = ["), path)
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid registry polyfw.toml"), "{error}");
    }
}
