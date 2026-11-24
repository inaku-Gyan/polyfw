use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::{Error, ErrorKind, Profile, Result};

/// How a Cargo operation handles its child processes' terminal streams.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputMode {
    /// Buffer user-facing stdout and stderr for the result, with child stdin closed.
    #[default]
    Capture,
    /// Forward child output to the caller and let the child use the caller's stdin.
    Inherit,
}

/// The work performed by one Cargo stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StageKind {
    Build,
    Clean,
    Format,
    FormatCheck,
    Lint,
    Test,
}

/// The compilation role of a stage that targets firmware or host packages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageTarget {
    Embedded,
    Host,
}

/// A Cargo invocation that is starting or finishing.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Stage<'a> {
    /// One-based position in this operation.
    pub index: usize,
    pub total: usize,
    pub kind: StageKind,
    pub target: Option<StageTarget>,
    /// Firmware IDs for build and clean, package names for other stages.
    pub subjects: &'a [String],
    pub profile: Option<&'a Profile>,
    /// The rendered command line for display.
    pub command: &'a str,
}

/// The directories and registered firmware affected by a Cargo clean stage.
#[derive(Debug)]
#[non_exhaustive]
pub struct CleanScope {
    pub target_dir: PathBuf,
    /// `None` means the entire directory is being cleaned.
    pub package: Option<String>,
    pub firmwares: Vec<String>,
}

/// A synchronous notification from a Cargo operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProgressEvent<'a> {
    Cleaning(&'a CleanScope),
    Started(Stage<'a>),
    Finished {
        stage: Stage<'a>,
        elapsed: Duration,
        success: bool,
    },
}

/// Exit status and optional captured streams for a Cargo stage.
#[derive(Debug)]
#[non_exhaustive]
pub struct StageOutput {
    pub kind: StageKind,
    pub command: String,
    pub status: ExitStatus,
    pub stdout: Option<Vec<u8>>,
    pub stderr: Option<Vec<u8>>,
}

/// The completed stages in execution order.
#[derive(Debug)]
#[non_exhaustive]
pub struct RunReport {
    pub stages: Vec<StageOutput>,
}

pub(crate) struct Invocation {
    pub(crate) kind: StageKind,
    pub(crate) target: Option<StageTarget>,
    pub(crate) subjects: Vec<String>,
    pub(crate) profile: Option<Profile>,
    pub(crate) clean: Option<CleanScope>,
    pub(crate) command: Command,
}

impl Invocation {
    pub(crate) fn new(
        kind: StageKind,
        target: Option<StageTarget>,
        subjects: Vec<String>,
        command: Command,
    ) -> Self {
        Self {
            kind,
            target,
            subjects,
            profile: None,
            clean: None,
            command,
        }
    }
}

fn render_command(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

type OutputInspector<'a> = dyn FnMut(usize, &mut StageOutput) -> Result<()> + 'a;

pub(crate) fn run(
    invocations: Vec<Invocation>,
    mode: OutputMode,
    mut progress: Option<&mut dyn FnMut(ProgressEvent<'_>)>,
    mut inspect: Option<&mut OutputInspector<'_>>,
) -> Result<RunReport> {
    let total = invocations.len();
    let mut stages = Vec::with_capacity(total);
    for (index, mut invocation) in invocations.into_iter().enumerate() {
        let rendered = render_command(&invocation.command);
        let stage = Stage {
            index: index + 1,
            total,
            kind: invocation.kind,
            target: invocation.target,
            subjects: &invocation.subjects,
            profile: invocation.profile.as_ref(),
            command: &rendered,
        };
        if let Some(observer) = progress.as_deref_mut() {
            if let Some(clean) = &invocation.clean {
                observer(ProgressEvent::Cleaning(clean));
            }
            observer(ProgressEvent::Started(stage));
        }
        let started = Instant::now();
        let output = match (mode, inspect.is_some()) {
            (OutputMode::Capture, _) => invocation
                .command
                .output()
                .map(|output| (output.status, Some(output.stdout), Some(output.stderr))),
            (OutputMode::Inherit, true) => invocation
                .command
                .stdin(Stdio::inherit())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .output()
                .map(|output| (output.status, Some(output.stdout), None)),
            (OutputMode::Inherit, false) => invocation
                .command
                .status()
                .map(|status| (status, None, None)),
        };
        let output = output
            .map_err(|error| Error::new(ErrorKind::ProcessLaunch, error))
            .and_then(|(status, stdout, stderr)| {
                let mut output = StageOutput {
                    kind: invocation.kind,
                    command: rendered.clone(),
                    status,
                    stdout,
                    stderr,
                };
                let inspection = inspect
                    .as_mut()
                    .map_or(Ok(()), |inspect| inspect(index, &mut output));
                let forwarding = if mode == OutputMode::Inherit
                    && let Some(stdout) = output.stdout.take()
                {
                    io::stdout()
                        .lock()
                        .write_all(&stdout)
                        .map_err(|error| Error::new(ErrorKind::Cargo, error))
                } else {
                    Ok(())
                };
                if !status.success() {
                    return Err(Error::process_failed(output));
                }
                match inspection.and(forwarding) {
                    Ok(()) => Ok(output),
                    Err(error) => Err(error.with_stage(output)),
                }
            });
        if let Some(observer) = progress.as_deref_mut() {
            observer(ProgressEvent::Finished {
                stage,
                elapsed: started.elapsed(),
                success: output.is_ok(),
            });
        }
        stages.push(output?);
    }
    Ok(RunReport { stages })
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io;

    use super::*;

    #[test]
    fn a_failed_launch_retains_the_os_error_and_finishes_progress() {
        let missing =
            std::env::temp_dir().join(format!("polyfw-missing-command-{}", std::process::id()));
        for mode in [OutputMode::Capture, OutputMode::Inherit] {
            let command = Command::new(&missing);
            let invocation = Invocation::new(StageKind::Build, None, vec![], command);
            let mut finished = None;
            let mut progress = |event: ProgressEvent<'_>| {
                if let ProgressEvent::Finished { success, .. } = event {
                    finished = Some(success);
                }
            };
            let error = run(vec![invocation], mode, Some(&mut progress), None).unwrap_err();
            assert_eq!(finished, Some(false));
            assert_eq!(error.kind(), ErrorKind::ProcessLaunch);
            let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
            assert_eq!(source.kind(), io::ErrorKind::NotFound);
            assert!(source.raw_os_error().is_some());
        }
    }
}
