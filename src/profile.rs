use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, ErrorKind};

/// A validated Cargo profile name, such as `dev`, `release` or a custom profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String")]
pub struct Profile(String);

impl Profile {
    /// Returns the name passed to Cargo.
    pub fn name(&self) -> &str {
        &self.0
    }

    // Cargo shares output directories for its built-in build and test profiles.
    // https://doc.rust-lang.org/cargo/reference/build-cache.html
    pub(crate) fn directory(&self) -> &str {
        match self.name() {
            "dev" | "test" => "debug",
            "release" | "bench" => "release",
            name => name,
        }
    }
}

impl FromStr for Profile {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        name.to_owned().try_into()
    }
}

impl TryFrom<String> for Profile {
    type Error = Error;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(Error::new(
                ErrorKind::InvalidProfile,
                "profile names must contain only letters, digits, '_' or '-'",
            ));
        }
        Ok(Self(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_share_cargos_directories_and_custom_profiles_keep_their_names() {
        for (name, directory) in [
            ("dev", "debug"),
            ("test", "debug"),
            ("release", "release"),
            ("bench", "release"),
            ("bench-debug", "bench-debug"),
        ] {
            let profile: Profile = name.parse().unwrap();
            assert_eq!(profile.directory(), directory);
        }
    }

    #[test]
    fn a_profile_cannot_escape_its_firmware_output_directory() {
        for name in ["", ".", "..", "../release", "/tmp/image", "a\\b", "a b"] {
            assert!(name.parse::<Profile>().is_err(), "{name:?}");
        }
        for name in ["dev", "release", "debug_2", "test-fast"] {
            assert!(name.parse::<Profile>().is_ok(), "{name:?}");
        }
    }
}
