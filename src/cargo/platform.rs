use std::collections::BTreeMap;
use std::fs;

use cargo_config2::{Config, ResolveOptions, Walk};

use super::model::Selection;
use crate::InternalResult as Result;

use super::runner;

/// Check only supported, determinable Cargo configuration. Native Cargo always owns execution.
pub(crate) fn check(selection: &Selection) -> Result<()> {
    if !selection
        .firmware
        .check_platform
        .or(selection.platform.check_platform)
        .unwrap_or(true)
    {
        return Ok(());
    }
    check_inner(selection).map_err(|error| format!(
        "firmware {} platform {} consistency check failed: {error}; adjust Cargo settings or set check-platform = false to skip checking",
        selection.id, selection.firmware.platform
    ).into())
}

fn check_inner(selection: &Selection) -> Result<()> {
    let cwd = &selection.cargo.working_dir;
    // These sources cannot be resolved faithfully by the pinned reader. Reject them before
    // resolution so cfg queries cannot create children outside the shared runner boundary.
    for path in Walk::new(cwd) {
        let document: toml::Table = toml::from_str(&fs::read_to_string(&path)?)?;
        if document.contains_key("include") {
            return Err(format!(
                "cannot reliably verify configuration includes in {}",
                path.display()
            )
            .into());
        }
        if document
            .get("target")
            .and_then(toml::Value::as_table)
            .is_some_and(|targets| {
                targets
                    .keys()
                    .any(|name| name.trim_start().starts_with("cfg("))
            })
        {
            return Err(format!(
                "cannot reliably verify conditional target configuration in {}",
                path.display()
            )
            .into());
        }
        if document.contains_key("profile") || document.contains_key("unstable") {
            return Err(format!(
                "cannot reliably verify profile/unstable compiler settings in {}",
                path.display()
            )
            .into());
        }
    }
    let mut environment = std::env::vars_os()
        .map(|(key, value)| {
            // Cargo's conventional uppercase keys are case-insensitive on Windows.
            let key = if cfg!(windows) {
                key.to_ascii_uppercase()
            } else {
                key
            };
            (key, value)
        })
        .collect::<BTreeMap<_, _>>();
    let command = runner::target_cargo(&selection.cargo, &selection.platform);
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => {
                environment.insert(key.to_owned(), value.to_owned());
            }
            None => {
                environment.remove(key);
            }
        }
    }
    if environment.keys().any(|key| {
        key.to_str()
            .is_some_and(|key| key.starts_with("CARGO_PROFILE_") && key.ends_with("_RUSTFLAGS"))
    }) {
        return Err("cannot reliably verify profile-specific environment rustflags".into());
    }
    let config = Config::load_with_options(cwd, ResolveOptions::default().env(environment))?;
    let flags = config
        .rustflags(selection.cargo.target.as_str())?
        .unwrap_or_default();
    if flags
        .flags
        .iter()
        .any(|flag| flag == "--target" || flag.starts_with("--target="))
    {
        return Err("cannot reliably verify a second --target supplied through rustflags".into());
    }
    let cpu = target_cpu(&flags.flags)?.ok_or(
        "cannot determine an explicit effective target CPU (compiler defaults are not inferred)",
    )?;
    if cpu != selection.platform.cpu {
        return Err(format!(
            "declared CPU {:?}, effective CPU {:?}",
            selection.platform.cpu, cpu
        )
        .into());
    }
    Ok(())
}

fn target_cpu(flags: &[String]) -> Result<Option<String>> {
    let mut cpu = None;
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        if flag.starts_with('@') {
            return Err("cannot determine the target CPU from rustflags response files".into());
        }
        let option = if flag == "-C" || flag == "--codegen" {
            flags.next().map(String::as_str)
        } else {
            flag.strip_prefix("-C")
                .or_else(|| flag.strip_prefix("--codegen="))
        };
        if let Some(("target-cpu" | "target_cpu", value)) =
            option.and_then(|option| option.split_once('='))
        {
            if value.is_empty() {
                return Err("target-cpu must not be empty".into());
            }
            cpu = Some(value.to_owned());
        }
    }
    Ok(cpu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_explicit_cpu_wins_and_response_files_are_unverifiable() {
        for option in [
            "-Ctarget-cpu=cortex-m4",
            "--codegen=target-cpu=cortex-m4",
            "-Ctarget_cpu=cortex-m4",
        ] {
            assert_eq!(
                target_cpu(&["-Ctarget-cpu=cortex-m7".into(), option.into()])
                    .unwrap()
                    .as_deref(),
                Some("cortex-m4")
            );
        }
        assert!(target_cpu(&["@flags".into()]).is_err());
        assert_eq!(target_cpu(&[]).unwrap(), None);
    }
}
