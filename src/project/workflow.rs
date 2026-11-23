use crate::cargo::{model::Selection, workflow as cargo_workflow};
use crate::{Check, Profile, Result, RunReport, process};

use super::{Artifact, BuildReport, Context, FirmwareTarget, Options, Project};

impl Project {
    /// Builds the requested firmware and reports its expected artifacts.
    pub fn build(
        &self,
        target: FirmwareTarget<'_>,
        profile: Option<&Profile>,
        mut options: Options<'_>,
    ) -> Result<BuildReport> {
        let context = self.context.resolved()?;
        let selections = context.targets(target, profile, &mut options)?;
        let execution = process::run(
            cargo_workflow::build(&selections)?,
            options.output,
            options.progress,
        )?;
        let artifacts = selections.iter().map(Artifact::from_selection).collect();
        Ok(BuildReport {
            artifacts,
            execution,
        })
    }

    /// Cleans an individual firmware package or all registered output directory contexts.
    pub fn clean(&self, target: FirmwareTarget<'_>, mut options: Options<'_>) -> Result<RunReport> {
        let context = self.context.resolved()?;
        let selections = context.targets(target, None, &mut options)?;
        let stages = cargo_workflow::clean(
            &context.registry,
            &context.cargo,
            &selections,
            matches!(target, FirmwareTarget::All),
        );
        process::run(stages, options.output, options.progress)
    }

    /// Runs a formatting, lint, test or verification workflow over the requested scope.
    pub fn check(
        &self,
        check: Check,
        target: FirmwareTarget<'_>,
        mut options: Options<'_>,
    ) -> Result<RunReport> {
        let context = self.context.resolved()?;
        let selections = context.targets(target, None, &mut options)?;
        let stages = cargo_workflow::check(
            &context.root,
            &context.registry.config.local_dependencies,
            &selections,
            check,
        )?;
        process::run(stages, options.output, options.progress)
    }
}

impl Context {
    fn targets(
        &self,
        target: FirmwareTarget<'_>,
        profile: Option<&Profile>,
        options: &mut Options<'_>,
    ) -> Result<Vec<Selection>> {
        match target {
            FirmwareTarget::One(choice) => {
                let (id, profile) = self.choose_firmware(choice, profile, options)?;
                let mut selection = self.selection(&id)?;
                selection.profile = profile;
                Ok(vec![selection])
            }
            FirmwareTarget::All => self
                .registry
                .firmwares
                .keys()
                .map(|id| {
                    let mut selection = self.selection(id)?;
                    selection.profile = profile.cloned();
                    Ok(selection)
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::cargo::model::{CargoContext, FirmwareCargo};
    use crate::cargo::runner;
    use crate::cargo::scope::{self, PackageScope, WorkspaceScope};
    use crate::cargo::workflow::{
        build_command, clean_command, fmt_stages, lint_stages, test_stages,
    };
    use crate::config::model::{Config, Firmware, Platform, Registry};
    use crate::config::version::CLI_VERSION;
    use crate::process::Invocation;
    use crate::project::vscode;
    use crate::{FirmwareChoice, OutputMode};

    fn scopes(context: &Context, selections: &[Selection]) -> Vec<WorkspaceScope> {
        scope::resolve(
            &context.root,
            &context.registry.config.local_dependencies,
            selections,
        )
        .unwrap()
    }

    fn build(selections: &[Selection]) {
        process::run(
            cargo_workflow::build(selections).unwrap(),
            OutputMode::Inherit,
            None,
        )
        .unwrap();
    }

    #[test]
    fn saving_a_selection_generates_matching_launch_and_attach_without_building() {
        let mut context = test_context(test_root("vscode profiles with spaces"));
        let firmware = context.registry.firmwares.get_mut("test-firmware").unwrap();
        firmware.bin = Some("robot-image".into());
        context.cargo.get_mut("test-firmware").unwrap().bin = "robot-image".into();
        let mut other_platform = context.registry.platforms["test-platform"].clone();
        other_platform.chip = "another-chip".into();
        context
            .registry
            .platforms
            .insert("other-platform".into(), other_platform);
        let mut other_firmware = context.registry.firmwares["test-firmware"].clone();
        other_firmware.platform = "other-platform".into();
        context
            .registry
            .firmwares
            .insert("other-firmware".into(), other_firmware);
        let other_cargo = context.cargo["test-firmware"].clone();
        context.cargo.insert("other-firmware".into(), other_cargo);
        for firmware in ["test-firmware", "other-firmware"] {
            for (profile, directory, suffix) in [
                (None, "debug", ""),
                (Some("dev"), "debug", " (dev)"),
                (Some("release"), "release", " (release)"),
                (Some("bench-debug"), "bench-debug", " (bench-debug)"),
            ] {
                let profile_value = profile.map(|profile| profile.parse().unwrap());
                context
                    .save_selection(firmware, profile_value.as_ref())
                    .unwrap();
                let state: toml::Value =
                    toml::from_str(&fs::read_to_string(context.selection_path()).unwrap()).unwrap();
                assert_eq!(state.get("profile").and_then(toml::Value::as_str), profile);
                let value: serde_json::Value = serde_json::from_slice(
                    &fs::read(context.root.join(vscode::LAUNCH_FILE)).unwrap(),
                )
                .unwrap();
                let configurations = value["configurations"].as_array().unwrap();
                assert_eq!(configurations.len(), 2);
                let selection = context.selection(firmware).unwrap();
                let expected_elf = selection
                    .cargo
                    .target_dir
                    .join("thumbv7em-none-eabihf")
                    .join(directory)
                    .join("robot-image")
                    .to_string_lossy()
                    .replace('\\', "/");
                for (configuration, request, label) in [
                    (&configurations[0], "launch", "Launch"),
                    (&configurations[1], "attach", "Attach"),
                ] {
                    assert_eq!(configuration["request"], request);
                    assert_eq!(configuration["name"], format!("{label} {firmware}{suffix}"));
                    assert_eq!(configuration["chip"], selection.platform.chip);
                    assert_eq!(
                        configuration["coreConfigs"][0]["programBinary"],
                        expected_elf
                    );
                }
            }
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(context.root.join(vscode::LAUNCH_FILE)).unwrap())
                .unwrap();
        assert_eq!(value["version"], "0.2.0");
        let configurations = value["configurations"].as_array().unwrap();
        for configuration in configurations {
            assert_eq!(configuration["type"], "probe-rs-debug");
            assert_eq!(configuration["cwd"], "${workspaceFolder}");
            assert_eq!(configuration["wireProtocol"], "Swd");
            assert!(configuration.get("probe").is_none());
            assert!(configuration.get("speed").is_none());
            assert!(configuration.get("preLaunchTask").is_none());
            assert!(configuration.get("postDebugTask").is_none());
            let core = &configuration["coreConfigs"][0];
            assert_eq!(core["coreIndex"], 0);
            assert_eq!(core["rttEnabled"], true);
            assert_eq!(core["rttChannelFormats"][0]["channelNumber"], 0);
            assert_eq!(core["rttChannelFormats"][0]["dataFormat"], "Defmt");
            assert_eq!(core["rttChannelFormats"][0]["mode"], "NoBlockSkip");
        }
        let flashing = &configurations[0]["flashingConfig"];
        assert_eq!(flashing["flashingEnabled"], true);
        assert_eq!(flashing["verifyAfterFlashing"], true);
        assert_eq!(flashing["haltAfterReset"], true);
        assert!(configurations[1].get("flashingConfig").is_none());
        assert!(!context.root.join("target").exists());
        assert!(!context.root.join("platform").exists());
        assert!(!context.root.join(".vscode/tasks.json").exists());
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn clearing_selection_removes_launch_but_preserves_other_editor_files_and_handles_orphans() {
        let context = test_context(test_root("clear-vscode"));
        context.save_selection("test-firmware", None).unwrap();
        let editor = context.root.join(".vscode");
        fs::write(editor.join("settings.json"), "{}\n").unwrap();
        fs::write(editor.join("tasks.json"), "{}\n").unwrap();
        assert!(context.clear_selection().unwrap().success());
        assert!(!context.selection_path().exists());
        assert!(!editor.join("launch.json").exists());
        assert_eq!(
            fs::read_to_string(editor.join("settings.json")).unwrap(),
            "{}\n"
        );
        assert_eq!(
            fs::read_to_string(editor.join("tasks.json")).unwrap(),
            "{}\n"
        );
        fs::remove_dir_all(context.selection_path().parent().unwrap()).unwrap();
        fs::write(editor.join("launch.json"), "{}\n").unwrap();
        let removed = context.clear_selection().unwrap();
        assert!(removed.files[0].result.is_ok());
        assert_eq!(
            removed.files[1].result.as_ref().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(!editor.join("launch.json").exists());
        assert!(context.clear_selection().unwrap().success());
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn clearing_selection_attempts_every_file_and_returns_failure_after_an_error() {
        for failed_file in [vscode::LAUNCH_FILE, ".polyfw-state/selected"] {
            let context = test_context(test_root(&format!("clear-failed-{failed_file}")));
            context.save_selection("test-firmware", None).unwrap();
            let failed_path = context.root.join(failed_file);
            fs::remove_file(&failed_path).unwrap();
            fs::create_dir(&failed_path).unwrap();

            assert!(!context.clear_selection().unwrap().success());
            assert!(failed_path.is_dir());
            for file in [vscode::LAUNCH_FILE, ".polyfw-state/selected"] {
                if file != failed_file {
                    assert!(!context.root.join(file).exists());
                }
            }
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    #[test]
    fn selection_changes_leave_both_files_untouched_when_the_lock_cannot_be_opened() {
        let context = test_context(test_root("selection-lock-failure"));
        context.save_selection("test-firmware", None).unwrap();
        let selection = context.selection_path();
        let launch = context.root.join(vscode::LAUNCH_FILE);
        let previous_selection = fs::read(&selection).unwrap();
        let previous_launch = fs::read(&launch).unwrap();
        let lock = context.root.join(".polyfw-state/selection.lock");
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();

        let profile = "release".parse().unwrap();
        let save_error = context
            .save_selection("test-firmware", Some(&profile))
            .unwrap_err();
        let clear_error = context.clear_selection().unwrap_err();
        for error in [save_error, clear_error] {
            assert!(
                error.to_string().contains("could not lock selection"),
                "{error}"
            );
        }
        assert_eq!(fs::read(selection).unwrap(), previous_selection);
        assert_eq!(fs::read(launch).unwrap(), previous_launch);
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn disabling_vscode_leaves_existing_editor_files_untouched_when_saving_or_clearing() {
        let mut context = test_context(test_root("vscode-disabled"));
        context.save_selection("test-firmware", None).unwrap();
        let path = context.root.join(vscode::LAUNCH_FILE);
        let previous = fs::read(&path).unwrap();
        context.registry.config.vscode.enabled = false;
        context
            .save_selection("test-firmware", Some(&"release".parse().unwrap()))
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(
            context
                .read_selection()
                .unwrap()
                .unwrap()
                .profile
                .unwrap()
                .name(),
            "release"
        );
        assert!(context.clear_selection().unwrap().success());
        assert!(!context.selection_path().exists());
        assert_eq!(fs::read(&path).unwrap(), previous);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        context.save_selection("test-firmware", None).unwrap();
        assert_eq!(context.clear_selection().unwrap().files.len(), 1);
        assert!(path.is_dir());
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn firmware_commands_that_save_a_selection_respect_the_vscode_switch() {
        for command in ["clean", "build", "fmt", "lint", "test", "verify", "image"] {
            let mut context = test_context(test_root(&format!("vscode-disabled-{command}")));
            context.registry.config.vscode.enabled = false;
            fs::write(context.root.join(".vscode"), "not a directory").unwrap();
            let root = context.root.clone();
            let target = FirmwareTarget::One(FirmwareChoice::Save(Some("test-firmware")));
            let selections = context
                .targets(target, None, &mut Options::default())
                .unwrap();
            // The operation fails after saving on this fixture.
            let plan = match command {
                "clean" => Ok(cargo_workflow::clean(
                    &context.registry,
                    &context.cargo,
                    &selections,
                    false,
                )),
                "build" => cargo_workflow::build(&selections),
                "fmt" => cargo_workflow::check(&root, &[], &selections, Check::Format),
                "lint" => cargo_workflow::check(&root, &[], &selections, Check::Lint),
                "test" => cargo_workflow::check(&root, &[], &selections, Check::Test),
                "verify" => cargo_workflow::check(&root, &[], &selections, Check::Verify),
                "image" => {
                    assert!(!Artifact::from_selection(&selections[0]).exists);
                    Ok(vec![])
                }
                _ => unreachable!(),
            };
            if command != "image" {
                assert!(
                    plan.and_then(|stages| process::run(stages, OutputMode::Capture, None))
                        .is_err()
                );
            }
            assert_eq!(
                fs::read_to_string(root.join(".polyfw-state/selected")).unwrap(),
                "firmware = \"test-firmware\"\n",
                "{command}"
            );
            assert_eq!(
                fs::read_to_string(root.join(".vscode")).unwrap(),
                "not a directory"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_failed_selection_save_restores_the_previous_launch_or_removes_the_new_one() {
        for existing in [false, true] {
            let context = test_context(test_root(&format!("vscode-rollback-{existing}")));
            let path = context.root.join(vscode::LAUNCH_FILE);
            let previous = if existing {
                context.save_selection("test-firmware", None).unwrap();
                let previous = fs::read(&path).unwrap();
                fs::remove_file(context.selection_path()).unwrap();
                Some(previous)
            } else {
                None
            };
            fs::create_dir_all(context.selection_path()).unwrap();
            assert!(
                context
                    .save_selection("test-firmware", Some(&"release".parse().unwrap()))
                    .is_err()
            );
            if let Some(previous) = previous {
                assert_eq!(fs::read(path).unwrap(), previous);
            } else {
                assert!(!path.exists());
            }
            assert!(context.selection_path().is_dir());
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    #[test]
    fn profiles_are_inherited_but_remain_unspecified_when_saving_or_building_all_firmwares() {
        let mut context = test_context(test_root("profile-selection"));
        let other = context.registry.firmwares["test-firmware"].clone();
        context
            .registry
            .firmwares
            .insert("other-firmware".into(), other);
        let with_saved = |id| FirmwareTarget::One(FirmwareChoice::WithSaved(id));
        let save = FirmwareTarget::One(FirmwareChoice::Save(Some("other-firmware")));
        let other_cargo = context.cargo["test-firmware"].clone();
        context.cargo.insert("other-firmware".into(), other_cargo);
        for (target, profile, expected, saved_firmware, saved_profile) in [
            (
                with_saved(None),
                None,
                Some("release"),
                "test-firmware",
                Some("release"),
            ),
            (
                with_saved(Some("other-firmware")),
                None,
                Some("release"),
                "test-firmware",
                Some("release"),
            ),
            (
                with_saved(Some("other-firmware")),
                Some("dev"),
                Some("dev"),
                "test-firmware",
                Some("release"),
            ),
            (save, None, None, "other-firmware", None),
            (
                save,
                Some("dev"),
                Some("dev"),
                "other-firmware",
                Some("dev"),
            ),
            (
                save,
                Some("bench-debug"),
                Some("bench-debug"),
                "other-firmware",
                Some("bench-debug"),
            ),
            (
                FirmwareTarget::All,
                None,
                None,
                "test-firmware",
                Some("release"),
            ),
            (
                FirmwareTarget::All,
                Some("dev"),
                Some("dev"),
                "test-firmware",
                Some("release"),
            ),
        ] {
            context
                .save_selection("test-firmware", Some(&"release".parse().unwrap()))
                .unwrap();
            let profile = profile.map(|name| name.parse().unwrap());
            let selections = context
                .targets(target, profile.as_ref(), &mut Options::default())
                .unwrap();
            assert!(!selections.is_empty());
            for selection in selections {
                assert_eq!(selection.profile.as_ref().map(Profile::name), expected);
                let command = build_command(&selection);
                let mut expected_args = vec![
                    "build",
                    "--target",
                    "thumbv7em-none-eabihf",
                    "-p",
                    "test-firmware",
                    "--locked",
                ];
                if let Some(profile) = expected {
                    expected_args.extend(["--profile", profile]);
                }
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    expected_args.iter().map(OsStr::new).collect::<Vec<_>>()
                );
            }
            let saved = context.read_selection().unwrap().unwrap();
            assert_eq!(saved.firmware, saved_firmware);
            assert_eq!(saved.profile.as_ref().map(Profile::name), saved_profile);
            let launch: serde_json::Value =
                serde_json::from_slice(&fs::read(context.root.join(vscode::LAUNCH_FILE)).unwrap())
                    .unwrap();
            assert_eq!(
                launch["configurations"][0]["name"],
                match saved_profile {
                    Some(profile) => format!("Launch {saved_firmware} ({profile})"),
                    None => format!("Launch {saved_firmware}"),
                }
            );
            let state: toml::Value =
                toml::from_str(&fs::read_to_string(context.selection_path()).unwrap()).unwrap();
            assert_eq!(
                state.get("profile").and_then(toml::Value::as_str),
                saved_profile
            );
        }
        context.save_selection("other-firmware", None).unwrap();
        assert!(context.read_selection().unwrap().unwrap().profile.is_none());
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn a_custom_profile_builds_in_its_own_directory_even_when_it_inherits_release() {
        let mut context = host_test_context("custom-profile");
        let output = Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let target = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        context
            .registry
            .platforms
            .get_mut("test-platform")
            .unwrap()
            .target = target.clone();
        let workspace = context.root.join("platform/test");
        let manifest = workspace.join("Cargo.toml");
        let source = fs::read_to_string(&manifest).unwrap();
        fs::write(
            manifest,
            format!("{source}\n[profile.bench-debug]\ninherits = \"release\"\n"),
        )
        .unwrap();
        fs::write(
            workspace.join(".cargo/config.toml"),
            format!("[build]\ntarget = {target:?}\n"),
        )
        .unwrap();
        fs::write(workspace.join("firmware/src/main.rs"), "fn main() {}\n").unwrap();
        context
            .registry
            .platforms
            .get_mut("test-platform")
            .unwrap()
            .cpu = "native".into();
        context.resolve_cargo().unwrap();
        let mut selection = context.selection("test-firmware").unwrap();
        selection.profile = Some("bench-debug".parse().unwrap());
        build(std::slice::from_ref(&selection));
        let elf =
            runner::firmware_elf_path(&selection).with_extension(std::env::consts::EXE_EXTENSION);
        assert!(elf.is_file(), "{}", elf.display());
        assert!(
            !context
                .root
                .join("target/firmware/test-platform")
                .join(target)
                .join("release")
                .exists()
        );
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn grouped_checks_preserve_distinct_execution_contexts_in_one_workspace() {
        let context = test_context(test_root("grouped-checks"));
        let platform = context.registry.platforms["test-platform"].clone();
        let scopes = ["first", "second"].map(|id| {
            let mut cargo = CargoContext::fixture(&context.root, id);
            cargo.working_dir = context.root.join(id);
            WorkspaceScope {
                platform: platform.clone(),
                cargo: cargo.clone(),
                firmware: Default::default(),
                packages: vec![PackageScope {
                    name: "common".into(),
                    manifest_path: "common/Cargo.toml".into(),
                    absolute_manifest_path: context.root.join("common/Cargo.toml"),
                    embedded: true,
                    id: "common-id".into(),
                    host_id: "common-owner-id".into(),
                    cargo,
                }],
            }
        });
        let host = test_stages(&scopes);
        assert_eq!(host.len(), 2);
        assert_ne!(
            host[0].command.get_current_dir(),
            host[1].command.get_current_dir()
        );
        for Invocation { command, .. } in &host {
            assert!(command.get_args().any(|arg| arg == "host-tuple"));
            assert!(!command.get_envs().any(|(key, value)| {
                key == "CARGO_TARGET_THUMBV7EM_NONE_EABIHF_RUSTFLAGS" && value.is_some()
            }));
        }
        assert_eq!(fmt_stages(&scopes, true).len(), 2);
        assert_eq!(lint_stages(&scopes).len(), 4);
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn resolving_an_explicit_firmware_with_select_saves_it_before_the_command() {
        let root = test_root("select");
        let context = test_context(root.clone());

        let choice = FirmwareChoice::Save(Some("test-firmware"));
        let selections = context
            .targets(FirmwareTarget::One(choice), None, &mut Options::default())
            .unwrap();

        assert_eq!(selections[0].id, "test-firmware");
        let saved = context.read_selection().unwrap().unwrap();
        assert_eq!(saved.firmware, "test-firmware");
        assert!(saved.profile.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolving_an_explicit_firmware_without_select_does_not_save_it() {
        let root = test_root("transient");
        let context = test_context(root.clone());

        let choice = FirmwareChoice::Explicit("test-firmware");
        let selections = context
            .targets(FirmwareTarget::One(choice), None, &mut Options::default())
            .unwrap();

        assert_eq!(selections[0].id, "test-firmware");
        assert!(!context.selection_path().exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clean_command_uses_the_platform_workspace_and_firmware_target() {
        let root = test_root("clean-command");
        let context = test_context(root.clone());
        let selection = context.selection("test-firmware").unwrap();
        let command = clean_command(&selection, false);
        let workspace = root.join("platform/test");
        let target_dir = root.join("target/firmware/test-platform");

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("clean"),
                OsStr::new("-p"),
                OsStr::new("test-firmware"),
                OsStr::new("--target"),
                OsStr::new("thumbv7em-none-eabihf"),
                OsStr::new("--target"),
                OsStr::new("host-tuple")
            ]
        );
        assert_eq!(command.get_current_dir(), Some(workspace.as_path()));
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == OsStr::new("CARGO_TARGET_DIR"))
                .and_then(|(_, value)| value)
                .map(Path::new),
            Some(target_dir.as_path())
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleaning_a_package_lists_all_affected_firmwares_in_that_platform() {
        let mut context = test_context(test_root("clean-output"));
        let firmware = context.registry.firmwares["test-firmware"].clone();
        let base = context.cargo["test-firmware"].clone();
        let platform = context.registry.platforms["test-platform"].clone();
        context
            .registry
            .platforms
            .insert("other-family".into(), platform);
        for (id, package, platform) in [
            ("secondary-firmware", "test-firmware", "test-platform"),
            ("unrelated-firmware", "test-platform", "test-platform"),
            ("other-family-firmware", "test-firmware", "other-family"),
        ] {
            context.registry.firmwares.insert(
                id.into(),
                Firmware {
                    package: package.into(),
                    platform: platform.into(),
                    ..firmware.clone()
                },
            );
            context.cargo.insert(id.into(), base.clone());
        }
        let selection = context.selection("test-firmware").unwrap();
        let stages = cargo_workflow::clean(&context.registry, &context.cargo, &[selection], false);
        assert_eq!(stages.len(), 1);
        let clean = stages[0].clean.as_ref().unwrap();
        assert_eq!(clean.package.as_deref(), Some("test-firmware"));
        assert_eq!(
            clean.firmwares,
            [
                "other-family-firmware",
                "secondary-firmware",
                "test-firmware"
            ]
        );
        assert!(!stages[0].command.get_args().any(|arg| arg == "--bin"));
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn all_firmwares_clean_command_keeps_the_whole_platform_target() {
        let root = test_root("clean-all-command");
        let context = test_context(root.clone());
        let selection = context.selection("test-firmware").unwrap();
        let command = clean_command(&selection, true);

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("clean")]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn builds_pass_a_binary_only_when_the_registry_explicitly_selects_it() {
        for bin in [None, Some("test-firmware"), Some("robot-image")] {
            let mut context = test_context(test_root("build-bin-argument"));
            let firmware = context.registry.firmwares.get_mut("test-firmware").unwrap();
            firmware.bin = bin.map(str::to_owned);
            context.cargo.get_mut("test-firmware").unwrap().bin =
                bin.unwrap_or("robot-image").into();
            let selection = context.selection("test-firmware").unwrap();
            let command = build_command(&selection);
            let mut expected = vec![
                "build",
                "--target",
                "thumbv7em-none-eabihf",
                "-p",
                "test-firmware",
            ];
            if let Some(bin) = bin {
                expected.extend(["--bin", bin]);
            }
            expected.push("--locked");
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                expected.iter().map(OsStr::new).collect::<Vec<_>>()
            );
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    #[test]
    fn builds_compile_an_inferred_binary_or_only_the_explicit_binary() {
        let output = Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let target = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        for (index, bin) in [None, Some("robot-image")].into_iter().enumerate() {
            let name = bin.unwrap_or("robot-image");
            let context = host_test_context(&format!("build-{index}"));
            let workspace = context.root.join("platform/test");
            fs::write(
                workspace.join(".cargo/config.toml"),
                format!("[build]\ntarget = {target:?}\n"),
            )
            .unwrap();
            fs::create_dir_all(workspace.join("firmware/src/bin")).unwrap();
            fs::write(
                workspace
                    .join("firmware/src/bin")
                    .join(format!("{name}.rs")),
                "fn main() {}\n",
            )
            .unwrap();
            let mut registry = String::from(
                "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n[platforms.test-platform]\ntarget = \"thumbv7em-none-eabihf\"\ncpu = \"cortex-m7\"\ntarget-dir = \"target/firmware/test-platform\"\nworking-dir = \"platform/test\"\n\
             chip = \"test-chip\"\n\
                 [firmwares.test-firmware]\ndescription = \"test firmware\"\nplatform = \"test-platform\"\n\
                 package = \"test-firmware\"\n",
            );
            if let Some(bin) = bin {
                fs::write(
                    workspace.join("firmware/src/bin/unselected.rs"),
                    "compile_error!(\"an unrelated binary must not be built\");\n",
                )
                .unwrap();
                registry.push_str(&format!(
                    "bin = {bin:?}\n\
                     [firmwares.other-firmware]\ndescription = \"another binary\"\n\
                     platform = \"test-platform\"\npackage = \"test-firmware\"\nbin = \"unselected\"\n"
                ));
            }
            fs::write(
                context.root.join("polyfw.toml"),
                registry
                    .replace(
                        "target = \"thumbv7em-none-eabihf\"",
                        "target = \"host-tuple\"",
                    )
                    .replace("cpu = \"cortex-m7\"", "cpu = \"native\""),
            )
            .unwrap();
            let context = Context::load(context.root).unwrap();

            let selection = context.selection("test-firmware").unwrap();
            assert_eq!(selection.resolved_bin, name);
            assert_eq!(selection.firmware.bin.as_deref(), bin);
            build(&[selection]);
            assert!(
                context
                    .root
                    .join("target/firmware/test-platform")
                    .join(&target)
                    .join("debug")
                    .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
                    .is_file()
            );
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    #[test]
    fn host_lint_rejects_mistakes_in_both_platform_and_firmware_tests() {
        for (package, path) in [
            ("test-platform", "src/lib.rs"),
            ("test-firmware", "firmware/src/lib.rs"),
        ] {
            let context = host_test_context(&format!("test-lint-{package}"));
            fs::write(
                context.root.join("platform/test").join(path),
                "#![no_std]\n#![forbid(unsafe_code)]\n\
                 #[cfg(test)]\nmod tests {\n\
                 #[test]\nfn an_empty_slice_has_no_elements() {\n\
                 let values: &[u8] = &[];\nassert!(values.len() == 0);\n}\n}\n",
            )
            .unwrap();
            let selection = context.selection("test-firmware").unwrap();
            let scopes = scopes(&context, &[selection]);
            let mut host_stages = 0;

            for Invocation {
                target,
                mut command,
                ..
            } in lint_stages(&scopes)
            {
                let label = format!("{target:?}");
                let host = command.get_args().any(|argument| argument == "host-tuple");
                let output = command.output().unwrap();
                let stderr = String::from_utf8_lossy(&output.stderr);
                if host {
                    assert!(
                        !output.status.success(),
                        "{label} missed {package}'s test lint"
                    );
                    assert!(stderr.contains("clippy::len_zero"), "{label}: {stderr}");
                    host_stages += 1;
                } else {
                    assert!(output.status.success(), "{label}: {stderr}");
                }
            }
            assert_eq!(host_stages, 1, "both packages share one host lint stage");
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    #[test]
    fn host_tests_reject_failing_doctests_in_both_platform_and_firmware() {
        for (package, path) in [
            ("test-platform", "src/lib.rs"),
            ("test-firmware", "firmware/src/lib.rs"),
        ] {
            let context = host_test_context(&format!("doctest-{package}"));
            fs::write(
                context.root.join("platform/test").join(path),
                "//! ```\n//! assert!(false);\n//! ```\n\
                 #![no_std]\n#![forbid(unsafe_code)]\n",
            )
            .unwrap();
            let selection = context.selection("test-firmware").unwrap();
            let scopes = scopes(&context, &[selection]);
            let stages = test_stages(&scopes);
            assert_eq!(stages.len(), 1, "both packages share one host test stage");

            for Invocation {
                kind, mut command, ..
            } in stages
            {
                let label = format!("{kind:?}");
                let output = command.output().unwrap();
                assert!(
                    !output.status.success(),
                    "{label} missed {package}'s failing doctest"
                );
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("doctest failed"),
                    "{label}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    fn host_test_context(label: &str) -> Context {
        let root = test_root(label);
        let workspace = root.join("platform/test");
        fs::create_dir_all(workspace.join("firmware/src")).unwrap();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(workspace.join(".cargo")).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"test-platform\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [workspace]\nmembers = [\"firmware\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::write(
            workspace.join("firmware/Cargo.toml"),
            "[package]\nname = \"test-firmware\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [target.'cfg(target_os = \"none\")'.dependencies]\n\
             test-platform = { path = \"..\" }\n",
        )
        .unwrap();
        let source = "#![no_std]\n#![forbid(unsafe_code)]\n\
             #[cfg(test)]\nmod tests {\n\
             #[test]\nfn an_empty_slice_has_no_elements() {\n\
             let values: &[u8] = &[];\nassert!(values.is_empty());\n}\n}\n";
        fs::write(workspace.join("src/lib.rs"), source).unwrap();
        fs::write(workspace.join("firmware/src/lib.rs"), source).unwrap();
        fs::write(
            workspace.join(".cargo/config.toml"),
            "[build]\ntarget = \"thumbv7em-none-eabihf\"\n",
        )
        .unwrap();
        let output = runner::workspace_cargo(&workspace)
            .args(["generate-lockfile", "--offline"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        test_context(root)
    }

    fn test_context(root: PathBuf) -> Context {
        Context {
            root: root.clone(),
            cargo: BTreeMap::from([(
                "test-firmware".into(),
                FirmwareCargo {
                    bin: "test-firmware".into(),
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
                        target_dir: Some("target/firmware/test-platform".into()),
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
                        ..Firmware::default()
                    },
                )]),
            },
        }
    }

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("polyfw-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("rust-toolchain.toml"),
            include_str!("../../rust-toolchain.toml"),
        )
        .unwrap();
        root
    }
}
