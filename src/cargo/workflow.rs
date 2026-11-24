use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::config::model::Registry;
use crate::process::{self, Invocation};
use crate::{
    CleanScope, Error, ErrorKind, OutputMode, ProgressEvent, Result, RunReport, StageKind,
    StageTarget,
};

use super::model::{FirmwareCargo, MetadataTarget, Selection};
use super::runner;
use super::scope::{self, PackageScope, WorkspaceScope};

/// A Cargo checking workflow. Verification stops at the first failed stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Check {
    /// Format the selected sources in place.
    Format,
    /// Run target and host Clippy with warnings denied.
    Lint,
    /// Run host tests and doctests.
    Test,
    /// Check formatting, then run Clippy and tests.
    Verify,
}

pub(crate) fn build(
    selections: &[Selection],
    output: OutputMode,
    progress: Option<&mut dyn FnMut(ProgressEvent<'_>)>,
) -> Result<(RunReport, Vec<PathBuf>)> {
    let mut executables = Vec::with_capacity(selections.len());
    let report = process::run(
        build_stages(selections),
        output,
        progress,
        Some(&mut |index, stage| {
            let selection = &selections[index];
            let stdout = stage.stdout.as_mut().expect("build stdout is piped");
            let executable = executable_from_output(selection, stdout);
            if stage.status.success() {
                executables.push(executable.ok_or_else(|| missing_executable(selection))?);
            }
            Ok(())
        }),
    )?;
    Ok((report, executables))
}

pub(crate) fn build_stages(selections: &[Selection]) -> Vec<Invocation> {
    selections
        .iter()
        .map(|selection| {
            let mut invocation = Invocation::new(
                StageKind::Build,
                Some(StageTarget::Embedded),
                vec![selection.id.clone()],
                build_command(selection),
            );
            invocation.profile = selection.profile.clone();
            invocation
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum BuildMessage {
    CompilerArtifact {
        package_id: String,
        target: MetadataTarget,
        executable: Option<PathBuf>,
    },
    CompilerMessage {
        message: BuildDiagnostic,
    },
    BuildScriptExecuted,
    BuildFinished,
}

#[derive(Deserialize)]
struct BuildDiagnostic {
    rendered: Option<String>,
}

fn executable_from_output(selection: &Selection, stdout: &mut Vec<u8>) -> Option<PathBuf> {
    let bin = &selection.resolved_bin;
    let mut executable = None;
    let mut visible_stdout = Vec::new();
    for line in stdout.split_inclusive(|byte| *byte == b'\n') {
        match serde_json::from_slice(line) {
            Ok(BuildMessage::CompilerArtifact {
                package_id,
                target,
                executable: Some(path),
            }) if package_id == bin.package_id
                && target.name == bin.name
                && target.kind.iter().any(|kind| kind == "bin") =>
            {
                executable = Some(path);
            }
            Ok(BuildMessage::CompilerMessage { message }) => {
                if let Some(rendered) = message.rendered {
                    visible_stdout.extend(rendered.bytes());
                }
            }
            Ok(_) => {}
            Err(_) => visible_stdout.extend_from_slice(line),
        }
    }
    *stdout = visible_stdout;
    executable
}

fn missing_executable(selection: &Selection) -> Error {
    let bin = &selection.resolved_bin;
    let mut message = format!(
        "Cargo did not report an executable for binary `{}` of firmware `{}`",
        bin.name, selection.id
    );
    if !bin.required_features.is_empty() {
        message.push_str(&format!(
            ". Enable the required Cargo features ({})",
            bin.required_features.join(", ")
        ));
    }
    Error::new(ErrorKind::Cargo, message)
}

pub(crate) fn clean(
    registry: &Registry,
    cargo: &BTreeMap<String, FirmwareCargo>,
    selections: &[Selection],
    all_firmwares: bool,
) -> Vec<Invocation> {
    let mut seen = BTreeSet::new();
    selections
        .iter()
        .filter(|selection| {
            !all_firmwares
                || seen.insert((
                    &selection.cargo.working_dir,
                    &selection.cargo.target_dir,
                    &selection.cargo.build_dir,
                ))
        })
        .map(|selection| {
            let context = &selection.cargo;
            let firmwares = cargo
                .iter()
                .filter(|(id, entry)| {
                    (entry.cargo.target_dir == context.target_dir
                        || context
                            .build_dir
                            .as_ref()
                            .is_some_and(|dir| entry.cargo.build_dir.as_ref() == Some(dir)))
                        && (all_firmwares
                            || registry.firmwares[*id].package == selection.firmware.package)
                })
                .map(|(id, _)| id.clone())
                .collect();
            let mut invocation = Invocation::new(
                StageKind::Clean,
                None,
                vec![selection.id.clone()],
                clean_command(selection, all_firmwares),
            );
            invocation.clean = Some(CleanScope {
                target_dir: context.target_dir.clone(),
                package: (!all_firmwares).then(|| selection.firmware.package.clone()),
                firmwares,
            });
            invocation
        })
        .collect()
}

pub(crate) fn check(
    root: &Path,
    patterns: &[String],
    selections: &[Selection],
    check: Check,
) -> Result<Vec<Invocation>> {
    let scopes = scope::resolve(root, patterns, selections)
        .map_err(|error| Error::new(ErrorKind::Cargo, error))?;
    let stages = match check {
        Check::Format => fmt_stages(&scopes, false),
        Check::Lint => lint_stages(&scopes),
        Check::Test => test_stages(&scopes),
        Check::Verify => {
            let mut stages = fmt_stages(&scopes, true);
            stages.extend(lint_stages(&scopes));
            stages.extend(test_stages(&scopes));
            stages
        }
    };
    Ok(stages)
}

pub(crate) fn clean_command(selection: &Selection, all_firmwares: bool) -> Command {
    let mut command = runner::cargo_command(&selection.cargo);
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

pub(crate) fn build_command(selection: &Selection) -> Command {
    let mut command = runner::target_cargo(&selection.cargo, "build", &selection.platform.target);
    command.arg("-p").arg(&selection.firmware.package);
    if let Some(bin) = &selection.firmware.bin {
        command.arg("--bin").arg(bin);
    }
    command.args(["--locked", "--message-format=json-render-diagnostics"]);
    if let Some(profile) = &selection.profile {
        command.arg("--profile").arg(profile.name());
    }
    command
}

pub(crate) fn fmt_stages(scopes: &[WorkspaceScope], check: bool) -> Vec<Invocation> {
    let mut stages = Vec::new();
    for scope in scope::host_scopes(scopes) {
        let mut command = runner::cargo_command(&scope.cargo);
        command.arg("fmt");
        let names = add_packages(&mut command, scope.packages, |package| &package.name);
        let kind = if check {
            command.arg("--check");
            StageKind::FormatCheck
        } else {
            StageKind::Format
        };
        stages.push(Invocation::new(kind, None, names, command));
    }
    stages
}

pub(crate) fn lint_stages(scopes: &[WorkspaceScope]) -> Vec<Invocation> {
    let mut stages = Vec::new();
    for scope in scopes {
        if scope.packages.is_empty() {
            continue;
        }
        let mut command = runner::target_cargo(&scope.cargo, "clippy", &scope.target);
        let names = add_packages(
            &mut command,
            scope.packages.iter().filter(|package| package.embedded),
            |package| &package.id,
        );
        if !names.is_empty() {
            command
                .args(["--lib", "--bins"])
                .args(["--locked", "--", "-D", "warnings"]);
            stages.push(Invocation::new(
                StageKind::Lint,
                Some(StageTarget::Embedded),
                names,
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
        stages.push(Invocation::new(
            StageKind::Lint,
            Some(StageTarget::Host),
            names,
            host,
        ));
    }
    stages
}

pub(crate) fn test_stages(scopes: &[WorkspaceScope]) -> Vec<Invocation> {
    let mut stages = Vec::new();
    for scope in scope::host_scopes(scopes) {
        let mut command = runner::cargo_command(&scope.cargo);
        command.arg("test");
        let names = add_packages(&mut command, scope.packages, |package| &package.host_id);
        command.args(["--target", "host-tuple", "--locked"]);
        stages.push(Invocation::new(
            StageKind::Test,
            Some(StageTarget::Host),
            names,
            command,
        ));
    }
    stages
}

fn add_packages<'a>(
    command: &mut Command,
    packages: impl IntoIterator<Item = &'a PackageScope>,
    spec: fn(&PackageScope) -> &str,
) -> Vec<String> {
    packages
        .into_iter()
        .map(|package| {
            command.arg("-p").arg(spec(package));
            package.name.clone()
        })
        .collect()
}
