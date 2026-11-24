use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cargo::{
    self,
    model::{FirmwareCargo, Selection},
};
use crate::config::{
    REGISTRY_FILE,
    model::{Firmware, Registry},
};
use crate::{Error, ErrorKind, Result};

use super::FirmwareSummary;

pub(super) fn discover_root(from_dir: &Path) -> Result<PathBuf> {
    let working_directory = exact_root(from_dir)?;
    for directory in working_directory.ancestors() {
        let registry = directory.join(REGISTRY_FILE);
        match fs::symlink_metadata(&registry) {
            Ok(_) => return exact_root(directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::context(
                    ErrorKind::ProjectRoot,
                    format!("could not inspect {}", registry.display()),
                    error,
                ));
            }
        }
    }
    Err(Error::new(
        ErrorKind::ProjectRoot,
        format!(
            "no {REGISTRY_FILE} found in {} or its ancestors",
            working_directory.display()
        ),
    ))
}

pub(super) fn exact_root(path: &Path) -> Result<PathBuf> {
    let root = path.canonicalize().map_err(|error| {
        Error::context(
            ErrorKind::ProjectRoot,
            format!("invalid project root {}", path.display()),
            error,
        )
    })?;
    if !root.is_dir() {
        return Err(Error::new(
            ErrorKind::ProjectRoot,
            format!("project root is not a directory: {}", root.display()),
        ));
    }
    Ok(root)
}

#[derive(Clone, Debug)]
pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) registry: Registry,
    pub(crate) cargo: BTreeMap<String, FirmwareCargo>,
}

impl Context {
    #[cfg(test)]
    pub(crate) fn load(root: PathBuf) -> Result<Self> {
        Self::load_config(root)?.resolved()
    }

    pub(crate) fn load_config(root: PathBuf) -> Result<Self> {
        let registry = Registry::load(&root)?;
        Ok(Self {
            root,
            registry,
            cargo: BTreeMap::new(),
        })
    }

    pub(crate) fn resolved(&self) -> Result<Self> {
        let mut context = self.clone();
        context.resolve_cargo()?;
        Ok(context)
    }

    pub(crate) fn resolve_cargo(&mut self) -> Result<()> {
        self.cargo = cargo::context::resolve(&self.root, &self.registry)
            .map_err(|error| Error::new(ErrorKind::Cargo, error))?;
        Ok(())
    }

    pub(crate) fn firmware(&self, id: &str) -> Option<FirmwareSummary<'_>> {
        let (id, firmware) = self.registry.firmwares.get_key_value(id)?;
        Some(self.firmware_summary(id, firmware))
    }

    fn firmware_summary<'a>(&'a self, id: &'a str, firmware: &'a Firmware) -> FirmwareSummary<'a> {
        FirmwareSummary {
            id,
            description: firmware.description.as_deref(),
            platform: &firmware.platform,
            package: &firmware.package,
            chip: &self.registry.platforms[&firmware.platform].chip,
        }
    }

    pub(crate) fn firmwares(&self) -> impl ExactSizeIterator<Item = FirmwareSummary<'_>> {
        self.registry
            .firmwares
            .iter()
            .map(|(id, firmware)| self.firmware_summary(id, firmware))
    }

    pub(crate) fn require_firmware(&self, id: &str) -> Result<FirmwareSummary<'_>> {
        self.firmware(id).ok_or_else(|| {
            Error::new(
                ErrorKind::UnknownFirmware,
                format!("unknown firmware: {id}"),
            )
        })
    }

    pub(crate) fn selection(&self, id: &str) -> Result<Selection> {
        let info = self.require_firmware(id)?;
        let resolved = self.cargo.get(id).ok_or_else(|| {
            Error::new(
                ErrorKind::Cargo,
                format!("Cargo context was not resolved for firmware {id}"),
            )
        })?;
        Ok(Selection {
            id: id.to_owned(),
            firmware: self.registry.firmwares[id].clone(),
            platform: self.registry.platforms[info.platform].clone(),
            profile: None,
            cargo: resolved.cargo.clone(),
            resolved_bin: resolved.bin.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::vscode;
    use super::*;
    use crate::Profile;
    use crate::cargo::context::unique_contexts;
    use crate::cargo::model::{CargoContext, ResolvedBin};
    use crate::config::model::{Config, Firmware, Platform};
    use crate::config::version::CLI_VERSION;
    use crate::configuration_schema as schema_source;

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
        let discovered = discover_root(&working).unwrap();
        assert_eq!(discovered, nested.canonicalize().unwrap());
        let error = Context::load(discovered).err().unwrap().to_string();
        assert!(
            error.contains(&nested.join(REGISTRY_FILE).display().to_string()),
            "{error}"
        );
        fs::remove_file(nested.join(REGISTRY_FILE)).unwrap();
        fs::create_dir(nested.join(REGISTRY_FILE)).unwrap();
        let discovered = discover_root(&working).unwrap();
        assert_eq!(discovered, nested.canonicalize().unwrap());
        assert!(Context::load(discovered).is_err());
        fs::remove_dir_all(nested.join(REGISTRY_FILE)).unwrap();
        assert_eq!(
            discover_root(&working).unwrap(),
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
                exact_root(&root.join("caller").join(explicit)).unwrap(),
                root.join("project").canonicalize().unwrap()
            );
        }
        for explicit in [root.join("missing"), root.join(REGISTRY_FILE)] {
            assert!(exact_root(&root.join("caller").join(explicit)).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_configuration_reports_an_error_instead_of_using_the_tools_source() {
        let root = std::env::temp_dir().join(format!("polyfw-fallback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("caller")).unwrap();
        let error = discover_root(&root.join("caller")).unwrap_err();
        assert!(
            error.to_string().contains("no polyfw.toml found"),
            "{error}"
        );
        assert_eq!(error.kind(), ErrorKind::ProjectRoot);
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
            "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n",
        )
        .unwrap();

        let result = Context::load(root.clone());
        fs::remove_dir_all(root).unwrap();
        let error = result.expect_err("a binary-only firmware must be rejected");
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
        let source = "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n";
        fs::write(root.join(REGISTRY_FILE), source).unwrap();
        let context = Context::load(root.clone()).unwrap();
        assert_eq!(
            context
                .selection("test-firmware")
                .unwrap()
                .resolved_bin
                .name,
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
                .resolved_bin
                .name,
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
        assert_eq!(selection.resolved_bin.name, "robot-image");
        let elf = crate::cargo::runner::firmware_elf_path(&selection, None);
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
        let source = "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n";
        fs::write(root.join(REGISTRY_FILE), source).unwrap();
        let error = Context::load(root.clone())
            .expect_err("multiple binaries must require an explicit bin")
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
        let source = "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n[platforms.first-family]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/first-family\"\nworking-dir = \"platform/test\"\n\
             chip = \"test\"\n\
             [platforms.second-family]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/second-family\"\nworking-dir = \"platform/test\"\n\
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
                    context.selection(firmware).unwrap().resolved_bin.name,
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
            "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/test\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
             [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test\"\n\
             package = \"test-firmware\"\n",
        )
        .unwrap();
        cargo::context::resolve(&root, &registry).unwrap();

        fs::rename(workspace.join("firmware"), workspace.join("relocated")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"relocated\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        cargo::context::resolve(&root, &registry).unwrap();

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
        let error = cargo::context::resolve(&root, &registry)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("package is not a member of the Cargo workspace: test-firmware"),
            "{error}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compiler_overrides_are_rejected_on_firmware_entries() {
        for setting in ["target = \"thumbv7em-none-eabihf\"", "cpu = \"cortex-m7\""] {
            let source = format!(
                "{}[platforms.test]\ntarget = \"thumbv7em-none-eabihf\"\nchip = \"test\"\nworking-dir = \".\"\n[firmwares.demo]\ndescription = \"Demo\"\nplatform = \"test\"\npackage = \"demo\"\n{setting}\n",
                crate::config::version::TEST_HEADER
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
            "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {}\nfirmwares = {}\n",
        )
        .unwrap();
        assert_eq!(registry.config.local_dependencies, ["./**"]);
        let registry: Registry =
            toml::from_str("config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {}\nfirmwares = {}\n[config]\nlocal-dependencies = []\n").unwrap();
        assert!(registry.config.local_dependencies.is_empty());
        for value in ["42", "\"shared/**\"", "[42]"] {
            assert!(
                toml::from_str::<Registry>(&format!(
                    "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {{}}\nfirmwares = {{}}\n[config]\nlocal-dependencies = {value}\n"
                ))
                .is_err()
            );
        }
        for pattern in ["[", "shared/**suffix", "*/../shared"] {
            let registry: Registry = toml::from_str(&format!(
                "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {{}}\nfirmwares = {{}}\n[config]\nlocal-dependencies = [{pattern:?}]\n"
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
                toml::from_str(&format!("config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {{}}\nfirmwares = {{}}\n{config}")).unwrap();
            assert_eq!(registry.config.local_dependencies, ["./**"]);
            assert!(registry.config.vscode.enabled);
        }
        let registry: Registry = toml::from_str(
            "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {}\nfirmwares = {}\n[config]\nlocal-dependencies = []\n\
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
                toml::from_str::<Registry>(&format!("config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\nplatforms = {{}}\nfirmwares = {{}}\n{config}"))
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
                "{}[platforms.test]\n{field}target = \"host-tuple\"\nchip = \"test\"\n\
                 [firmwares.demo]\n{field}platform = \"test\"\npackage = \"demo\"\n",
                crate::config::version::TEST_HEADER
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
            "{}[platforms.test]\ntarget = \"host-tuple\"\nchip = \"test\"\n\
             [firmwares.demo]\nplatform = \"test\"\npackage = \"demo\"\n",
            crate::config::version::TEST_HEADER
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
            include_str!("../../assets/polyfw.schema.json"),
            schema_source().unwrap(),
            "registry schema is stale; run just schema"
        );
    }

    #[test]
    fn duplicate_platform_work_is_removed_for_all_firmwares() {
        let platform = Platform {
            working_dir: Some("platform/dm-mc02".into()),
            target: "thumbv7em-none-eabihf".into(),
            chip: "STM32H723VGTx".into(),
            ..Platform::default()
        };
        let firmware = |id: &str| Selection {
            id: id.into(),
            profile: None,
            cargo: CargoContext::default(),
            resolved_bin: ResolvedBin {
                name: id.into(),
                ..ResolvedBin::default()
            },
            firmware: Firmware {
                description: Some(id.into()),
                platform: "dm-mc02".into(),
                package: id.into(),
                bin: None,
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
            cargo: BTreeMap::from([(
                "test-firmware".into(),
                FirmwareCargo {
                    bin: ResolvedBin {
                        name: "test-firmware".into(),
                        ..ResolvedBin::default()
                    },
                    cargo: CargoContext::fixture(&root, "test-platform"),
                    manifest_path: root.join("platform/test/firmware/Cargo.toml"),
                },
            )]),
            registry: Registry {
                config_version: CLI_VERSION.into(),
                min_polyfw_version: CLI_VERSION.into(),
                config: Config::default(),
                platforms: BTreeMap::from([(
                    "test-platform".into(),
                    Platform {
                        working_dir: Some("platform/test".into()),
                        target: "thumbv7em-none-eabihf".into(),
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
                .files
                .iter()
                .all(|file| file.result.is_ok())
        );
        assert!(context.clear_selection().unwrap().files.iter().all(|file| {
            file.result.as_ref().unwrap_err().kind() == std::io::ErrorKind::NotFound
        }));
        assert!(context.read_selection().unwrap().is_none());

        fs::remove_dir_all(root).unwrap();
    }
}
