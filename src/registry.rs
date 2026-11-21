use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::mem::DropGuard;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::config_version;
use crate::local_dependencies::LocalDependencies;
use crate::model::{CargoContext, Metadata, Platform, Registry, SavedSelection, Selection};
use crate::profile::Profile;
use crate::runner;
use crate::vscode;

pub(crate) const REGISTRY_FILE: &str = "polyfw.toml";
pub(crate) const STATE_DIRECTORY: &str = ".polyfw-state";
const SELECTION_FILE: &str = ".polyfw-state/selected";

pub(crate) fn project_root(explicit: Option<&Path>, working_directory: &Path) -> Result<PathBuf> {
    let path = if let Some(explicit) = explicit {
        working_directory.join(explicit)
    } else {
        let mut path = None;
        for directory in working_directory.ancestors() {
            let registry = directory.join(REGISTRY_FILE);
            match fs::symlink_metadata(&registry) {
                Ok(_) => {
                    path = Some(directory.to_owned());
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("could not inspect {}: {error}", registry.display()).into());
                }
            }
        }
        path.ok_or_else(|| {
            format!(
                "no {REGISTRY_FILE} found in {} or its ancestors; use --project-root <DIR>",
                working_directory.display()
            )
        })?
    };
    let root = path.canonicalize().map_err(|error| {
        format!(
            "invalid project root {}: {error}; use --project-root <DIR>",
            path.display()
        )
    })?;
    if !root.is_dir() {
        return Err(format!("project root is not a directory: {}", root.display()).into());
    }
    Ok(root)
}

pub(crate) fn schema_source() -> Result<String> {
    let schema = schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<Registry>();
    Ok(format!("{}\n", serde_json::to_string_pretty(&schema)?))
}

pub(crate) fn write_schema(output: Option<&Path>) -> Result<()> {
    let source = schema_source()?;
    match output {
        Some(path) => write_file(path, source.as_bytes()),
        None => Ok(std::io::stdout().lock().write_all(source.as_bytes())?),
    }
}

pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) registry: Registry,
}

impl Context {
    pub(crate) fn load(root: PathBuf) -> Result<Self> {
        let mut context = Self::load_config(root)?;
        context.resolve_cargo()?;
        Ok(context)
    }

    pub(crate) fn load_config(root: PathBuf) -> Result<Self> {
        let path = root.join(REGISTRY_FILE);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        config_version::preflight(&source, &path)?;
        let registry: Registry = toml::from_str(&source)
            .map_err(|error| format!("invalid registry {}: {error}", path.display()))?;
        registry.validate_structure(&root)?;
        Ok(Self { root, registry })
    }

    pub(crate) fn resolve_cargo(&mut self) -> Result<()> {
        self.registry.resolve_cargo(&self.root)
    }

    pub(crate) fn selection(&self, id: &str) -> Result<Selection> {
        let firmware =
            self.registry.firmwares.get(id).ok_or_else(|| {
                format!("unknown firmware: {id}; run polyfw info for valid names")
            })?;
        let platform = self
            .registry
            .platforms
            .get(&firmware.platform)
            .ok_or_else(|| {
                format!(
                    "firmware {id} refers to unknown PLATFORM {}",
                    firmware.platform
                )
            })?;
        Ok(Selection {
            id: id.to_owned(),
            firmware: firmware.clone(),
            platform: platform.clone(),
            profile: None,
        })
    }

    pub(crate) fn read_selection(&self) -> Result<Option<SavedSelection>> {
        let saved = self.read_saved_selection()?;
        if let Some(saved) = &saved {
            self.selection(&saved.firmware).map_err(|_| {
                format!(
                    "saved firmware {} at {} is no longer registered;\n    \
                     run `polyfw select [FIRMWARE]` to replace it \
                     or `polyfw select --clear` to reset selection",
                    saved.firmware,
                    self.selection_path().display()
                )
            })?;
        }
        Ok(saved)
    }

    pub(crate) fn read_saved_selection(&self) -> Result<Option<SavedSelection>> {
        let path = self.selection_path();
        let value = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", path.display()).into()),
        };
        let saved = toml::from_str::<SavedSelection>(&value).map_err(|error| {
            format!("invalid saved selection at {}: {error}; run polyfw select FIRMWARE or polyfw select --clear", path.display())
        })?;
        Ok(Some(saved))
    }

    pub(crate) fn selection_path(&self) -> PathBuf {
        self.root.join(SELECTION_FILE)
    }

    pub(crate) fn save_selection(&self, id: &str, profile: Option<&Profile>) -> Result<()> {
        let mut selection = self.selection(id)?;
        selection.profile = profile.cloned();
        let value = toml::to_string(&SavedSelection {
            firmware: id.to_owned(),
            profile: profile.cloned(),
        })?;
        let _lock = self.lock_selection()?;
        let path = self.selection_path();
        if !self.registry.config.vscode.enabled {
            return write_file(&path, value.as_bytes());
        }
        let launch = vscode::configuration(&selection)?;
        let launch_path = self.root.join(vscode::LAUNCH_FILE);
        let previous_launch = match fs::read(&launch_path) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("{}: {error}", launch_path.display()).into()),
        };
        write_file(&launch_path, launch.as_bytes())?;
        if let Err(error) = write_file(&path, value.as_bytes()) {
            let rollback = match previous_launch {
                Some(previous) => write_file(&launch_path, &previous),
                None => fs::remove_file(&launch_path).map_err(Into::into),
            };
            rollback.map_err(|rollback| {
                format!(
                    "{error}; restoring {} also failed: {rollback}",
                    launch_path.display()
                )
            })?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn clear_selection(&self) -> Result<Vec<(&'static str, std::io::Result<()>)>> {
        let mut files = Vec::with_capacity(2);
        if self.registry.config.vscode.enabled {
            files.push(vscode::LAUNCH_FILE);
        }
        files.push(SELECTION_FILE);

        let missing = |path| {
            fs::symlink_metadata(self.root.join(path))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        };
        // Check the state directory last. Every writer creates it before modifying files.
        if files.iter().all(|&path| missing(path)) && missing(STATE_DIRECTORY) {
            return Ok(files
                .into_iter()
                .map(|file| (file, Err(std::io::ErrorKind::NotFound.into())))
                .collect());
        }

        let _lock = self.lock_selection()?;
        Ok(files
            .into_iter()
            .map(|file| (file, fs::remove_file(self.root.join(file))))
            .collect())
    }

    fn lock_selection(&self) -> Result<fs::File> {
        let directory = self.root.join(STATE_DIRECTORY);
        let path = directory.join("selection.lock");
        let lock = (|| -> std::io::Result<_> {
            fs::create_dir_all(&directory)?;
            let file = fs::File::options()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?;
            file.lock()?;
            Ok(file)
        })();
        lock.map_err(|error| {
            format!("could not lock selection at {}: {error}", path.display()).into()
        })
    }
}

fn write_file(path: &Path, value: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("generated file path has no parent")?)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let cleanup = DropGuard::new(&temporary, |path| {
        let _ = fs::remove_file(path);
    });
    let result = fs::write(&temporary, value).and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = result {
        return Err(format!("could not write {}: {error}", path.display()).into());
    }
    DropGuard::dismiss(cleanup);
    Ok(())
}

impl Registry {
    fn validate_structure(&self, root: &Path) -> Result<()> {
        config_version::check(
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
            for (field, value) in [
                ("target", &platform.target),
                ("cpu", &platform.cpu),
                ("chip", &platform.chip),
            ] {
                if value.trim().is_empty() {
                    return Err(format!("platform {id} has empty {field}").into());
                }
            }
            if !platform
                .cpu
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte))
            {
                return Err(format!("platform {id} has invalid CPU name: {}", platform.cpu).into());
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

    fn resolve_cargo(&mut self, root: &Path) -> Result<()> {
        let mut metadata = BTreeMap::new();
        for (id, firmware) in &mut self.firmwares {
            let platform = &self.platforms[&firmware.platform];
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
            let key = (cargo.clone(), platform.target.clone(), platform.cpu.clone());
            if !metadata.contains_key(&key) {
                metadata.insert(
                    key.clone(),
                    cargo_metadata_for(&cargo, Some(platform), false)?,
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
                .map(|target| target.name.as_str())
                .collect::<Vec<_>>();
            firmware.resolved_bin = match firmware.bin.as_deref() {
                Some(bin) if bins.contains(&bin) => bin.to_owned(),
                Some(bin) => return Err(format!("firmware {id} package {} has no binary target named {bin}", firmware.package).into()),
                None => match bins.as_slice() {
                    [bin] => (*bin).to_owned(),
                    [] => return Err(format!("firmware {id} package {} has no binary targets", firmware.package).into()),
                    _ => return Err(format!("firmware {id} package {} has multiple binary targets: {}; set bin in {REGISTRY_FILE}", firmware.package, bins.join(", ")).into()),
                },
            };
            cargo.workspace = canonical(Path::new(&data.workspace_root))?;
            cargo.target_dir = data.target_directory.clone();
            cargo.build_dir = data
                .build_directory
                .clone()
                .or_else(|| cargo.build_dir_override.clone());
            firmware.cargo = cargo;
        }
        Ok(())
    }
}

fn validate_directories(id: &str, values: [&Option<String>; 3]) -> Result<()> {
    for value in values.into_iter().flatten() {
        if value.trim().is_empty() || value.contains('\0') {
            return Err(format!("{id} has an empty or invalid directory value").into());
        }
    }
    Ok(())
}

fn absolute_directory(root: &Path, value: &str) -> PathBuf {
    let path = root.join(value);
    // Preserve native filesystem semantics for parent components following symlinks.
    path.canonicalize().unwrap_or(path)
}

pub(crate) fn cargo_metadata_for(
    context: &CargoContext,
    platform: Option<&Platform>,
    dependencies: bool,
) -> Result<Metadata> {
    let mut command = match platform {
        Some(platform) => runner::target_cargo(context, platform),
        None => runner::cargo_command(context),
    };
    command.args(["metadata", "--locked", "--format-version", "1"]);
    if let Some(platform) = platform {
        // metadata has no --target switch; align its native target default with builds.
        command.env("CARGO_BUILD_TARGET", &platform.target);
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

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.ends_with('-')
        && !name.contains("--")
}

pub(crate) fn same_context(left: &Selection, right: &Selection) -> bool {
    left.firmware.cargo == right.firmware.cargo
        && left.platform.target == right.platform.target
        && left.platform.cpu == right.platform.cpu
        && left
            .firmware
            .check_platform
            .or(left.platform.check_platform)
            .unwrap_or(true)
            == right
                .firmware
                .check_platform
                .or(right.platform.check_platform)
                .unwrap_or(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Firmware, Platform};

    #[test]
    fn the_nearest_configuration_defines_the_project_root() {
        let root = std::env::temp_dir().join(format!("polyfw-discovery-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let nested = root.join("nested");
        let working = nested.join("source/deep");
        fs::create_dir_all(&working).unwrap();
        for directory in [&root, &nested] {
            fs::write(directory.join(REGISTRY_FILE), "invalid registry").unwrap();
        }
        let discovered = project_root(None, &working).unwrap();
        assert_eq!(discovered, nested.canonicalize().unwrap());
        let error = Context::load(discovered).err().unwrap().to_string();
        assert!(
            error.contains(&nested.join(REGISTRY_FILE).display().to_string()),
            "{error}"
        );
        fs::remove_file(nested.join(REGISTRY_FILE)).unwrap();
        fs::create_dir(nested.join(REGISTRY_FILE)).unwrap();
        let discovered = project_root(None, &working).unwrap();
        assert_eq!(discovered, nested.canonicalize().unwrap());
        assert!(Context::load(discovered).is_err());
        fs::remove_dir_all(nested.join(REGISTRY_FILE)).unwrap();
        assert_eq!(
            project_root(None, &working).unwrap(),
            root.canonicalize().unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_explicit_root_is_exact_and_invalid_paths_never_fall_back() {
        let root = std::env::temp_dir().join(format!("polyfw-explicit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("caller")).unwrap();
        fs::create_dir_all(root.join("project")).unwrap();
        fs::write(root.join(REGISTRY_FILE), "").unwrap();
        for explicit in [PathBuf::from("../project"), root.join("project")] {
            assert_eq!(
                project_root(Some(&explicit), &root.join("caller")).unwrap(),
                root.join("project").canonicalize().unwrap()
            );
        }
        for explicit in [root.join("missing"), root.join(REGISTRY_FILE)] {
            assert!(project_root(Some(&explicit), &root.join("caller")).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_configuration_reports_an_error_instead_of_using_the_tools_source() {
        let root = std::env::temp_dir().join(format!("polyfw-fallback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("caller")).unwrap();
        let error = project_root(None, &root.join("caller")).unwrap_err();
        assert!(
            error.to_string().contains("no polyfw.toml found"),
            "{error}"
        );
        assert!(error.to_string().contains("--project-root"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loading_the_registry_rejects_a_registered_firmware_without_a_library() {
        let root = std::env::temp_dir().join(format!("polyfw-no-lib-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("platform/test");
        configure(&workspace);
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"test-firmware\"\nversion = \"0.1.0\"\n[workspace]\n",
        )
        .unwrap();
        fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(
            root.join(REGISTRY_FILE),
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n",
        )
        .unwrap();

        let result = Context::load(root.clone());
        fs::remove_dir_all(root).unwrap();
        let error = result
            .err()
            .expect("a binary-only firmware must be rejected");
        assert!(
            error
                .to_string()
                .contains("test-firmware has no library target"),
            "{error}"
        );
    }

    #[test]
    fn firmware_binaries_use_the_only_target_or_an_explicit_name() {
        let root = std::env::temp_dir().join(format!("polyfw-bin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("platform/test");
        configure(&workspace);
        fs::create_dir_all(workspace.join("src/bin")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"test-firmware\"\nversion = \"0.1.0\"\n[workspace]\n",
        )
        .unwrap();
        fs::write(workspace.join("src/lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
        fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
        let source = "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n";
        fs::write(root.join(REGISTRY_FILE), source).unwrap();
        let context = Context::load(root.clone()).unwrap();
        assert_eq!(
            context
                .selection("test-firmware")
                .unwrap()
                .firmware
                .resolved_bin,
            "test-firmware"
        );
        assert!(context.registry.firmwares["test-firmware"].bin.is_none());

        fs::write(workspace.join("src/bin/robot-image.rs"), "fn main() {}\n").unwrap();
        fs::write(
            root.join(REGISTRY_FILE),
            format!("{source}bin = \"robot-image\"\n"),
        )
        .unwrap();
        let context = Context::load(root.clone()).unwrap();
        assert_eq!(
            context
                .selection("test-firmware")
                .unwrap()
                .firmware
                .resolved_bin,
            "robot-image"
        );

        for bin in ["missing-image", "../robot-image", ""] {
            fs::write(root.join(REGISTRY_FILE), format!("{source}bin = {bin:?}\n")).unwrap();
            let error = Context::load(root.clone()).err().unwrap();
            assert!(
                error.to_string().contains("has no binary target named"),
                "{error}"
            );
        }

        fs::write(root.join(REGISTRY_FILE), source).unwrap();
        fs::remove_file(workspace.join("src/main.rs")).unwrap();
        let context = Context::load(root.clone()).unwrap();
        let selection = context.selection("test-firmware").unwrap();
        assert!(selection.firmware.bin.is_none());
        assert_eq!(selection.firmware.resolved_bin, "robot-image");
        let elf = crate::runner::firmware_elf_path(&selection);
        assert_eq!(
            elf,
            root.join("target/firmware/test/thumbv7em-none-eabihf/debug/robot-image")
        );
        context.save_selection("test-firmware", None).unwrap();
        let launch: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join(vscode::LAUNCH_FILE)).unwrap())
                .unwrap();
        for configuration in launch["configurations"].as_array().unwrap() {
            assert_eq!(
                configuration["coreConfigs"][0]["programBinary"],
                elf.to_string_lossy().replace('\\', "/")
            );
        }

        fs::remove_file(workspace.join("src/bin/robot-image.rs")).unwrap();
        let error = Context::load(root.clone()).err().unwrap();
        assert!(
            error.to_string().contains("has no binary targets"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multiple_binaries_require_an_explicit_bin_even_when_one_matches_the_package() {
        let root =
            std::env::temp_dir().join(format!("polyfw-ambiguous-bin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("platform/test");
        configure(&workspace);
        fs::create_dir_all(workspace.join("src/bin")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"test-firmware\"\nversion = \"0.1.0\"\n[workspace]\n",
        )
        .unwrap();
        fs::write(workspace.join("src/lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
        fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(workspace.join("src/bin/robot-image.rs"), "fn main() {}\n").unwrap();
        let source = "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n";
        fs::write(root.join(REGISTRY_FILE), source).unwrap();
        let error = Context::load(root.clone())
            .err()
            .expect("multiple binaries must require an explicit bin")
            .to_string();
        for expected in [
            "test-firmware",
            "robot-image",
            "multiple binary targets",
            "set bin",
        ] {
            assert!(error.contains(expected), "{error}");
        }

        fs::write(
            root.join(REGISTRY_FILE),
            format!("{source}bin = \"test-firmware\"\n"),
        )
        .unwrap();
        Context::load(root.clone()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_named_binaries_are_allowed_to_use_cargos_native_output_handling() {
        let root =
            std::env::temp_dir().join(format!("polyfw-bin-collision-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("platform/test");
        configure(&workspace);
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"firmware-a\", \"firmware-b\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        for name in ["firmware-a", "firmware-b"] {
            let directory = workspace.join(name);
            fs::create_dir_all(directory.join("src")).unwrap();
            fs::write(
                directory.join("Cargo.toml"),
                format!(
                    "[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                     [[bin]]\nname = \"shared-image\"\npath = \"src/main.rs\"\ntest = false\n"
                ),
            )
            .unwrap();
            fs::write(directory.join("src/lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
            fs::write(
                directory.join("src/main.rs"),
                "#![forbid(unsafe_code)]\nfn main() {}\n",
            )
            .unwrap();
        }
        let source = "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.first-family]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/first-family\"\nworking-dir = \"platform/test\"\n\
             chip = \"test\"\n\
             [platforms.second-family]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/second-family\"\nworking-dir = \"platform/test\"\n\
             chip = \"test\"\n\
             [firmwares.one]\ndescription = \"first firmware\"\nplatform = \"first-family\"\n\
             package = \"firmware-a\"\nbin = \"shared-image\"\n\
             [firmwares.alias]\ndescription = \"same image\"\nplatform = \"first-family\"\n\
             package = \"firmware-a\"\nbin = \"shared-image\"\n\
             [firmwares.two]\ndescription = \"other package\"\nplatform = \"second-family\"\n\
             package = \"firmware-b\"\nbin = \"shared-image\"\n";
        for source in [
            source.to_owned(),
            source.replace("bin = \"shared-image\"\n", ""),
        ] {
            fs::write(root.join(REGISTRY_FILE), &source).unwrap();
            let context = Context::load(root.clone()).unwrap();
            assert_eq!(context.registry.firmwares.len(), 3);
            for firmware in context.registry.firmwares.keys() {
                assert_eq!(
                    context.selection(firmware).unwrap().firmware.resolved_bin,
                    "shared-image"
                );
            }

            fs::write(
                root.join(REGISTRY_FILE),
                source.replace(
                    "platform = \"second-family\"",
                    "platform = \"first-family\"",
                ),
            )
            .unwrap();
            assert!(Context::load(root.clone()).is_ok());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_firmware_can_move_within_its_platform_but_cannot_use_another_workspace() {
        let root = std::env::temp_dir().join(format!("polyfw-package-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("platform/test");
        configure(&workspace);
        fs::create_dir_all(workspace.join("firmware")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"firmware\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::write(
            workspace.join("firmware/Cargo.toml"),
            "[package]\nname = \"test-firmware\"\nversion = \"0.1.0\"\n[lib]\npath = \"lib.rs\"\n",
        )
        .unwrap();
        fs::write(
            workspace.join("firmware/lib.rs"),
            "#![forbid(unsafe_code)]\n",
        )
        .unwrap();
        fs::create_dir_all(workspace.join("firmware/src")).unwrap();
        fs::write(workspace.join("firmware/src/main.rs"), "fn main() {}\n").unwrap();

        let mut registry: Registry = toml::from_str(
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n",
        )
        .unwrap();
        registry.resolve_cargo(&root).unwrap();

        fs::rename(workspace.join("firmware"), workspace.join("relocated")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"relocated\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        registry.resolve_cargo(&root).unwrap();

        let other = root.join("platform/other");
        fs::create_dir_all(&other).unwrap();
        fs::write(
            other.join("Cargo.toml"),
            "[package]\nname = \"other-firmware\"\nversion = \"0.1.0\"\n\
             [workspace]\n[lib]\npath = \"lib.rs\"\n",
        )
        .unwrap();
        fs::write(other.join("lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
        let mut platform = registry.platforms["test"].clone();
        platform.working_dir = Some("platform/other".into());
        registry.platforms.insert("other".into(), platform);
        registry
            .firmwares
            .get_mut("test-firmware")
            .unwrap()
            .platform = "other".into();
        let error = registry.resolve_cargo(&root).unwrap_err().to_string();
        assert!(
            error.contains("package is not a member of the Cargo workspace: test-firmware"),
            "{error}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_names_are_lowercase_slug_names() {
        assert!(valid_name("sentry-chassis"));
        assert!(valid_name("bringup-dm-mc02"));
        assert!(!valid_name("Sentry"));
        assert!(!valid_name("sentry--chassis"));
        assert!(!valid_name("sentry-"));
    }

    #[test]
    fn compiler_overrides_are_rejected_on_firmware_entries() {
        for setting in ["target = \"thumbv7em-none-eabihf\"", "cpu = \"cortex-m7\""] {
            let source = format!(
                "{}[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\nchip = \"test\"\nworking-dir = \".\"\n[firmwares.demo]\ndescription = \"Demo\"\nplatform = \"test\"\npackage = \"demo\"\n{setting}\n",
                crate::config_version::TEST_HEADER
            );
            let error = toml::from_str::<Registry>(&source).unwrap_err();
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
    }

    fn configure(workspace: &Path) {
        fs::create_dir_all(workspace.join(".cargo")).unwrap();
        fs::write(
            workspace.join(".cargo/config.toml"),
            "[build]\ntarget = \"test-target\"\n\
             [target.test-target]\nrustflags = \"-Ctarget-cpu=test-cpu\"\n",
        )
        .unwrap();
    }

    #[test]
    fn dependency_patterns_default_to_the_project_and_invalid_patterns_fail_before_cargo() {
        let registry: Registry = toml::from_str(
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {}\nfirmwares = {}\n",
        )
        .unwrap();
        assert_eq!(registry.config.local_dependencies, ["./**"]);
        let registry: Registry =
            toml::from_str("config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {}\nfirmwares = {}\n[config]\nlocal-dependencies = []\n").unwrap();
        assert!(registry.config.local_dependencies.is_empty());
        for value in ["42", "\"shared/**\"", "[42]"] {
            assert!(
                toml::from_str::<Registry>(&format!(
                    "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {{}}\nfirmwares = {{}}\n[config]\nlocal-dependencies = {value}\n"
                ))
                .is_err()
            );
        }
        for pattern in ["[", "shared/**suffix", "*/../shared"] {
            let registry: Registry = toml::from_str(&format!(
                "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {{}}\nfirmwares = {{}}\n[config]\nlocal-dependencies = [{pattern:?}]\n"
            ))
            .unwrap();
            let error = registry
                .validate_structure(Path::new("."))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("invalid local-dependencies pattern"),
                "{error}"
            );
            assert!(error.contains(pattern), "{error}");
        }
    }

    #[test]
    fn tool_configuration_defaults_are_preserved_for_missing_and_partial_tables() {
        for config in [
            "",
            "[config]\n",
            "[config.vscode]\n",
            "[config.vscode]\nenabled = true\n",
        ] {
            let registry: Registry =
                toml::from_str(&format!("config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {{}}\nfirmwares = {{}}\n{config}")).unwrap();
            assert_eq!(registry.config.local_dependencies, ["./**"]);
            assert!(registry.config.vscode.enabled);
        }
        let registry: Registry = toml::from_str(
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {}\nfirmwares = {}\n[config]\nlocal-dependencies = []\n\
             [config.vscode]\nenabled = false\n",
        )
        .unwrap();
        assert!(registry.config.local_dependencies.is_empty());
        assert!(!registry.config.vscode.enabled);
    }

    #[test]
    fn misplaced_unknown_and_wrongly_typed_configuration_options_are_rejected() {
        for config in [
            "local-dependencies = []\n",
            "[config]\nunknown = true\n",
            "[config]\nvscode = false\n",
            "[config.vscode]\nenabled = \"false\"\n",
            "[config.vscode]\nenable = false\n",
        ] {
            assert!(
                toml::from_str::<Registry>(&format!("config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\nplatforms = {{}}\nfirmwares = {{}}\n{config}"))
                    .is_err(),
                "{config}"
            );
        }
    }

    #[test]
    fn descriptions_are_optional_and_preserve_explicit_text() {
        for description in [None, Some(""), Some(" \t"), Some("Demo description")] {
            let field = description
                .map(|value| format!("description = {}\n", toml::Value::String(value.into())))
                .unwrap_or_default();
            let source = format!(
                "{}[platforms.test]\n{field}target = \"host-tuple\"\ncpu = \"native\"\nchip = \"test\"\n\
                 [firmwares.demo]\n{field}platform = \"test\"\npackage = \"demo\"\n",
                crate::config_version::TEST_HEADER
            );
            let registry: Registry = toml::from_str(&source).unwrap();
            registry.validate_structure(Path::new(".")).unwrap();
            assert_eq!(
                registry.platforms["test"].description.as_deref(),
                description
            );
            assert_eq!(
                registry.firmwares["demo"].description.as_deref(),
                description
            );
        }
    }

    #[test]
    fn working_directory_defaults_do_not_accept_explicit_invalid_paths() {
        let source = format!(
            "{}[platforms.test]\ntarget = \"host-tuple\"\ncpu = \"native\"\nchip = \"test\"\n\
             [firmwares.demo]\nplatform = \"test\"\npackage = \"demo\"\n",
            crate::config_version::TEST_HEADER
        );
        for platform_override in [false, true] {
            for value in ["", " \t", "invalid\0path"] {
                let mut registry: Registry = toml::from_str(&source).unwrap();
                if platform_override {
                    registry.platforms.get_mut("test").unwrap().working_dir = Some(value.into());
                    registry.firmwares.get_mut("demo").unwrap().working_dir = Some(".".into());
                } else {
                    registry.firmwares.get_mut("demo").unwrap().working_dir = Some(value.into());
                }
                let error = registry.validate_structure(Path::new(".")).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("empty or invalid directory value"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn the_schema_identifies_the_package_version_and_draft07_dialect() {
        let schema: serde_json::Value = serde_json::from_str(&schema_source().unwrap()).unwrap();
        assert_eq!(
            schema["$id"],
            format!(
                "https://raw.githubusercontent.com/inaku-Gyan/polyfw/v{}/assets/polyfw.schema.json",
                env!("CARGO_PKG_VERSION")
            )
        );
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
        for name in ["config-version", "min-polyfw-version"] {
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|field| field == name)
            );
            assert_eq!(schema["properties"][name]["type"], "string");
        }
    }

    #[test]
    fn the_registry_schema_matches_the_generated_schema() {
        assert_eq!(
            include_str!("../assets/polyfw.schema.json"),
            schema_source().unwrap(),
            "registry schema is stale; run just schema"
        );
    }

    #[test]
    fn duplicate_platform_work_is_removed_for_all_firmwares() {
        let platform = Platform {
            working_dir: Some("platform/dm-mc02".into()),
            target: "thumbv7em-none-eabihf".into(),
            cpu: "cortex-m7".into(),
            chip: "STM32H723VGTx".into(),
            ..Platform::default()
        };
        let firmware = |id: &str| Selection {
            id: id.into(),
            profile: None,
            firmware: Firmware {
                description: Some(id.into()),
                platform: "dm-mc02".into(),
                package: id.into(),
                bin: None,
                resolved_bin: id.into(),
                ..Firmware::default()
            },
            platform: platform.clone(),
        };
        assert_eq!(
            unique_contexts(&[firmware("one"), firmware("two")]).len(),
            1
        );
    }

    #[test]
    fn saving_a_valid_firmware_writes_the_selection_file_and_rejects_unknown_firmwares() {
        let root = std::env::temp_dir().join(format!("polyfw-selection-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let context = Context {
            root: root.clone(),
            registry: Registry {
                config_version: crate::config_version::CLI_VERSION.into(),
                min_polyfw_version: crate::config_version::CLI_VERSION.into(),
                config: crate::model::Config::default(),
                platforms: BTreeMap::from([(
                    "test-platform".into(),
                    Platform {
                        working_dir: Some("platform/test".into()),
                        target: "thumbv7em-none-eabihf".into(),
                        cpu: "cortex-m7".into(),
                        chip: "test-chip".into(),
                        ..Platform::default()
                    },
                )]),
                firmwares: BTreeMap::from([(
                    "test-firmware".into(),
                    Firmware {
                        description: Some("test firmware".into()),
                        platform: "test-platform".into(),
                        package: "test-firmware".into(),
                        bin: None,
                        resolved_bin: "test-firmware".into(),
                        cargo: crate::model::CargoContext::fixture(&root, "test-platform"),
                        ..Firmware::default()
                    },
                )]),
            },
        };

        let profile = "release".parse().unwrap();
        context
            .save_selection("test-firmware", Some(&profile))
            .unwrap();
        let saved = context.read_selection().unwrap().unwrap();
        assert_eq!(saved.firmware, "test-firmware");
        assert_eq!(saved.profile.as_ref(), Some(&profile));
        assert!(
            context
                .save_selection("missing-firmware", Some(&profile))
                .is_err()
        );
        assert_eq!(
            context.read_selection().unwrap().unwrap().firmware,
            "test-firmware"
        );

        fs::write(context.selection_path(), "test-firmware\n").unwrap();
        assert!(context.read_selection().is_err());
        fs::write(context.selection_path(), "firmware = \"test-firmware\"\n").unwrap();
        assert!(context.read_selection().unwrap().unwrap().profile.is_none());
        fs::write(
            context.selection_path(),
            "firmware = \"test-firmware\"\nprofile = \"dev\"\n",
        )
        .unwrap();
        assert_eq!(
            context
                .read_selection()
                .unwrap()
                .unwrap()
                .profile
                .as_ref()
                .map(Profile::name),
            Some("dev")
        );
        context.save_selection("test-firmware", None).unwrap();
        assert_eq!(
            fs::read_to_string(context.selection_path()).unwrap(),
            "firmware = \"test-firmware\"\n"
        );
        assert!(context.read_selection().unwrap().unwrap().profile.is_none());
        fs::write(
            context.selection_path(),
            "firmware = \"test-firmware\"\nprofile = \"../release\"\n",
        )
        .unwrap();
        assert!(context.read_selection().is_err());
        assert!(
            context
                .clear_selection()
                .unwrap()
                .iter()
                .all(|(_, result)| result.is_ok())
        );
        assert!(
            context
                .clear_selection()
                .unwrap()
                .iter()
                .all(|(_, result)| {
                    result.as_ref().unwrap_err().kind() == std::io::ErrorKind::NotFound
                })
        );
        assert!(context.read_selection().unwrap().is_none());

        fs::remove_dir_all(root).unwrap();
    }
}
