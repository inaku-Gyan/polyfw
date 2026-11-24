pub(crate) mod local_dependencies;
pub(crate) mod model;
pub(crate) mod version;

use std::fs;
use std::path::Path;

use crate::{Error, ErrorKind, InternalResult, Result};
use local_dependencies::LocalDependencies;
use model::Registry;

pub(crate) const REGISTRY_FILE: &str = "polyfw.toml";

/// Generates the JSON Schema for this release's project configuration.
pub fn configuration_schema() -> Result<String> {
    let schema = schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<Registry>();
    serde_json::to_string_pretty(&schema)
        .map(|source| format!("{source}\n"))
        .map_err(|error| Error::new(ErrorKind::Configuration, error))
}

/// Writes the configuration schema, creating its parent directories as needed.
pub fn write_configuration_schema(path: impl AsRef<Path>) -> Result<()> {
    crate::files::replace(path.as_ref(), configuration_schema()?.as_bytes())
        .map_err(|error| Error::new(ErrorKind::Configuration, error))
}

impl Registry {
    pub(crate) fn load(root: &Path) -> Result<Self> {
        let path = root.join(REGISTRY_FILE);
        let source = fs::read_to_string(&path).map_err(|error| {
            Error::context(
                ErrorKind::Configuration,
                format!("could not read {}", path.display()),
                error,
            )
        })?;
        version::preflight(&source, &path)
            .map_err(|error| Error::new(ErrorKind::Configuration, error))?;
        let registry: Self = toml::from_str(&source).map_err(|error| {
            Error::context(
                ErrorKind::Configuration,
                format!("invalid registry {}", path.display()),
                error,
            )
        })?;
        registry
            .validate_structure(root)
            .map_err(|error| Error::new(ErrorKind::Configuration, error))?;
        Ok(registry)
    }

    pub(crate) fn validate_structure(&self, root: &Path) -> InternalResult<()> {
        version::check(
            &self.config_version,
            &self.min_polyfw_version,
            &root.join(REGISTRY_FILE),
        )?;
        LocalDependencies::new(root, &self.config.local_dependencies)?;
        if self.platforms.is_empty() {
            return Err("registry has no platforms".into());
        }
        if self.firmwares.is_empty() {
            return Err("registry has no firmwares".into());
        }
        for name in self.platforms.keys().chain(self.firmwares.keys()) {
            if !valid_name(name) {
                return Err(format!("invalid registry name: {name}").into());
            }
        }
        for (id, platform) in &self.platforms {
            for (field, value) in [("target", &platform.target), ("chip", &platform.chip)] {
                if value.trim().is_empty() {
                    return Err(format!("platform {id} has empty {field}").into());
                }
            }
            validate_directories(
                id,
                [
                    &platform.working_dir,
                    &platform.build_dir,
                    &platform.target_dir,
                ],
            )?;
        }
        for (id, firmware) in &self.firmwares {
            if !self.platforms.contains_key(&firmware.platform) {
                return Err(format!(
                    "firmware {id} refers to unknown platform {}",
                    firmware.platform
                )
                .into());
            }
            validate_directories(
                id,
                [
                    &firmware.working_dir,
                    &firmware.build_dir,
                    &firmware.target_dir,
                ],
            )?;
        }
        Ok(())
    }
}

fn validate_directories(id: &str, values: [&Option<String>; 3]) -> InternalResult<()> {
    for value in values.into_iter().flatten() {
        if value.trim().is_empty() || value.contains('\0') {
            return Err(format!("{id} has an empty or invalid directory value").into());
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.ends_with('-')
        && !name.contains("--")
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn registry_names_are_lowercase_slug_names() {
        assert!(valid_name("sentry-chassis"));
        assert!(valid_name("bringup-dm-mc02"));
        assert!(!valid_name("Sentry"));
        assert!(!valid_name("sentry--chassis"));
        assert!(!valid_name("sentry-"));
    }
}
