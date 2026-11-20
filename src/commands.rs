use std::process::{Command, ExitCode};

use crate::Result;
use crate::cli::{FirmwareArgs, Task};
use crate::info::Report;
use crate::model::Selection;
use crate::probe::{self, Action};
use crate::profile::Profile;
use crate::registry::{Context, unique_contexts};
use crate::runner;
use crate::scope::{self, PackageScope, WorkspaceScope};
use crate::ui::{self, Progress, Ui, run_command_with_progress};

pub(crate) fn run(context: Context, task: Task, ui: &Ui) -> Result<ExitCode> {
    let selections = match task.firmware_args() {
        Some(firmwares) => resolve(&context, ui, &firmwares, task.profile_override())?,
        None => Vec::new(),
    };
    if matches!(&task, Task::Build { .. } | Task::Lint(_) | Task::Verify(_)) {
        for selection in unique_contexts(&selections) {
            crate::cargo_config::check(selection)?;
        }
    }
    let scopes = match &task {
        Task::Fmt(_) | Task::Lint(_) | Task::Test(_) | Task::Verify(_) => {
            scope::resolve(&context, &selections)?
        }
        Task::Init
        | Task::Schema { .. }
        | Task::Info { .. }
        | Task::Select { .. }
        | Task::Clean(_)
        | Task::Build { .. }
        | Task::Probes
        | Task::Run { .. }
        | Task::Flash { .. }
        | Task::Attach { .. }
        | Task::Reset(_) => Vec::new(),
    };
    match task {
        Task::Init => crate::init::run(&context.root, ui),
        Task::Schema { output } => crate::registry::write_schema(output.as_deref()),
        Task::Info { firmware, json } => info(&context, ui, firmware, json),
        Task::Select {
            firmware,
            clear,
            profile,
        } => {
            return select(&context, ui, firmware.as_deref(), clear, profile.explicit());
        }
        Task::Clean(firmwares) => clean(&context, ui, &selections, firmwares.all_firmwares),
        Task::Build { .. } => build(&context, ui, &selections),
        Task::Probes => probe::list(&context.root, ui),
        Task::Run {
            target, monitor, ..
        } => probe::run(
            &context,
            ui,
            &selections[0],
            Action::Run { monitor: &monitor },
            &target,
        ),
        Task::Flash { target, .. } => {
            probe::run(&context, ui, &selections[0], Action::Flash, &target)
        }
        Task::Attach {
            target, monitor, ..
        } => probe::run(
            &context,
            ui,
            &selections[0],
            Action::Attach { monitor: &monitor },
            &target,
        ),
        Task::Reset(target) => probe::run(&context, ui, &selections[0], Action::Reset, &target),
        Task::Fmt(_) => run_stages(ui, fmt_stages(&context, ui, &scopes, false)),
        Task::Lint(_) => run_stages(ui, lint_stages(&context, ui, &scopes)),
        Task::Test(_) => run_stages(ui, test_stages(&context, ui, &scopes)),
        Task::Verify(_) => {
            let mut stages = fmt_stages(&context, ui, &scopes, true);
            stages.extend(lint_stages(&context, ui, &scopes));
            stages.extend(test_stages(&context, ui, &scopes));
            run_stages(ui, stages)
        }
    }?;
    Ok(ExitCode::SUCCESS)
}

fn info(context: &Context, ui: &Ui, firmware: Option<Option<String>>, json: bool) -> Result<()> {
    let show_saved_selection = !matches!(firmware, Some(None));
    let firmware = match firmware {
        None => None,
        Some(Some(firmware)) => Some(firmware),
        Some(None) => {
            let saved = context.read_saved_selection()?;
            Some(ui.prompt_firmware(
                &context.registry,
                saved.as_ref().map(|saved| saved.firmware.as_str()),
                "pass a firmware name with `polyfw info --firmware <FIRMWARE>`",
            )?)
        }
    };
    let report = Report::load(context, firmware.as_deref())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        ui.print_info(&report, show_saved_selection);
    }
    Ok(())
}

fn select(
    context: &Context,
    ui: &Ui,
    firmware: Option<&str>,
    clear: bool,
    profile: Option<Profile>,
) -> Result<ExitCode> {
    if clear {
        let mut exit = ExitCode::SUCCESS;
        for (path, result) in context.clear_selection() {
            ui.removal(path, &result);
            if result.is_err_and(|error| error.kind() != std::io::ErrorKind::NotFound) {
                exit = ExitCode::FAILURE;
            }
        }
        return Ok(exit);
    }

    let firmware = match firmware {
        Some(firmware) => firmware.to_owned(),
        None => {
            let saved = context.read_selection()?;
            ui.prompt_firmware(
                &context.registry,
                saved.as_ref().map(|saved| saved.firmware.as_str()),
                "pass a firmware name or run polyfw select in a terminal",
            )?
        }
    };
    context.save_selection(&firmware, profile.as_ref())?;
    ui.selected(&firmware, profile.as_ref());
    Ok(ExitCode::SUCCESS)
}

fn clean(context: &Context, ui: &Ui, selections: &[Selection], all_firmwares: bool) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    let selected = selections
        .iter()
        .filter(|selection| {
            !all_firmwares
                || seen.insert((
                    selection.firmware.cargo.working_dir.clone(),
                    selection.firmware.cargo.target_dir.clone(),
                    selection.firmware.cargo.build_dir.clone(),
                ))
        })
        .collect::<Vec<_>>();
    let mut progress = Progress::new(ui, selected.len());
    for selection in selected {
        let cargo = &selection.firmware.cargo;
        let firmwares = context
            .registry
            .firmwares
            .iter()
            .filter(|(_, entry)| {
                (entry.cargo.target_dir == cargo.target_dir
                    || cargo
                        .build_dir
                        .as_ref()
                        .is_some_and(|directory| entry.cargo.build_dir.as_ref() == Some(directory)))
                    && (all_firmwares || entry.package == selection.firmware.package)
            })
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>();
        if all_firmwares {
            ui.cleaning_directories(&cargo.target_dir.display().to_string(), &firmwares);
        } else {
            ui.cleaning_package(
                &selection.firmware.package,
                &cargo.target_dir.display().to_string(),
                &firmwares,
            );
        }
        run_command_with_progress(
            ui,
            &mut progress,
            &ui.label("clean", &[&selection.id], ui::Target::None),
            clean_command(context, selection, all_firmwares),
        )?;
    }
    Ok(())
}

fn clean_command(_context: &Context, selection: &Selection, all_firmwares: bool) -> Command {
    let mut command = runner::cargo_command(&selection.firmware.cargo);
    command.arg("clean");
    if !all_firmwares {
        command
            .arg("-p")
            .arg(&selection.firmware.package)
            .arg("--target")
            .arg(&selection.platform.target)
            .args(["--target", "host-tuple"]);
    }
    command
}

fn build(_context: &Context, ui: &Ui, selections: &[Selection]) -> Result<()> {
    let mut progress = Progress::new(ui, selections.len());
    for selection in selections {
        let command = runner::build_command(selection);
        let firmware = match &selection.profile {
            Some(profile) => format!("{} ({})", selection.id, profile.name()),
            None => selection.id.clone(),
        };
        let label = ui.label("build", &[&firmware], ui::Target::Embedded);
        run_command_with_progress(ui, &mut progress, &label, command)?;
    }
    Ok(())
}

struct Stage(String, Command);

fn run_stages(ui: &Ui, stages: Vec<Stage>) -> Result<()> {
    let mut progress = Progress::new(ui, stages.len());
    for Stage(label, command) in stages {
        run_command_with_progress(ui, &mut progress, &label, command)?;
    }
    Ok(())
}

fn fmt_stages(_context: &Context, ui: &Ui, scopes: &[WorkspaceScope], check: bool) -> Vec<Stage> {
    let mut stages = Vec::new();
    for scope in scope::host_scopes(scopes) {
        let mut command = runner::cargo_command(&scope.cargo);
        command.arg("fmt");
        let names = add_packages(&mut command, scope.packages, |package| &package.name);
        let action = if check {
            command.arg("--check");
            "format check"
        } else {
            "format"
        };
        stages.push(Stage(ui.label(action, &names, ui::Target::None), command));
    }
    stages
}

fn lint_stages(_context: &Context, ui: &Ui, scopes: &[WorkspaceScope]) -> Vec<Stage> {
    let mut stages = Vec::new();
    for scope in scopes {
        if scope.packages.is_empty() {
            continue;
        }
        let mut command = runner::target_cargo(&scope.cargo, &scope.platform);
        command
            .arg("clippy")
            .arg("--target")
            .arg(&scope.platform.target);
        let names = add_packages(
            &mut command,
            scope.packages.iter().filter(|package| package.embedded),
            |package| &package.id,
        );
        if !names.is_empty() {
            command
                .args(["--lib", "--bins"])
                .args(["--locked", "--", "-D", "warnings"]);
            stages.push(Stage(
                ui.label("lint", &names, ui::Target::Embedded),
                command,
            ));
        }
    }
    for scope in scope::host_scopes(scopes) {
        let mut host = runner::cargo_command(&scope.cargo);
        host.arg("clippy");
        let names = add_packages(&mut host, scope.packages, |package| &package.host_id);
        host.args([
            "--lib",
            "--tests",
            "--target",
            "host-tuple",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]);
        stages.push(Stage(ui.label("lint", &names, ui::Target::Host), host));
    }
    stages
}

fn test_stages(_context: &Context, ui: &Ui, scopes: &[WorkspaceScope]) -> Vec<Stage> {
    let mut stages = Vec::new();
    for scope in scope::host_scopes(scopes) {
        let mut command = runner::cargo_command(&scope.cargo);
        command.arg("test");
        let names = add_packages(&mut command, scope.packages, |package| &package.host_id);
        command.args(["--target", "host-tuple", "--locked"]);
        stages.push(Stage(ui.label("test", &names, ui::Target::Host), command));
    }
    stages
}

fn add_packages<'a>(
    command: &mut Command,
    packages: impl IntoIterator<Item = &'a PackageScope>,
    spec: fn(&PackageScope) -> &str,
) -> Vec<&'a str> {
    packages
        .into_iter()
        .map(|package| {
            let name = package.name.as_str();
            command.arg("-p").arg(spec(package));
            name
        })
        .collect()
}

fn resolve(
    context: &Context,
    ui: &Ui,
    firmwares: &FirmwareArgs,
    profile: Option<Profile>,
) -> Result<Vec<Selection>> {
    if firmwares.all_firmwares {
        return context
            .registry
            .firmwares
            .keys()
            .map(|id| {
                let mut selection = context.selection(id)?;
                selection.profile = profile.clone();
                Ok(selection)
            })
            .collect();
    }

    let saved = if firmwares.select && firmwares.firmware.is_some() {
        None
    } else {
        context.read_selection()?
    };
    let id = match (
        firmwares.firmware.as_deref(),
        firmwares.select,
        saved.as_ref(),
    ) {
        (Some(id), _, _) => id.to_owned(),
        (None, false, Some(saved)) => saved.firmware.clone(),
        (None, _, saved) => ui.prompt_firmware(
            &context.registry,
            saved.map(|saved| saved.firmware.as_str()),
            "pass a firmware name or run polyfw select in a terminal",
        )?,
    };
    let mut selection = context.selection(&id)?;
    selection.profile = profile.or_else(|| {
        if firmwares.select {
            None
        } else {
            saved.and_then(|saved| saved.profile)
        }
    });
    if firmwares.select {
        context.save_selection(&id, selection.profile.as_ref())?;
        ui.selected(&id, selection.profile.as_ref());
    }
    Ok(vec![selection])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};

    use clap::ColorChoice;
    use clap::Parser;

    use super::*;
    use crate::model::{Firmware, Platform, Registry};

    #[test]
    fn info_requires_a_terminal_to_pick_even_when_a_firmware_is_saved() {
        const MODE: &str = "FLEET_XTASK_TEST_INFO_PICKER";
        if std::env::var_os(MODE).is_some() {
            for saved in [false, true] {
                for json in [false, true] {
                    let root = test_root(&format!("info-picker-{saved}-{json}"));
                    let context = test_context(root.clone());
                    if saved {
                        context
                            .save_selection("test-firmware", Some(&"release".parse().unwrap()))
                            .unwrap();
                    }
                    let state_path = context.selection_path();
                    let launch_path = root.join(crate::vscode::LAUNCH_FILE);
                    let state = fs::read(&state_path).ok();
                    let launch = fs::read(&launch_path).ok();
                    let mut args = vec!["polyfw", "info", "--firmware"];
                    if json {
                        args.push("--json");
                    }
                    let task = crate::cli::Cli::try_parse_from(args)
                        .unwrap()
                        .command
                        .unwrap();
                    let error = run(context, task, &Ui::new(ColorChoice::Never)).unwrap_err();
                    assert!(
                        error
                            .to_string()
                            .contains("polyfw info --firmware <FIRMWARE>"),
                        "{error}"
                    );
                    assert_eq!(fs::read(state_path).ok(), state);
                    assert_eq!(fs::read(launch_path).ok(), launch);
                    fs::remove_dir_all(root).unwrap();
                }
            }
            return;
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::tests::info_requires_a_terminal_to_pick_even_when_a_firmware_is_saved",
                "--nocapture",
            ])
            .env(MODE, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(!stdout.contains("repository_root"), "{stdout}");
    }

    #[test]
    fn saving_a_selection_generates_matching_launch_and_attach_without_building() {
        let mut context = test_context(test_root("vscode profiles with spaces"));
        let firmware = context.registry.firmwares.get_mut("test-firmware").unwrap();
        firmware.bin = Some("robot-image".into());
        firmware.resolved_bin = "robot-image".into();
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
        for firmware in ["test-firmware", "other-firmware"] {
            for (profile, directory, suffix) in [
                (None, "debug", ""),
                (Some("dev"), "debug", " (dev)"),
                (Some("release"), "release", " (release)"),
                (Some("bench-debug"), "bench-debug", " (bench-debug)"),
            ] {
                let _ = select(
                    &context,
                    &Ui::new(ColorChoice::Never),
                    Some(firmware),
                    false,
                    profile.map(|profile| profile.parse().unwrap()),
                )
                .unwrap();
                let state: toml::Value =
                    toml::from_str(&fs::read_to_string(context.selection_path()).unwrap()).unwrap();
                assert_eq!(state.get("profile").and_then(toml::Value::as_str), profile);
                let value: serde_json::Value = serde_json::from_slice(
                    &fs::read(context.root.join(crate::vscode::LAUNCH_FILE)).unwrap(),
                )
                .unwrap();
                let configurations = value["configurations"].as_array().unwrap();
                assert_eq!(configurations.len(), 2);
                let selection = context.selection(firmware).unwrap();
                let expected_elf = selection
                    .firmware
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
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(context.root.join(crate::vscode::LAUNCH_FILE)).unwrap(),
        )
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
        let ui = Ui::new(ColorChoice::Never);
        let _ = select(&context, &ui, None, true, None).unwrap();
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
        fs::write(editor.join("launch.json"), "{}\n").unwrap();
        let removed = context.clear_selection();
        assert!(removed[0].1.is_ok());
        assert_eq!(
            removed[1].1.as_ref().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(!editor.join("launch.json").exists());
        assert_eq!(
            select(&context, &ui, None, true, None).unwrap(),
            ExitCode::SUCCESS
        );
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn clearing_selection_attempts_every_file_and_returns_failure_after_an_error() {
        for failed_file in [crate::vscode::LAUNCH_FILE, ".polyfw-state/selected"] {
            let context = test_context(test_root(&format!("clear-failed-{failed_file}")));
            context.save_selection("test-firmware", None).unwrap();
            let failed_path = context.root.join(failed_file);
            fs::remove_file(&failed_path).unwrap();
            fs::create_dir(&failed_path).unwrap();

            assert_eq!(
                select(&context, &Ui::new(ColorChoice::Never), None, true, None).unwrap(),
                ExitCode::FAILURE
            );
            assert!(failed_path.is_dir());
            for file in [crate::vscode::LAUNCH_FILE, ".polyfw-state/selected"] {
                if file != failed_file {
                    assert!(!context.root.join(file).exists());
                }
            }
            fs::remove_dir_all(context.root).unwrap();
        }
    }

    #[test]
    fn disabling_vscode_leaves_existing_editor_files_untouched_when_saving_or_clearing() {
        let mut context = test_context(test_root("vscode-disabled"));
        context.save_selection("test-firmware", None).unwrap();
        let path = context.root.join(crate::vscode::LAUNCH_FILE);
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
        let exit = select(&context, &Ui::new(ColorChoice::Never), None, true, None).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(!context.selection_path().exists());
        assert_eq!(fs::read(&path).unwrap(), previous);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        context.save_selection("test-firmware", None).unwrap();
        assert_eq!(context.clear_selection().len(), 1);
        assert!(path.is_dir());
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn firmware_commands_that_save_a_selection_respect_the_vscode_switch() {
        use clap::Parser;

        for command in [
            "clean", "build", "fmt", "lint", "test", "verify", "run", "flash", "attach",
        ] {
            let mut context = test_context(test_root(&format!("vscode-disabled-{command}")));
            context.registry.config.vscode.enabled = false;
            fs::write(context.root.join(".vscode"), "not a directory").unwrap();
            let root = context.root.clone();
            let task =
                crate::cli::Cli::try_parse_from(["polyfw", command, "test-firmware", "--select"])
                    .unwrap()
                    .command
                    .unwrap();
            // Every action fails on this fixture after saving; the selected firmware remains saved.
            assert!(run(context, task, &Ui::new(ColorChoice::Never)).is_err());
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
            let path = context.root.join(crate::vscode::LAUNCH_FILE);
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
        let ui = Ui::new(ColorChoice::Never);
        for (args, expected, saved_firmware, saved_profile) in [
            (
                vec!["build"],
                Some("release"),
                "test-firmware",
                Some("release"),
            ),
            (
                vec!["build", "other-firmware"],
                Some("release"),
                "test-firmware",
                Some("release"),
            ),
            (
                vec!["build", "other-firmware", "--profile", "dev"],
                Some("dev"),
                "test-firmware",
                Some("release"),
            ),
            (
                vec!["build", "other-firmware", "--select"],
                None,
                "other-firmware",
                None,
            ),
            (
                vec!["build", "other-firmware", "--select", "--profile", "dev"],
                Some("dev"),
                "other-firmware",
                Some("dev"),
            ),
            (
                vec![
                    "build",
                    "other-firmware",
                    "--select",
                    "--profile",
                    "bench-debug",
                ],
                Some("bench-debug"),
                "other-firmware",
                Some("bench-debug"),
            ),
            (
                vec!["build", "--all-firmwares"],
                None,
                "test-firmware",
                Some("release"),
            ),
            (
                vec!["build", "--all-firmwares", "--profile", "dev"],
                Some("dev"),
                "test-firmware",
                Some("release"),
            ),
        ] {
            context
                .save_selection("test-firmware", Some(&"release".parse().unwrap()))
                .unwrap();
            let task = crate::cli::Cli::try_parse_from(std::iter::once("polyfw").chain(args))
                .unwrap()
                .command
                .unwrap();
            let selections = resolve(
                &context,
                &ui,
                &task.firmware_args().unwrap(),
                task.profile_override(),
            )
            .unwrap();
            assert!(!selections.is_empty());
            for selection in selections {
                assert_eq!(selection.profile.as_ref().map(Profile::name), expected);
                let command = runner::build_command(&selection);
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
            let launch: serde_json::Value = serde_json::from_slice(
                &fs::read(context.root.join(crate::vscode::LAUNCH_FILE)).unwrap(),
            )
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
        let _ = select(&context, &ui, Some("other-firmware"), false, None).unwrap();
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
        build(&context, &Ui::new(ColorChoice::Never), &[selection.clone()]).unwrap();
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
            let mut cargo = crate::model::CargoContext::fixture(&context.root, id);
            cargo.working_dir = context.root.join(id);
            WorkspaceScope {
                platform: platform.clone(),
                cargo: cargo.clone(),
                firmware: Default::default(),
                packages: vec![PackageScope {
                    name: "common".into(),
                    manifest_path: "common/Cargo.toml".into(),
                    embedded: true,
                    id: "common-id".into(),
                    host_id: "common-owner-id".into(),
                    cargo,
                }],
            }
        });
        let ui = Ui::new(ColorChoice::Never);
        let host = test_stages(&context, &ui, &scopes);
        assert_eq!(host.len(), 2);
        assert_ne!(host[0].1.get_current_dir(), host[1].1.get_current_dir());
        for Stage(_, command) in &host {
            assert!(command.get_args().any(|arg| arg == "host-tuple"));
            assert!(
                !command
                    .get_envs()
                    .any(|(key, _)| key == "CARGO_TARGET_THUMBV7EM_NONE_EABIHF_RUSTFLAGS")
            );
        }
        assert_eq!(fmt_stages(&context, &ui, &scopes, true).len(), 2);
        assert_eq!(lint_stages(&context, &ui, &scopes).len(), 4);
        fs::remove_dir_all(context.root).unwrap();
    }

    #[test]
    fn resolving_an_explicit_firmware_with_select_saves_it_before_the_command() {
        let root = test_root("select");
        let context = test_context(root.clone());
        let ui = Ui::new(ColorChoice::Never);

        let selections = resolve(
            &context,
            &ui,
            &FirmwareArgs {
                firmware: Some("test-firmware".into()),
                select: true,
                all_firmwares: false,
            },
            None,
        )
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
        let ui = Ui::new(ColorChoice::Never);

        let selections = resolve(
            &context,
            &ui,
            &FirmwareArgs {
                firmware: Some("test-firmware".into()),
                select: false,
                all_firmwares: false,
            },
            None,
        )
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
        let command = clean_command(&context, &selection, false);
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
        const MODE: &str = "FLEET_XTASK_TEST_CLEAN_OUTPUT";
        if std::env::var_os(MODE).is_some() {
            let mut context = host_test_context("clean-output");
            let firmware = context.registry.firmwares["test-firmware"].clone();
            context.registry.firmwares.insert(
                "secondary-firmware".into(),
                Firmware {
                    bin: Some("second-image".into()),
                    resolved_bin: "second-image".into(),
                    ..firmware.clone()
                },
            );
            context.registry.firmwares.insert(
                "unrelated-firmware".into(),
                Firmware {
                    package: "test-platform".into(),
                    resolved_bin: "test-platform".into(),
                    ..firmware.clone()
                },
            );
            let platform = context.registry.platforms["test-platform"].clone();
            context
                .registry
                .platforms
                .insert("other-family".into(), platform);
            context.registry.firmwares.insert(
                "other-family-firmware".into(),
                Firmware {
                    platform: "other-family".into(),
                    ..firmware
                },
            );
            let root = context.root.clone();
            let _ = run(
                context,
                Task::Clean(FirmwareArgs {
                    firmware: Some("test-firmware".into()),
                    select: false,
                    all_firmwares: false,
                }),
                &Ui::new(ColorChoice::Never),
            )
            .unwrap();
            fs::remove_dir_all(root).unwrap();
            return;
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::tests::cleaning_a_package_lists_all_affected_firmwares_in_that_platform",
                "--nocapture",
            ])
            .env(MODE, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(
            stdout.contains("firmwares: other-family-firmware, secondary-firmware, test-firmware)"),
            "{stdout}"
        );
        assert!(!stdout.contains("unrelated-firmware"), "{stdout}");
        assert!(stdout.contains("other-family-firmware"), "{stdout}");
        assert!(
            stderr.contains("$ cargo clean -p test-firmware"),
            "{stderr}"
        );
        assert!(!stderr.contains("--bin"), "{stderr}");
    }

    #[test]
    fn all_firmwares_clean_command_keeps_the_whole_platform_target() {
        let root = test_root("clean-all-command");
        let context = test_context(root.clone());
        let selection = context.selection("test-firmware").unwrap();
        let command = clean_command(&context, &selection, true);

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
            firmware.resolved_bin = bin.unwrap_or("robot-image").into();
            let selection = context.selection("test-firmware").unwrap();
            let command = runner::build_command(&selection);
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
            assert_eq!(selection.firmware.resolved_bin, name);
            assert_eq!(selection.firmware.bin.as_deref(), bin);
            build(&context, &Ui::new(ColorChoice::Never), &[selection]).unwrap();
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
            let scopes = scope::resolve(&context, &[selection]).unwrap();
            let ui = Ui::new(ColorChoice::Never);
            let mut host_stages = 0;

            for Stage(label, mut command) in lint_stages(&context, &ui, &scopes) {
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
            let scopes = scope::resolve(&context, &[selection]).unwrap();
            let ui = Ui::new(ColorChoice::Never);
            let stages = test_stages(&context, &ui, &scopes);
            assert_eq!(stages.len(), 1, "both packages share one host test stage");

            for Stage(label, mut command) in stages {
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
            registry: Registry {
                config_version: crate::config_version::CLI_VERSION.into(),
                min_polyfw_version: crate::config_version::CLI_VERSION.into(),
                config: crate::model::Config::default(),
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
                        resolved_bin: "test-firmware".into(),
                        cargo: crate::model::CargoContext::fixture(&root, "test-platform"),
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
            include_str!("../rust-toolchain.toml"),
        )
        .unwrap();
        root
    }
}
