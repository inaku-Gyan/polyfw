use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::model::{CargoContext, Metadata};
use super::runner;
use crate::InternalResult as Result;

#[derive(Default)]
pub(super) struct PlatformConditions {
    pub(super) host: BTreeSet<String>,
    firmware: Option<BTreeSet<String>>,
}

impl PlatformConditions {
    pub(super) fn load(
        cargo: &CargoContext,
        metadata: &Metadata,
        target: Option<&str>,
    ) -> Result<Self> {
        let graph = metadata
            .resolve
            .as_ref()
            .ok_or("Cargo metadata has no dependency graph")?;
        let conditions = graph
            .nodes
            .iter()
            .flat_map(|node| &node.deps)
            .flat_map(|dependency| &dependency.dep_kinds)
            .filter_map(|kind| kind.target.as_deref())
            .collect::<BTreeSet<_>>();
        if conditions.is_empty() {
            return Ok(Self::default());
        }

        // Filtered Cargo metadata keeps all dependency kinds between two included packages.
        // Give each condition its own path package so Cargo reports individual matches.
        let scratch = tempfile::Builder::new()
            .prefix("polyfw-platforms-")
            .tempdir()?;
        let probe = scratch.path().join("probe");
        fs::create_dir(&probe)?;
        fs::write(scratch.path().join("lib.rs"), "")?;
        let mut manifest = String::from(
            "[package]\nname = \"polyfw-platform-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\
             [workspace]\n[lib]\npath = \"../lib.rs\"\n",
        );
        let mut packages = BTreeMap::new();
        for (index, condition) in conditions.into_iter().enumerate() {
            let name = format!("polyfw-condition-{index}");
            let directory = scratch.path().join(&name);
            fs::create_dir(&directory)?;
            fs::write(
                directory.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\
                     [lib]\npath = \"../lib.rs\"\n"
                ),
            )?;
            let key = serde_json::to_string(condition)?;
            writeln!(
                manifest,
                "[target.{key}.dependencies]\n{name} = {{ path = \"../{name}\" }}"
            )?;
            packages.insert(name, condition);
        }
        let manifest_path = probe.join("Cargo.toml");
        fs::write(&manifest_path, manifest)?;

        let host = matching_conditions(cargo, &manifest_path, "host-tuple", &packages)?;
        let firmware = target
            .filter(|target| *target != "host-tuple")
            .map(|target| matching_conditions(cargo, &manifest_path, target, &packages))
            .transpose()?;
        Ok(Self { host, firmware })
    }

    pub(super) fn firmware(&self) -> &BTreeSet<String> {
        self.firmware.as_ref().unwrap_or(&self.host)
    }
}

fn matching_conditions(
    cargo: &CargoContext,
    manifest: &Path,
    target: &str,
    packages: &BTreeMap<String, &str>,
) -> Result<BTreeSet<String>> {
    let output = runner::cargo_command(cargo)
        .args([
            "metadata",
            "--offline",
            "--format-version",
            "1",
            "--filter-platform",
            target,
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "could not resolve Cargo target conditions for {target} in {}: {}",
            cargo.working_dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
    Ok(metadata
        .packages
        .iter()
        .filter_map(|package| packages.get(&package.name))
        .map(|condition| (*condition).to_owned())
        .collect())
}
