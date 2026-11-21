use std::path::PathBuf;

use serde::Deserialize;

use crate::Profile;
use crate::config::model::{Firmware, Platform};

/// Resolved Cargo invocation identity; workspace ownership and invocation cwd are distinct.
#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CargoContext {
    pub(crate) working_dir: PathBuf,
    pub(crate) workspace: PathBuf,
    pub(crate) target_dir: PathBuf,
    pub(crate) build_dir: Option<PathBuf>,
    pub(crate) target_dir_override: Option<PathBuf>,
    pub(crate) build_dir_override: Option<PathBuf>,
    /// Concrete triple or custom-target file stem for Cargo's public output layout.
    pub(crate) target: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Metadata {
    pub(crate) workspace_root: String,
    pub(crate) target_directory: PathBuf,
    #[serde(default)]
    pub(crate) build_directory: Option<PathBuf>,
    pub(crate) workspace_members: Vec<String>,
    pub(crate) packages: Vec<MetadataPackage>,
    pub(crate) resolve: Option<MetadataResolve>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataPackage {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) source: Option<String>,
    pub(crate) manifest_path: PathBuf,
    pub(crate) targets: Vec<MetadataTarget>,
}

impl Metadata {
    pub(crate) fn workspace_package(&self, name: &str) -> Option<&MetadataPackage> {
        self.packages
            .iter()
            .find(|package| package.name == name && self.workspace_members.contains(&package.id))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataResolve {
    pub(crate) nodes: Vec<MetadataNode>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataNode {
    pub(crate) id: String,
    pub(crate) deps: Vec<MetadataDependency>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataDependency {
    pub(crate) pkg: String,
    pub(crate) dep_kinds: Vec<MetadataDependencyKind>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataDependencyKind {
    pub(crate) kind: Option<DependencyKind>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DependencyKind {
    Build,
    Dev,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataTarget {
    pub(crate) name: String,
    pub(crate) kind: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct Selection {
    pub(crate) id: String,
    pub(crate) firmware: Firmware,
    pub(crate) platform: Platform,
    pub(crate) profile: Option<Profile>,
    pub(crate) cargo: CargoContext,
    pub(crate) resolved_bin: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FirmwareCargo {
    pub(crate) cargo: CargoContext,
    pub(crate) bin: String,
}

#[cfg(test)]
impl CargoContext {
    pub(crate) fn fixture(root: &std::path::Path, id: &str) -> Self {
        let target_dir = root.join("target/firmware").join(id);
        Self {
            working_dir: root.join("platform/test"),
            workspace: root.join("platform/test"),
            target_dir: target_dir.clone(),
            target_dir_override: Some(target_dir),
            target: "thumbv7em-none-eabihf".into(),
            ..Self::default()
        }
    }
}
