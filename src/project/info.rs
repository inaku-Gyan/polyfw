use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cargo::{model::Selection, runner::firmware_elf_path, scope};
use crate::config::model::Registry;
use crate::{Error, ErrorKind, Profile, Result};

use super::{context::Context, selection::SavedSelection};

/// A read-only report on the resolved firmware and any saved selection.
#[derive(Debug)]
#[non_exhaustive]
pub struct ProjectInfo {
    pub root: PathBuf,
    pub saved_selection: Option<SavedSelection>,
    pub platforms: BTreeMap<String, PlatformInfo>,
    pub firmwares: BTreeMap<String, FirmwareInfo>,
}

/// Platform declarations and the number of firmware entries that refer to them.
#[derive(Debug)]
#[non_exhaustive]
pub struct PlatformInfo {
    pub description: Option<String>,
    pub registered_firmwares: usize,
    pub target: String,
    pub cpu: String,
    pub chip: String,
}

impl PlatformInfo {
    pub(crate) fn load(registry: &Registry, id: &str) -> Self {
        let platform = &registry.platforms[id];
        Self {
            description: platform.description.clone(),
            registered_firmwares: registry
                .firmwares
                .values()
                .filter(|firmware| firmware.platform == id)
                .count(),
            target: platform.target.clone(),
            cpu: platform.cpu.clone(),
            chip: platform.chip.clone(),
        }
    }
}

/// Resolved Cargo information and known artifact locations for one firmware.
#[derive(Debug)]
#[non_exhaustive]
pub struct FirmwareInfo {
    pub description: Option<String>,
    pub platform: String,
    pub package: String,
    pub bin: String,
    pub workspace: PathBuf,
    pub working_dir: PathBuf,
    pub target_dir: PathBuf,
    pub build_dir: Option<PathBuf>,
    pub target: String,
    pub check_platform: bool,
    pub artifacts: Vec<Artifact>,
    /// Present in the detailed firmware report.
    pub local_dependencies: Option<Vec<DependencyInfo>>,
}

/// A local package included in a detailed firmware report.
#[derive(Debug)]
#[non_exhaustive]
pub struct DependencyInfo {
    pub name: String,
    pub manifest_path: PathBuf,
    /// Whether this dependency participates in the target-side check scope.
    pub embedded: bool,
}

/// Expected ELF location and a snapshot of whether a file is present there.
#[derive(Debug)]
#[non_exhaustive]
pub struct Artifact {
    pub firmware: String,
    /// The explicit or inherited profile. Omission uses Cargo's build default.
    pub profile: Option<Profile>,
    pub path: PathBuf,
    pub exists: bool,
}

impl Artifact {
    pub(crate) fn from_selection(selection: &Selection) -> Self {
        let path = firmware_elf_path(selection);
        Self {
            firmware: selection.id.clone(),
            profile: selection.profile.clone(),
            exists: path.is_file(),
            path,
        }
    }
}

impl ProjectInfo {
    pub(crate) fn load(context: &Context, firmware: Option<&str>) -> Result<Self> {
        let saved_selection = context.read_saved_selection()?;
        let selections = match firmware {
            Some(id) => vec![context.selection(id)?],
            None => context
                .registry
                .firmwares
                .keys()
                .map(|id| context.selection(id))
                .collect::<Result<Vec<_>>>()?,
        };
        let mut profiles = vec!["dev".parse::<Profile>()?, "release".parse()?];
        if let Some(profile) = saved_selection
            .as_ref()
            .and_then(|saved| saved.profile.as_ref())
            .filter(|profile| !profiles.contains(profile))
        {
            profiles.push(profile.clone());
        }

        let mut platforms = BTreeMap::new();
        let mut firmwares = BTreeMap::new();
        for mut selection in selections {
            platforms
                .entry(selection.firmware.platform.clone())
                .or_insert_with(|| {
                    PlatformInfo::load(&context.registry, &selection.firmware.platform)
                });
            let local_dependencies = if firmware.is_some() {
                let scope = scope::resolve(
                    &context.root,
                    &context.registry.config.local_dependencies,
                    std::slice::from_ref(&selection),
                )
                .map_err(|error| Error::new(ErrorKind::Cargo, error))?
                .into_iter()
                .next()
                .expect("one firmware has one platform scope");
                Some(
                    scope
                        .packages
                        .into_iter()
                        .filter(|package| !scope.firmware.contains(&package.manifest_path))
                        .map(|package| DependencyInfo {
                            name: package.name,
                            manifest_path: package.manifest_path,
                            embedded: package.embedded,
                        })
                        .collect(),
                )
            } else {
                None
            };
            let artifacts = profiles
                .iter()
                .map(|profile| {
                    selection.profile = Some(profile.clone());
                    Artifact::from_selection(&selection)
                })
                .collect();
            firmwares.insert(
                selection.id,
                FirmwareInfo {
                    description: selection.firmware.description,
                    platform: selection.firmware.platform,
                    package: selection.firmware.package,
                    bin: selection.resolved_bin,
                    workspace: selection.cargo.workspace,
                    working_dir: selection.cargo.working_dir,
                    target_dir: selection.cargo.target_dir,
                    build_dir: selection.cargo.build_dir,
                    target: selection.cargo.target,
                    check_platform: selection
                        .firmware
                        .check_platform
                        .or(selection.platform.check_platform)
                        .unwrap_or(true),
                    artifacts,
                    local_dependencies,
                },
            );
        }
        Ok(Self {
            root: context.root.clone(),
            saved_selection,
            platforms,
            firmwares,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn an_overview_checks_elf_files_without_resolving_the_dependency_graph() {
        let context = fixture("overview");
        let artifact_dir = context
            .root
            .join("target/firmware/test/thumbv7em-none-eabihf");
        fs::create_dir_all(artifact_dir.join("debug")).unwrap();
        fs::write(artifact_dir.join("debug/robot-image"), "existing ELF").unwrap();
        fs::create_dir_all(artifact_dir.join("release/robot-image")).unwrap();
        fs::remove_file(context.root.join("platform/test/host-helper/Cargo.toml")).unwrap();

        let report = ProjectInfo::load(&context, None).unwrap();
        assert_eq!(report.firmwares.len(), 2);
        assert_eq!(report.platforms["test"].registered_firmwares, 2);
        assert!(report.saved_selection.is_none());
        assert_eq!(report.firmwares["one"].bin, "robot-image");
        for firmware in report.firmwares.values() {
            assert!(firmware.local_dependencies.is_none());
            assert_eq!(firmware.artifacts.len(), 2);
            assert!(
                firmware
                    .artifacts
                    .iter()
                    .all(|artifact| artifact.path.is_absolute())
            );
        }
        let artifacts = &report.firmwares["one"].artifacts;
        assert_eq!(artifacts[0].profile.as_ref().unwrap().name(), "dev");
        assert_eq!(artifacts[0].path, artifact_dir.join("debug/robot-image"));
        assert!(artifacts[0].exists);
        assert_eq!(artifacts[1].profile.as_ref().unwrap().name(), "release");
        assert!(!artifacts[1].exists);
        assert!(ProjectInfo::load(&context, Some("one")).is_err());
        assert!(!context.selection_path().exists());
        assert!(
            !context
                .root
                .join(super::super::vscode::LAUNCH_FILE)
                .exists()
        );
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn firmware_details_list_local_dependencies_without_changing_selection() {
        let context = fixture("detail with spaces");
        context
            .save_selection("two", Some(&"bench-debug".parse().unwrap()))
            .unwrap();
        let state = fs::read(context.selection_path()).unwrap();
        let launch_path = context.root.join(super::super::vscode::LAUNCH_FILE);
        let launch = fs::read(&launch_path).unwrap();

        let report = ProjectInfo::load(&context, Some("one")).unwrap();
        assert_eq!(report.firmwares.len(), 1);
        assert_eq!(report.platforms.len(), 1);
        let firmware = &report.firmwares["one"];
        let packages = firmware
            .local_dependencies
            .as_ref()
            .unwrap()
            .iter()
            .map(|package| (package.name.as_str(), package.embedded))
            .collect::<Vec<_>>();
        assert_eq!(
            packages,
            [
                ("common", true),
                ("common-leaf", true),
                ("host-helper", false)
            ]
        );
        assert_eq!(firmware.artifacts.len(), 3);
        assert_eq!(
            firmware.artifacts[2].profile.as_ref().unwrap().name(),
            "bench-debug"
        );
        assert_eq!(
            firmware.artifacts[2].path,
            context
                .root
                .join("target/firmware/test/thumbv7em-none-eabihf/bench-debug/robot-image")
        );
        let saved = report.saved_selection.as_ref().unwrap();
        assert_eq!(saved.firmware, "two");
        assert_eq!(saved.profile.as_ref().unwrap().name(), "bench-debug");
        assert_eq!(fs::read(context.selection_path()).unwrap(), state);
        assert_eq!(fs::read(launch_path).unwrap(), launch);
        assert!(!context.root.join("target").exists());

        assert_eq!(firmware.bin, "robot-image");
        assert_eq!(report.platforms["test"].target, "thumbv7em-none-eabihf");
        let manifest_path = |directory: &str| {
            PathBuf::from("platform")
                .join("test")
                .join(directory)
                .join("Cargo.toml")
        };
        let manifests = firmware
            .local_dependencies
            .as_ref()
            .unwrap()
            .iter()
            .map(|package| package.manifest_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            manifests,
            [
                manifest_path("common"),
                manifest_path("leaf"),
                manifest_path("host-helper")
            ]
        );
        assert!(!report.firmwares.contains_key("two"));
        let report = ProjectInfo::load(&context, Some("two")).unwrap();
        assert!(
            report.firmwares["two"]
                .local_dependencies
                .as_ref()
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn descriptions_are_independent_and_keep_explicit_values() {
        let mut context = fixture("descriptions");
        for platform in [None, Some(""), Some(" \t"), Some("Demo board")] {
            for firmware in [None, Some(""), Some(" \t"), Some("Demo firmware")] {
                context
                    .registry
                    .platforms
                    .get_mut("test")
                    .unwrap()
                    .description = platform.map(str::to_owned);
                context
                    .registry
                    .firmwares
                    .get_mut("one")
                    .unwrap()
                    .description = firmware.map(str::to_owned);
                let report = ProjectInfo::load(&context, None).unwrap();
                assert_eq!(report.platforms["test"].description.as_deref(), platform);
                assert_eq!(report.firmwares["one"].description.as_deref(), firmware);
            }
        }
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn reports_include_explicitly_saved_profiles() {
        let context = fixture("saved profiles");
        assert!(
            ProjectInfo::load(&context, None)
                .unwrap()
                .saved_selection
                .is_none()
        );
        for profile in [None, Some("dev"), Some("release"), Some("bench-debug")] {
            context
                .save_selection("one", profile.map(|name| name.parse().unwrap()).as_ref())
                .unwrap();
            let report = ProjectInfo::load(&context, None).unwrap();
            let expected_count = if profile == Some("bench-debug") { 3 } else { 2 };
            for firmware in report.firmwares.values() {
                assert_eq!(firmware.artifacts.len(), expected_count);
            }
            let saved = report.saved_selection.unwrap();
            assert_eq!(saved.firmware, "one");
            assert_eq!(saved.profile.as_ref().map(Profile::name), profile);
            assert!(report.firmwares["one"].local_dependencies.is_none());
        }
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn info_lists_current_firmwares_without_discarding_a_stale_saved_selection() {
        let context = fixture("stale selection");
        let saved = "firmware = \"removed-firmware\"\nprofile = \"release\"\n";
        fs::create_dir_all(context.selection_path().parent().unwrap()).unwrap();
        fs::write(context.selection_path(), saved).unwrap();
        for firmware in [None, Some("one")] {
            let report = ProjectInfo::load(&context, firmware).unwrap();
            assert!(report.firmwares.contains_key("one"));
            assert!(!report.firmwares.contains_key("removed-firmware"));
            let selection = report.saved_selection.unwrap();
            assert_eq!(selection.firmware, "removed-firmware");
            assert_eq!(selection.profile.as_ref().unwrap().name(), "release");
            assert_eq!(fs::read_to_string(context.selection_path()).unwrap(), saved);
        }
        let error = context.read_selection().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::StaleSelection);
        assert!(error.to_string().contains("removed-firmware"), "{error}");
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn an_info_query_reports_invalid_saved_state_and_unknown_firmwares() {
        let context = fixture("errors");
        assert!(
            ProjectInfo::load(&context, Some("unknown"))
                .err()
                .unwrap()
                .to_string()
                .contains("unknown firmware: unknown")
        );
        fs::create_dir_all(context.selection_path().parent().unwrap()).unwrap();
        fs::write(context.selection_path(), "invalid = [").unwrap();
        let error = ProjectInfo::load(&context, Some("one")).err().unwrap();
        assert!(error.to_string().contains("invalid saved selection"));
        assert_eq!(
            fs::read_to_string(context.selection_path()).unwrap(),
            "invalid = ["
        );
        fs::remove_dir_all(context.root).unwrap();
    }

    fn fixture(label: &str) -> Context {
        let root = std::env::temp_dir().join(format!("polyfw-info-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("platform/test");
        fs::create_dir_all(workspace.join(".cargo")).unwrap();
        fs::write(
            workspace.join(".cargo/config.toml"),
            "[build]\ntarget = \"thumbv7em-none-eabihf\"\n\
             [target.thumbv7em-none-eabihf]\nrustflags = \"-Ctarget-cpu=cortex-m4\"\n",
        )
        .unwrap();
        for (directory, name, extra) in [
            (
                "firmware",
                "firmware-fw",
                "[[bin]]\nname = \"robot-image\"\npath = \"src/main.rs\"\n\
                 [dependencies]\ncommon = { path = \"../common\" }\n\
                 [build-dependencies]\nhost-helper = { path = \"../host-helper\" }\n",
            ),
            ("other", "other-fw", ""),
            (
                "common",
                "common",
                "[dependencies]\ncommon-leaf = { path = \"../leaf\" }\n",
            ),
            ("leaf", "common-leaf", ""),
            ("host-helper", "host-helper", ""),
            ("unused", "unused", ""),
        ] {
            let package = workspace.join(directory);
            fs::create_dir_all(package.join("src")).unwrap();
            fs::write(
                package.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\
                     edition = \"2024\"\n{extra}"
                ),
            )
            .unwrap();
            fs::write(package.join("src/lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
            if matches!(directory, "firmware" | "other") {
                fs::write(package.join("src/main.rs"), "fn main() {}\n").unwrap();
            }
        }
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nresolver = \"3\"\n\
             members = [\"firmware\", \"other\", \"common\", \"leaf\", \"host-helper\", \"unused\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("polyfw.toml"),
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.one]\ndescription = \"first firmware\"\nplatform = \"test\"\n\
             package = \"firmware-fw\"\n\
             [firmwares.two]\ndescription = \"other firmware\"\nplatform = \"test\"\n\
             package = \"other-fw\"\n",
        )
        .unwrap();
        let output = crate::cargo::runner::workspace_cargo(&workspace)
            .args(["generate-lockfile", "--offline"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        Context::load(root).unwrap()
    }
}
