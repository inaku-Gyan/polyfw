use std::num::NonZeroU32;
use std::path::PathBuf;

use clap::builder::styling::{AnsiColor, Styles};
use clap::{
    Args, ColorChoice, Command, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
};

use crate::ui::color_enabled;
use polyfw::Profile;

/// Build, check, flash and monitor embedded Rust firmware registered in polyfw.toml.
///
/// A firmware ID selects a Cargo package/binary and one Platform. The Platform declares a target
/// and chip. Directory settings inherit from Platform to Firmware. Cargo owns configuration
/// merging, workspace membership, dependencies and profiles.
#[derive(Debug, Parser)]
#[command(
    name = "plfw",
    bin_name = "plfw",
    version,
    after_long_help = "Project and selection:
  Find polyfw.toml in the current directory or an ancestor, or pass --project-root DIR.
  Pass FIRMWARE for one invocation, or omit it to use the saved firmware and profile.
  If no firmware is saved, choose interactively; invalid saved state is an error.
  Use select or --select to save a firmware. Saving keeps only an explicit profile.
  Build/check/clean commands accept --all-firmwares; hardware commands use one firmware.
  Pickers require a terminal. Use FIRMWARE or --all-firmwares in scripts.
  run, flash and attach use an existing image: run build after changing firmware source.

Examples:
  plfw init
  plfw info
  plfw select led-demo --release
  plfw verify --all-firmwares
  plfw build --all-firmwares --release
  plfw probes
  plfw run
  plfw select buzzer-demo --release
  plfw run
  plfw help build",
    styles = Styles::styled()
        .header(AnsiColor::BrightGreen.on_default())
        .usage(AnsiColor::BrightGreen.on_default())
        .literal(AnsiColor::BrightCyan.on_default())
        .placeholder(AnsiColor::Cyan.on_default())
        .context(AnsiColor::BrightBlack.on_default())
        .context_value(AnsiColor::BrightCyan.on_default())
)]
pub(crate) struct Cli {
    /// Operate on the directory containing polyfw.toml; init creates it there.
    ///
    /// Relative paths start at the caller's working directory. Otherwise, search
    /// that directory and its ancestors for the nearest polyfw.toml. An invalid
    /// selected project is an error. For init, use this existing directory or the
    /// current directory without searching ancestors. Help, version, schema and
    /// probes ignore this option.
    #[arg(
        long,
        value_name = "DIR",
        global = true,
        overrides_with = "project_root",
        verbatim_doc_comment
    )]
    pub(crate) project_root: Option<PathBuf>,

    /// Choose automatic colors, always-on colors or plain output.
    #[arg(
        long,
        value_enum,
        default_value_t = ColorChoice::Auto,
        global = true,
        verbatim_doc_comment
    )]
    pub(crate) color: ColorChoice,

    #[command(subcommand)]
    pub(crate) command: Option<Task>,
}

impl Cli {
    pub(crate) fn parse_with_color() -> (Self, Command) {
        let args: Vec<_> = std::env::args_os().collect();
        // Clap exits on help before returning matches, so its color policy must be set first.
        let color = args
            .iter()
            .enumerate()
            .skip(1)
            .take_while(|(_, arg)| *arg != "--")
            .find_map(|(index, arg)| {
                let arg = arg.to_str()?;
                let value = if arg == "--color" {
                    args.get(index + 1)?.to_str()?
                } else {
                    arg.strip_prefix("--color=")?
                };
                ColorChoice::from_str(value, false).ok()
            })
            .unwrap_or(ColorChoice::Auto);
        let mut command = Self::command().color(if color_enabled(color) {
            ColorChoice::Always
        } else {
            ColorChoice::Never
        });
        let matches = command
            .try_get_matches_from_mut(args)
            .unwrap_or_else(|error| error.exit());
        let cli = Self::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
        (cli, command)
    }
}

#[derive(Clone, Debug, Args)]
pub(crate) struct FirmwareArgs {
    /// Firmware ID for this invocation; otherwise use the saved firmware or a picker.
    ///
    /// Use an ID from plfw info. A named or picked firmware is transient unless
    /// --select is passed. The picker opens only when no firmware is saved; stale or
    /// malformed saved state is an error, including with a one-time FIRMWARE override.
    /// Replace it with plfw select FIRMWARE or clear it with plfw select --clear.
    /// Pickers require a terminal; name a firmware in scripts.
    #[arg(verbatim_doc_comment)]
    pub(crate) firmware: Option<String>,
    /// Save the named firmware, or always open the picker and save its choice.
    ///
    /// Save before running the action. Without FIRMWARE, the picker opens even if a
    /// firmware is already saved and requires a terminal. State is stored in
    /// .polyfw-state/selected; saving retains only an explicitly supplied profile.
    #[arg(short, long, verbatim_doc_comment)]
    pub(crate) select: bool,
    /// Operate on every registered firmware, ignoring saved firmware/profile state.
    ///
    /// Include every registered firmware. Conflicts with FIRMWARE and --select.
    #[arg(
        short = 'a',
        long = "all-firmwares",
        conflicts_with_all = ["firmware", "select"],
        verbatim_doc_comment
    )]
    pub(crate) all_firmwares: bool,
}

#[derive(Debug, Default, Args)]
pub(crate) struct ProbeArgs {
    /// Firmware ID; otherwise use the saved firmware or pick one if no firmware is saved.
    pub(crate) firmware: Option<String>,
    /// Save FIRMWARE before acting, or always open the picker and save its choice.
    #[arg(short, long)]
    pub(crate) select: bool,
    /// Use probe VID:PID[:SERIAL] for this session; otherwise let probe-rs choose.
    #[arg(long, value_name = "ID")]
    pub(crate) probe: Option<String>,
    /// Set debug interface speed in kHz for this session; otherwise use probe-rs settings.
    #[arg(long = "speed", value_name = "KHZ")]
    pub(crate) speed_khz: Option<NonZeroU32>,
    /// Assert the target's reset line while connecting.
    #[arg(long)]
    pub(crate) connect_under_reset: bool,
}

#[derive(Debug, Default, Args)]
pub(crate) struct ProfileArgs {
    /// Choose the dev profile explicitly; equivalent to --profile dev.
    #[arg(long, conflicts_with_all = ["release", "profile"])]
    pub(crate) dev: bool,
    /// Choose the release profile; equivalent to --profile release.
    #[arg(short = 'r', long, conflicts_with = "profile")]
    pub(crate) release: bool,
    /// Use this Cargo profile instead of the saved profile.
    ///
    /// Single-firmware actions otherwise inherit the saved profile. Saving a selection
    /// and --all-firmwares use only an explicit profile. Without a resolved profile,
    /// omit the Cargo flag and use its default.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<Profile>,
}

impl ProfileArgs {
    pub(crate) fn explicit(&self) -> Option<Profile> {
        if self.release {
            Some("release".parse().expect("release is a valid profile name"))
        } else if self.dev {
            Some("dev".parse().expect("dev is a valid profile name"))
        } else {
            self.profile.clone()
        }
    }
}

#[derive(Debug, Default, Args)]
pub(crate) struct MonitorArgs {
    /// Choose RTT buffer behavior; the default drops whole messages when full.
    #[arg(long, value_enum, default_value_t = RttChannelMode::NoBlockSkip)]
    pub(crate) rtt_channel_mode: RttChannelMode,
    /// Disable probe-rs capture of target resets.
    #[arg(long)]
    pub(crate) no_catch_reset: bool,
    /// Disable probe-rs capture of HardFault exceptions.
    #[arg(long)]
    pub(crate) no_catch_hardfault: bool,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum RttChannelMode {
    #[default]
    NoBlockSkip,
    NoBlockTrim,
    BlockIfFull,
}

impl RttChannelMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoBlockSkip => "no-block-skip",
            Self::NoBlockTrim => "no-block-trim",
            Self::BlockIfFull => "block-if-full",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InfoJsonVersion {
    #[value(name = "2")]
    V2,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Task {
    /// Create a polyfw.toml template and local Git ignore rules.
    ///
    /// Write in the current directory, or the existing directory named by
    /// --project-root, without searching ancestors. Refuse to overwrite any existing
    /// polyfw.toml. Create .polyfw-state/.gitignore with a single * pattern, or use
    /// a regular file that already exists there. Print the paths created.
    /// Edit the TODO values before running plfw info to validate the project.
    /// Runs without project configuration, external tools, network access or prompts.
    #[command(
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw init
  plfw --project-root /path/to/firmware-demos init"
    )]
    Init,

    /// Print or write JSON Schema for this version's polyfw.toml.
    ///
    /// Its $id points to the exact package version's Git tag, including prerelease suffixes.
    /// Works without a project, Rust installation or hardware tools.
    Schema {
        /// Write to FILE instead of stdout; relative paths start at the current directory.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },

    /// Inspect registered firmwares, existing images and the saved selection.
    ///
    /// List firmwares and their referenced platforms, with dev/release ELF presence and any
    /// different saved profile. --firmware [FIRMWARE] adds one firmware's resolved configuration,
    /// ELF paths and local check dependencies. Omit FIRMWARE after the flag to pick one
    /// without saving. A stale saved firmware is reported; malformed state is an error.
    /// This query does not build, write state/editor files or connect to hardware.
    /// ELF presence establishes neither source freshness nor a match with running firmware.
    #[command(
        visible_alias = "i",
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw info
  plfw info --firmware
  plfw info --firmware led-demo
  plfw info --json=2
  plfw info --firmware buzzer-demo --json=2"
    )]
    Info {
        /// Show one firmware's details; omit FIRMWARE to pick interactively without saving.
        #[arg(long, value_name = "FIRMWARE")]
        firmware: Option<Option<String>>,
        /// Write the selected view as JSON to stdout.
        ///
        /// Bare --json uses format 2. Pin a numbered format in scripts.
        /// Interactive prompts go to stderr.
        #[arg(long, value_enum, value_name = "VERSION", num_args = 0..=1, default_missing_value = "2")]
        json: Option<InfoJsonVersion>,
    },

    /// Save a firmware and explicit profile, or clear the saved selection.
    ///
    /// Save FIRMWARE in .polyfw-state/selected for later commands. Saving without a
    /// profile clears the previous profile. With no FIRMWARE, always open the picker;
    /// this requires a terminal. When editor generation is enabled, saving also replaces
    /// the whole VS Code launch file.
    /// --clear removes saved state and the managed launch file without needing Cargo.
    #[command(
        visible_alias = "s",
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw select led-demo --release
  plfw select buzzer-demo --release
  plfw select
  plfw select --clear"
    )]
    Select {
        /// Save this firmware ID; otherwise open the picker.
        firmware: Option<String>,
        /// Remove saved selection and the managed VS Code launch file.
        ///
        /// Does not parse the old state or load Cargo workspaces. Editor files are
        /// untouched when generation is disabled. Conflicts with FIRMWARE and profile flags.
        #[arg(
            long,
            conflicts_with_all = ["firmware", "dev", "release", "profile"],
            verbatim_doc_comment
        )]
        clear: bool,
        #[command(flatten)]
        profile: ProfileArgs,
    },

    /// Clean a firmware package or the registered Cargo output directories.
    ///
    /// One firmware uses Cargo package cleaning for its target and the host target, with
    /// Cargo's default profile rules. --all-firmwares performs full cleaning per directory
    /// context, including dependencies and outputs from unregistered/manual Cargo builds
    /// that share those directories. The notice lists registered firmware sharing them.
    #[command(
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw clean led-demo
  plfw clean --all-firmwares"
    )]
    Clean(FirmwareArgs),

    /// Build the selected firmware or every registered firmware with Cargo.
    ///
    /// Build each firmware from its resolved working directory with --locked and the
    /// explicit Platform target. An explicit bin adds --bin. Cargo picks default targets
    /// when bin is omitted. Output directories inherit from the Platform. Cargo resolves
    /// compiler flags from its configuration and environment.
    /// Inherit a saved profile for one firmware, or override it with a profile flag.
    /// --all-firmwares uses only the explicitly supplied profile.
    #[command(
        visible_alias = "b",
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw build
  plfw build --select led-demo --release
  plfw build buzzer-demo --release
  plfw build --all-firmwares --release"
    )]
    Build {
        #[command(flatten)]
        firmwares: FirmwareArgs,
        #[command(flatten)]
        profile: ProfileArgs,
    },

    /// List connected probes using probe-rs; no project or Rust tools required.
    Probes,

    /// Flash and verify an existing ELF, then start and monitor RTT/defmt.
    ///
    /// Use the resolved firmware/profile image without building. Run plfw build after
    /// source changes. Omitted probe and speed options are delegated to probe-rs.
    /// Ctrl-C ends the probe-rs session; firmware may continue running.
    #[command(
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw run led-demo
  plfw run buzzer-demo --release
  plfw run --release
  plfw run led-demo --probe VID:PID:SERIAL --speed 4000"
    )]
    Run {
        #[command(flatten)]
        target: ProbeArgs,
        #[command(flatten)]
        profile: ProfileArgs,
        #[command(flatten)]
        monitor: MonitorArgs,
    },

    /// Flash and verify an existing ELF, reset the firmware, then exit.
    ///
    /// Use the resolved firmware/profile image without building. Run plfw build after
    /// source changes. This command does not start an RTT monitor.
    #[command(
        visible_aliases = ["f", "download", "d"],
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw flash led-demo --release
  plfw flash --select buzzer-demo --release"
    )]
    Flash {
        #[command(flatten)]
        target: ProbeArgs,
        #[command(flatten)]
        profile: ProfileArgs,
    },

    /// Attach an RTT/defmt monitor using the matching existing ELF.
    ///
    /// Does not build or flash. The resolved firmware/profile ELF must match the image
    /// already running. Connecting can briefly halt the core; --connect-under-reset
    /// asserts the reset line.
    #[command(
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw attach led-demo
  plfw attach buzzer-demo --release
  plfw attach --release --no-catch-reset"
    )]
    Attach {
        #[command(flatten)]
        target: ProbeArgs,
        #[command(flatten)]
        profile: ProfileArgs,
        #[command(flatten)]
        monitor: MonitorArgs,
    },

    /// Reset the selected board without building, flashing or requiring an ELF.
    ///
    /// Needs project configuration and probe-rs. Saving with --select also needs Cargo
    /// when VS Code generation is enabled.
    Reset(ProbeArgs),

    /// Rewrite formatting for firmware and selected local dependencies.
    ///
    /// Run cargo fmt in place for the firmware and reachable path packages matched
    /// by config.local-dependencies. Include whole packages, including other binaries,
    /// once per execution context. Use verify to check formatting without rewriting.
    #[command(
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw fmt led-demo
  plfw fmt buzzer-demo
  plfw fmt --all-firmwares"
    )]
    Fmt(FirmwareArgs),

    /// Check firmware and selected local dependencies with MCU and host Clippy.
    ///
    /// Deny warnings. Check runtime libraries and binaries for the MCU, then selected
    /// libraries and tests for the host. config.local-dependencies filters reachable
    /// path packages, including local test helpers. Other binaries in a package are
    /// included. Preserve each resolved working/output directory context. Host checks use
    /// host-tuple. Target checks use the Platform target. Cargo resolves compiler flags for both.
    #[command(
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw lint led-demo
  plfw lint buzzer-demo
  plfw lint --all-firmwares"
    )]
    Lint(FirmwareArgs),

    /// Run host tests and doctests for firmware and selected local dependencies.
    ///
    /// No hardware is required. Run packages in their resolved host execution contexts
    /// with --target host-tuple and native dev-dependencies. Use Cargo's
    /// default test targets, including other binaries when their test settings allow.
    /// config.local-dependencies filters reachable path packages and local test helpers.
    #[command(
        visible_alias = "t",
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw test led-demo
  plfw test buzzer-demo
  plfw test --all-firmwares"
    )]
    Test(FirmwareArgs),

    /// Check formatting, run MCU/host Clippy, then run host tests.
    ///
    /// Check without rewriting source, deny lint warnings, and stop at the first
    /// failure. Include selected firmware and reachable path packages matched by
    /// config.local-dependencies across workspaces. --all-firmwares checks every firmware.
    /// Does not build a firmware image. Cargo may need network access for dependencies.
    #[command(
        visible_alias = "v",
        verbatim_doc_comment,
        after_long_help = "Examples:
  plfw verify
  plfw verify led-demo
  plfw verify buzzer-demo
  plfw verify --all-firmwares"
    )]
    Verify(FirmwareArgs),
}

#[cfg(test)]
impl Task {
    pub(crate) fn profile_override(&self) -> Option<Profile> {
        match self {
            Self::Select { profile, .. }
            | Self::Build { profile, .. }
            | Self::Run { profile, .. }
            | Self::Flash { profile, .. }
            | Self::Attach { profile, .. } => profile.explicit(),
            Self::Init
            | Self::Schema { .. }
            | Self::Info { .. }
            | Self::Clean(_)
            | Self::Probes
            | Self::Reset(_)
            | Self::Fmt(_)
            | Self::Lint(_)
            | Self::Test(_)
            | Self::Verify(_) => None,
        }
    }

    pub(crate) fn firmware_args(&self) -> Option<FirmwareArgs> {
        match self {
            Self::Init
            | Self::Schema { .. }
            | Self::Info { .. }
            | Self::Select { .. }
            | Self::Probes => None,
            Self::Clean(firmwares)
            | Self::Build { firmwares, .. }
            | Self::Fmt(firmwares)
            | Self::Lint(firmwares)
            | Self::Test(firmwares)
            | Self::Verify(firmwares) => Some(firmwares.clone()),
            Self::Run { target, .. }
            | Self::Flash { target, .. }
            | Self::Attach { target, .. }
            | Self::Reset(target) => Some(FirmwareArgs {
                firmware: target.firmware.clone(),
                select: target.select,
                all_firmwares: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn the_project_root_is_accepted_before_or_after_every_command() {
        for command in Cli::command().get_subcommands() {
            for args in [
                vec![
                    "plfw",
                    "--project-root",
                    "../another project",
                    command.get_name(),
                ],
                vec![
                    "plfw",
                    command.get_name(),
                    "--project-root",
                    "../another project",
                ],
            ] {
                let cli = Cli::try_parse_from(args).unwrap();
                assert_eq!(cli.project_root, Some(PathBuf::from("../another project")));
            }
        }
        assert!(
            Cli::try_parse_from(["plfw", "info"])
                .unwrap()
                .project_root
                .is_none()
        );
        assert!(Cli::try_parse_from(["plfw", "info", "--project-root"]).is_err());
    }

    #[test]
    fn info_distinguishes_an_overview_a_picker_and_a_named_firmware() {
        use InfoJsonVersion::V2;

        for command in ["info", "i"] {
            for (flags, expected_firmware, expected_json) in [
                (vec![], None, None),
                (vec!["--json"], None, Some(V2)),
                (vec!["--firmware"], None, None),
                (vec!["--firmware", "--json"], None, Some(V2)),
                (vec!["--json", "--firmware"], None, Some(V2)),
                (vec!["--firmware", "--json=2"], None, Some(V2)),
                (vec!["--json", "2", "--firmware"], None, Some(V2)),
                (
                    vec!["--firmware", "sentry-chassis"],
                    Some("sentry-chassis"),
                    None,
                ),
                (
                    vec!["--firmware", "sentry-chassis", "--json=2"],
                    Some("sentry-chassis"),
                    Some(V2),
                ),
                (
                    vec!["--json", "2", "--firmware", "sentry-chassis"],
                    Some("sentry-chassis"),
                    Some(V2),
                ),
            ] {
                let has_firmware_flag = flags.contains(&"--firmware");
                let mut args = vec!["plfw", command];
                args.extend(flags);
                let task = Cli::try_parse_from(args).unwrap().command.unwrap();
                assert!(task.firmware_args().is_none());
                assert!(task.profile_override().is_none());
                let Task::Info { firmware, json } = task else {
                    panic!("expected an info query");
                };
                assert_eq!(firmware.is_some(), has_firmware_flag);
                assert_eq!(
                    firmware.as_ref().and_then(|firmware| firmware.as_deref()),
                    expected_firmware
                );
                assert_eq!(json, expected_json);
            }
        }
        for flags in [
            vec!["sentry-chassis"],
            vec!["--profile", "release"],
            vec!["-r"],
            vec!["--release"],
            vec!["--dev"],
            vec!["--select"],
            vec!["--all-firmwares"],
        ] {
            let mut args = vec!["plfw", "info"];
            args.extend(flags);
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn image_commands_accept_cargo_profile_flags_and_reject_conflicts_and_path_names() {
        for command in ["select", "build", "run", "flash", "attach"] {
            let cli = Cli::try_parse_from(["plfw", command]).unwrap();
            assert!(cli.command.unwrap().profile_override().is_none());
            for flag in ["-r", "--release"] {
                let cli = Cli::try_parse_from(["plfw", command, flag]).unwrap();
                assert_eq!(
                    cli.command.unwrap().profile_override().unwrap().name(),
                    "release"
                );
                let error = Cli::try_parse_from(["plfw", command, flag, "--profile", "release"])
                    .unwrap_err();
                assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
            let cli = Cli::try_parse_from(["plfw", command, "--dev"]).unwrap();
            assert_eq!(
                cli.command.unwrap().profile_override().unwrap().name(),
                "dev"
            );
            for flags in [
                vec!["-r"],
                vec!["--release"],
                vec!["--profile", "dev"],
                vec!["--profile", "release"],
                vec!["--profile", "bench-debug"],
            ] {
                let mut args = vec!["plfw", command, "--dev"];
                args.extend(flags);
                let error = Cli::try_parse_from(args).unwrap_err();
                assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
            let cli = Cli::try_parse_from(["plfw", command, "--profile", "bench-debug"]).unwrap();
            assert_eq!(
                cli.command.unwrap().profile_override().unwrap().name(),
                "bench-debug"
            );
            assert!(Cli::try_parse_from(["plfw", command, "--profile", "../image"]).is_err());
        }
        for flags in [vec!["-r"], vec!["--dev"], vec!["--profile", "dev"]] {
            let mut args = vec!["plfw", "select", "--clear"];
            args.extend(flags);
            let error = Cli::try_parse_from(args).unwrap_err();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn firmware_commands_accept_select_with_or_without_a_firmware() {
        for command in ["build", "clean", "fmt", "lint", "test", "verify"] {
            for flag in ["-s", "--select"] {
                let cli = Cli::try_parse_from(["plfw", command, flag, "sentry-chassis"])
                    .expect("select with a firmware must parse");
                let firmwares = cli.command.as_ref().unwrap().firmware_args().unwrap();
                assert!(firmwares.select);
                assert_eq!(firmwares.firmware.as_deref(), Some("sentry-chassis"));

                let cli = Cli::try_parse_from(["plfw", command, flag])
                    .expect("select without a firmware must parse");
                let firmwares = cli.command.as_ref().unwrap().firmware_args().unwrap();
                assert!(firmwares.select);
                assert!(firmwares.firmware.is_none());
            }
        }
    }

    #[test]
    fn all_firmwares_conflicts_with_a_firmware_and_select() {
        for command in ["build", "clean", "fmt", "lint", "test", "verify"] {
            for argument in ["--select", "sentry-chassis"] {
                let error = Cli::try_parse_from(["plfw", command, "--all-firmwares", argument])
                    .expect_err("all firmwares must reject single-firmware options");
                assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
        }
    }

    #[test]
    fn hardware_commands_select_one_firmware_and_reject_all_firmwares() {
        for command in ["run", "flash", "attach", "reset"] {
            let cli = Cli::try_parse_from(["plfw", command, "--select", "sentry-chassis"]).unwrap();
            let firmwares = cli.command.as_ref().unwrap().firmware_args().unwrap();
            assert!(firmwares.select);
            assert_eq!(firmwares.firmware.as_deref(), Some("sentry-chassis"));
            assert!(!firmwares.all_firmwares);
            assert!(Cli::try_parse_from(["plfw", command, "--all-firmwares"]).is_err());
            assert!(Cli::try_parse_from(["plfw", command, "--speed", "0"]).is_err());
        }
        for argument in ["sentry-chassis", "--select", "--probe", "--speed"] {
            assert!(Cli::try_parse_from(["plfw", "probes", argument]).is_err());
        }
        assert!(Cli::try_parse_from(["plfw", "reset", "--release"]).is_err());
    }

    #[test]
    fn flash_aliases_and_reset_preserve_their_options() {
        for (command, flash) in [
            ("f", true),
            ("download", true),
            ("d", true),
            ("reset", false),
        ] {
            let mut args = vec![
                "plfw",
                command,
                "sentry-chassis",
                "--select",
                "--probe",
                "1234:5678:serial",
                "--speed",
                "4000",
                "--connect-under-reset",
            ];
            if flash {
                args.push("--release");
            }
            let cli = Cli::try_parse_from(args).unwrap();
            let target = match cli.command.unwrap() {
                Task::Flash { target, profile } => {
                    assert!(flash);
                    assert_eq!(profile.explicit().unwrap().name(), "release");
                    target
                }
                Task::Reset(target) => {
                    assert!(!flash);
                    target
                }
                Task::Init
                | Task::Schema { .. }
                | Task::Info { .. }
                | Task::Select { .. }
                | Task::Clean(_)
                | Task::Build { .. }
                | Task::Probes
                | Task::Run { .. }
                | Task::Attach { .. }
                | Task::Fmt(_)
                | Task::Lint(_)
                | Task::Test(_)
                | Task::Verify(_) => panic!("unexpected command for {command}"),
            };
            assert_eq!(target.firmware.as_deref(), Some("sentry-chassis"));
            assert!(target.select);
            assert_eq!(target.probe.as_deref(), Some("1234:5678:serial"));
            assert_eq!(target.speed_khz.unwrap().get(), 4000);
            assert!(target.connect_under_reset);
        }
    }

    #[test]
    fn run_and_attach_default_to_nonblocking_logs_and_accept_native_monitor_controls() {
        for command in ["run", "attach"] {
            let cli = Cli::try_parse_from(["plfw", command]).unwrap();
            let (target, profile, monitor) = monitor_task(cli.command.unwrap());
            assert!(profile.explicit().is_none());
            assert!(target.probe.is_none());
            assert!(target.speed_khz.is_none());
            assert_eq!(monitor.rtt_channel_mode.as_str(), "no-block-skip");
            assert!(!monitor.no_catch_reset);
            assert!(!monitor.no_catch_hardfault);

            let cli = Cli::try_parse_from([
                "plfw",
                command,
                "--release",
                "--probe",
                "1234:5678:serial",
                "--speed",
                "4000",
                "--connect-under-reset",
                "--rtt-channel-mode",
                "block-if-full",
                "--no-catch-reset",
                "--no-catch-hardfault",
            ])
            .unwrap();
            let (target, profile, monitor) = monitor_task(cli.command.unwrap());
            assert_eq!(profile.explicit().unwrap().name(), "release");
            assert_eq!(target.probe.as_deref(), Some("1234:5678:serial"));
            assert_eq!(target.speed_khz.unwrap().get(), 4000);
            assert!(target.connect_under_reset);
            assert_eq!(monitor.rtt_channel_mode.as_str(), "block-if-full");
            assert!(monitor.no_catch_reset);
            assert!(monitor.no_catch_hardfault);
        }
        for command in ["flash", "reset"] {
            assert!(Cli::try_parse_from(["plfw", command, "--no-catch-reset"]).is_err());
        }
    }

    fn monitor_task(task: Task) -> (ProbeArgs, ProfileArgs, MonitorArgs) {
        match task {
            Task::Run {
                target,
                profile,
                monitor,
            }
            | Task::Attach {
                target,
                profile,
                monitor,
            } => (target, profile, monitor),
            Task::Init
            | Task::Schema { .. }
            | Task::Info { .. }
            | Task::Select { .. }
            | Task::Clean(_)
            | Task::Build { .. }
            | Task::Probes
            | Task::Flash { .. }
            | Task::Reset(_)
            | Task::Fmt(_)
            | Task::Lint(_)
            | Task::Test(_)
            | Task::Verify(_) => panic!("expected a monitor command"),
        }
    }
}
