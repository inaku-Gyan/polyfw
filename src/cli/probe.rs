use std::io;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use polyfw::{Options, Profile, Project};

use crate::Result;
use crate::args::{MonitorArgs, ProbeArgs};
use crate::dispatch::choice;
use crate::ui::Ui;

#[derive(Clone, Copy)]
pub(crate) enum Action<'a> {
    Run { monitor: &'a MonitorArgs },
    Flash,
    Attach { monitor: &'a MonitorArgs },
    Reset,
}

impl Action<'_> {
    fn name(&self) -> &'static str {
        match self {
            Self::Run { .. } => "run",
            Self::Flash => "download",
            Self::Attach { .. } => "attach",
            Self::Reset => "reset",
        }
    }
}

pub(crate) fn list(root: &Path, ui: &Ui) -> Result<()> {
    let mut command = Command::new("probe-rs");
    command.current_dir(root).arg("list");
    with_installation_hint(run_command(ui, command))
}

pub(crate) fn run(
    project: &Project,
    ui: &Ui,
    action: Action<'_>,
    arguments: &ProbeArgs,
    profile: Option<&Profile>,
    options: Options<'_>,
) -> Result<()> {
    let choice = choice(arguments.firmware.as_deref(), arguments.select);
    let artifact;
    let (firmware, image) = if matches!(action, Action::Reset) {
        (project.resolve_firmware(choice, options)?, None)
    } else {
        artifact = project.artifact(choice, profile, options)?;
        let firmware = project
            .firmware(&artifact.firmware)
            .expect("resolved artifact belongs to registered firmware");
        (
            firmware,
            Some(Image {
                firmware: &artifact.firmware,
                path: &artifact.path,
                profile: artifact.profile.as_ref(),
            }),
        )
    };
    let command = command(project.root(), firmware.chip, image, action, arguments)?;
    // Keep the parent waiting while the child handles the terminal's Ctrl-C and cleans up.
    ctrlc::set_handler(|| {})?;
    let result = if matches!(action, Action::Reset) {
        run_command(ui, command)
    } else {
        let label = ui.label(action.name(), &[firmware.id], None);
        ui.start_progress(1, 1, &label);
        let started = Instant::now();
        let result = run_command(ui, command);
        ui.finish_progress(started.elapsed(), result.is_ok());
        result
    };
    with_installation_hint(result)
}

#[derive(Clone, Copy)]
struct Image<'a> {
    firmware: &'a str,
    path: &'a Path,
    profile: Option<&'a Profile>,
}

fn run_command(ui: &Ui, mut command: Command) -> io::Result<()> {
    let rendered = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    ui.command(&rendered);
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{rendered} failed with {status}")))
    }
}

fn command(
    root: &Path,
    chip: &str,
    image: Option<Image<'_>>,
    action: Action<'_>,
    options: &ProbeArgs,
) -> Result<Command> {
    let mut command = Command::new("probe-rs");
    command
        .current_dir(root)
        .arg(action.name())
        .arg("--chip")
        .arg(chip);
    if let Some(probe) = &options.probe {
        command.arg("--probe").arg(probe);
    }
    if let Some(speed_khz) = options.speed_khz {
        command.arg("--speed").arg(speed_khz.to_string());
    }
    if options.connect_under_reset {
        command.arg("--connect-under-reset");
    }
    match action {
        Action::Run { monitor } => {
            command.arg(existing_elf(image)?).arg("--verify");
            add_monitor(&mut command, monitor);
        }
        Action::Flash => {
            command
                .arg(existing_elf(image)?)
                .args(["--verify", "--reset"]);
        }
        Action::Attach { monitor } => {
            command.arg(existing_elf(image)?);
            add_monitor(&mut command, monitor);
        }
        Action::Reset => {}
    }
    Ok(command)
}

fn existing_elf(image: Option<Image<'_>>) -> Result<&Path> {
    let image = image.ok_or("hardware action requires an ELF path")?;
    if !image.path.is_file() {
        let profile = image
            .profile
            .map(|profile| format!(" --profile {}", profile.name()))
            .unwrap_or_default();
        return Err(format!(
            "ELF not found: {}; build it with polyfw build {}{profile}",
            image.path.display(),
            image.firmware,
        )
        .into());
    }
    Ok(image.path)
}

fn with_installation_hint(result: io::Result<()>) -> Result<()> {
    result.map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            format!(
                "{error}; install probe-rs from \
                 https://probe.rs/docs/getting-started/installation/"
            )
            .into()
        } else {
            error.into()
        }
    })
}

fn add_monitor(command: &mut Command, options: &MonitorArgs) {
    command
        .arg("--rtt-channel-mode")
        .arg(options.rtt_channel_mode.as_str());
    if options.no_catch_reset {
        command.arg("--no-catch-reset");
    }
    if options.no_catch_hardfault {
        command.arg("--no-catch-hardfault");
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;

    use clap::ColorChoice;

    use super::*;
    use crate::args::RttChannelMode;

    #[test]
    fn only_a_missing_probe_command_reports_the_installation_link() {
        let fixture = Fixture::new("launch");
        let ui = Ui::new(ColorChoice::Never);
        let mut failed = Command::new("rustc");
        failed.arg("--polyfw-invalid-argument");
        for (command, missing) in [
            (Command::new(fixture.root.join("probe-rs")), true),
            (Command::new(&fixture.root), false),
            (failed, false),
        ] {
            let result = run_command(&ui, command);
            let original = result.as_ref().unwrap_err().to_string();
            let error = with_installation_hint(result).unwrap_err();
            if missing {
                assert_eq!(
                    error.to_string(),
                    format!(
                        "{original}; install probe-rs from \
                         https://probe.rs/docs/getting-started/installation/"
                    )
                );
            } else {
                assert_eq!(error.to_string(), original);
            }
        }
    }

    #[test]
    fn run_and_flash_select_the_existing_image_and_request_verification() {
        let mut fixture = Fixture::new("existing");
        let monitor = MonitorArgs::default();
        for name in [None, Some("dev"), Some("release"), Some("bench-debug")] {
            fixture.profile = name.map(|name| name.parse().unwrap());
            let elf = fixture.write_elf();
            for flash in [false, true] {
                let action = if flash {
                    Action::Flash
                } else {
                    Action::Run { monitor: &monitor }
                };
                let command = fixture.command(action, &ProbeArgs::default()).unwrap();
                let mut expected = vec![
                    OsStr::new(if flash { "download" } else { "run" }),
                    OsStr::new("--chip"),
                    OsStr::new("test-chip"),
                    elf.as_os_str(),
                    OsStr::new("--verify"),
                ];
                if flash {
                    expected.push(OsStr::new("--reset"));
                } else {
                    expected.extend([
                        OsStr::new("--rtt-channel-mode"),
                        OsStr::new("no-block-skip"),
                    ]);
                }
                assert_eq!(command.get_program(), OsStr::new("probe-rs"));
                assert_eq!(command.get_current_dir(), Some(fixture.root.as_path()));
                assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
            }
        }
    }

    #[test]
    fn attach_selects_the_profile_and_connection_and_monitor_options() {
        let mut fixture = Fixture::new("attach");
        fixture.profile = Some("release".parse().unwrap());
        let elf = fixture.write_elf();
        let monitor = MonitorArgs {
            rtt_channel_mode: RttChannelMode::NoBlockTrim,
            no_catch_reset: true,
            no_catch_hardfault: true,
        };
        let command = fixture
            .command(
                Action::Attach { monitor: &monitor },
                &ProbeArgs {
                    probe: Some("1234:5678:serial with spaces".into()),
                    speed_khz: Some(4000.try_into().unwrap()),
                    connect_under_reset: true,
                    ..ProbeArgs::default()
                },
            )
            .unwrap();
        assert_eq!(command.get_program(), OsStr::new("probe-rs"));
        assert_eq!(command.get_current_dir(), Some(fixture.root.as_path()));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("attach"),
                OsStr::new("--chip"),
                OsStr::new("test-chip"),
                OsStr::new("--probe"),
                OsStr::new("1234:5678:serial with spaces"),
                OsStr::new("--speed"),
                OsStr::new("4000"),
                OsStr::new("--connect-under-reset"),
                elf.as_os_str(),
                OsStr::new("--rtt-channel-mode"),
                OsStr::new("no-block-trim"),
                OsStr::new("--no-catch-reset"),
                OsStr::new("--no-catch-hardfault"),
            ]
        );
    }

    #[test]
    fn a_missing_image_reports_the_path_and_matching_build_command() {
        let mut fixture = Fixture::new("missing");
        let monitor = MonitorArgs::default();
        for (name, directory, suffix) in [
            (None, "debug", ""),
            (Some("dev"), "debug", " --profile dev"),
            (Some("release"), "release", " --profile release"),
            (Some("bench-debug"), "bench-debug", " --profile bench-debug"),
        ] {
            fixture.profile = name.map(|name| name.parse().unwrap());
            for action in [
                Action::Run { monitor: &monitor },
                Action::Flash,
                Action::Attach { monitor: &monitor },
            ] {
                let error = fixture.command(action, &ProbeArgs::default()).unwrap_err();
                let expected_path = fixture
                    .root
                    .join("target/firmware/test-platform/thumbv7em-none-eabihf")
                    .join(directory)
                    .join("robot-image");
                assert_eq!(
                    error.to_string(),
                    format!(
                        "ELF not found: {}; build it with polyfw build \
                         test-firmware{suffix}",
                        expected_path.display()
                    )
                );
                assert!(!fixture.root.join("target").exists());
            }
        }
    }

    #[test]
    fn reset_needs_no_firmware_artifact_or_workspace() {
        let fixture = Fixture::new("reset");
        let command = fixture
            .command(Action::Reset, &ProbeArgs::default())
            .unwrap();
        assert_eq!(command.get_program(), OsStr::new("probe-rs"));
        assert_eq!(command.get_current_dir(), Some(fixture.root.as_path()));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("reset"),
                OsStr::new("--chip"),
                OsStr::new("test-chip")
            ]
        );
        assert!(!fixture.root.join("platform").exists());
        assert!(!fixture.root.join("target").exists());
    }

    struct Fixture {
        root: PathBuf,
        profile: Option<Profile>,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "polyfw-probe-{label}-{} with spaces",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let root = root.canonicalize().unwrap();
            Self {
                root,
                profile: None,
            }
        }

        fn elf_path(&self) -> PathBuf {
            let directory = match self.profile.as_ref().map(Profile::name) {
                None | Some("dev") => "debug",
                Some(name) => name,
            };
            self.root
                .join("target/firmware/test-platform/thumbv7em-none-eabihf")
                .join(directory)
                .join("robot-image")
        }

        fn write_elf(&self) -> PathBuf {
            let path = self.elf_path();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "existing ELF").unwrap();
            path
        }

        fn command(&self, action: Action<'_>, options: &ProbeArgs) -> Result<Command> {
            let path = self.elf_path();
            let image = if matches!(action, Action::Reset) {
                None
            } else {
                Some(Image {
                    firmware: "test-firmware",
                    path: &path,
                    profile: self.profile.as_ref(),
                })
            };
            command(&self.root, "test-chip", image, action, options)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
