use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::InternalResult as Result;
use crate::config::{REGISTRY_FILE, model::Registry};

use super::model::{CargoContext, FirmwareCargo, Metadata, ResolvedBin, Selection};
use super::runner;

pub(crate) fn resolve(root: &Path, registry: &Registry) -> Result<BTreeMap<String, FirmwareCargo>> {
    let mut resolved = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    for (id, firmware) in &registry.firmwares {
        let platform = &registry.platforms[&firmware.platform];
        let working_dir = canonical(
            &root.join(
                firmware
                    .working_dir
                    .as_deref()
                    .or(platform.working_dir.as_deref())
                    .unwrap_or("."),
            ),
        )?;
        if !working_dir.is_dir() {
            return Err(format!(
                "firmware {id} working-dir is not a directory: {}",
                working_dir.display()
            )
            .into());
        }
        let build_dir_override = firmware
            .build_dir
            .as_ref()
            .or(platform.build_dir.as_ref())
            .map(|value| absolute_directory(root, value));
        let target_dir_override = firmware
            .target_dir
            .as_ref()
            .or(platform.target_dir.as_ref())
            .map(|value| absolute_directory(root, value));
        let mut cargo = CargoContext {
            target: runner::resolve_target(
                &working_dir,
                &platform.target,
                build_dir_override.is_some(),
            )?,
            working_dir,
            target_dir_override,
            build_dir_override,
            ..CargoContext::default()
        };
        let key = (cargo.clone(), platform.target.clone());
        if !metadata.contains_key(&key) {
            metadata.insert(
                key.clone(),
                cargo_metadata_for(&cargo, Some(&platform.target), false)?,
            );
        }
        let data = &metadata[&key];
        let package = data.workspace_package(&firmware.package).ok_or_else(|| {
            format!(
                "firmware {id} package is not a member of the Cargo workspace: {}",
                firmware.package
            )
        })?;
        if !package
            .targets
            .iter()
            .any(|target| target.kind.iter().any(|kind| kind == "lib"))
        {
            return Err(format!(
                "firmware {id} package {} has no library target",
                firmware.package
            )
            .into());
        }
        let bins = package
            .targets
            .iter()
            .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
            .collect::<Vec<_>>();
        let bin = match firmware.bin.as_deref() {
            Some(bin) => bins
                .iter()
                .copied()
                .find(|target| target.name == bin)
                .ok_or_else(|| {
                    format!(
                        "firmware {id} package {} has no binary target named {bin}",
                        firmware.package
                    )
                })?,
            None => match bins.as_slice() {
                [bin] => *bin,
                [] => {
                    return Err(format!(
                        "firmware {id} package {} has no binary targets",
                        firmware.package
                    )
                    .into());
                }
                _ => {
                    let names = bins
                        .iter()
                        .map(|target| target.name.as_str())
                        .collect::<Vec<_>>();
                    return Err(format!("firmware {id} package {} has multiple binary targets: {}; set bin in {REGISTRY_FILE}", firmware.package, names.join(", ")).into());
                }
            },
        };
        cargo.workspace = canonical(Path::new(&data.workspace_root))?;
        cargo.target_dir = data.target_directory.clone();
        cargo.build_dir = data
            .build_directory
            .clone()
            .or_else(|| cargo.build_dir_override.clone());
        resolved.insert(
            id.clone(),
            FirmwareCargo {
                cargo,
                bin: ResolvedBin {
                    name: bin.name.clone(),
                    package_id: package.id.clone(),
                    required_features: bin.required_features.clone(),
                },
                manifest_path: package.manifest_path.clone(),
            },
        );
    }
    Ok(resolved)
}

fn absolute_directory(root: &Path, value: &str) -> PathBuf {
    let path = root.join(value);
    // Preserve native filesystem semantics for parent components following symlinks.
    path.canonicalize().unwrap_or(path)
}

pub(crate) fn cargo_metadata_for(
    context: &CargoContext,
    target: Option<&str>,
    dependencies: bool,
) -> Result<Metadata> {
    let mut command = runner::cargo_command(context);
    command.args(["metadata", "--locked", "--format-version", "1"]);
    if let Some(target) = target {
        // metadata has no --target switch; align its native target default with builds.
        command.env("CARGO_BUILD_TARGET", target);
    }
    if !dependencies {
        command.arg("--no-deps");
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed for {}: {}",
            context.working_dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn canonical(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()).into())
}

pub(crate) fn same_context(left: &Selection, right: &Selection) -> bool {
    left.cargo == right.cargo && left.platform.target == right.platform.target
}

pub(crate) fn unique_contexts(selections: &[Selection]) -> Vec<&Selection> {
    let mut unique: Vec<&Selection> = Vec::new();
    for selection in selections {
        if !unique
            .iter()
            .any(|previous| same_context(previous, selection))
        {
            unique.push(selection);
        }
    }
    unique
}
