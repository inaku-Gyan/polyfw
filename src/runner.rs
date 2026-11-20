use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::{CargoContext, Platform, Selection};
use crate::profile::Profile;

pub(crate) fn workspace_cargo(workspace: &Path) -> Command {
    workspace_command(workspace, "cargo")
}

pub(crate) fn workspace_command(workspace: &Path, program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(workspace)
        .env_remove("RUSTUP_TOOLCHAIN");
    command
}

pub(crate) fn cargo_command(context: &CargoContext) -> Command {
    let mut command = workspace_cargo(&context.working_dir);
    if let Some(directory) = &context.target_dir_override {
        command.env("CARGO_TARGET_DIR", directory);
    }
    if let Some(directory) = &context.build_dir_override {
        command.env("CARGO_BUILD_BUILD_DIR", directory);
    }
    command
}

pub(crate) fn target_cargo(context: &CargoContext, platform: &Platform) -> Command {
    let mut command = cargo_command(context);
    let key = target_rustflags_key(&context.target);
    let mut flags = std::env::var_os(&key).unwrap_or_default();
    if !flags.is_empty() {
        flags.push(" ");
    }
    flags.push(format!("-Ctarget-cpu={}", platform.cpu));
    command.env(key, flags);
    command
}

/// The `cargo build` command that creates the selection's ELF.
pub(crate) fn build_command(selection: &Selection) -> Command {
    let mut command = target_cargo(&selection.firmware.cargo, &selection.platform);
    command
        .arg("build")
        .arg("--target")
        .arg(&selection.platform.target)
        .arg("-p")
        .arg(&selection.firmware.package);
    if let Some(bin) = &selection.firmware.bin {
        command.arg("--bin").arg(bin);
    }
    command.arg("--locked");
    if let Some(profile) = &selection.profile {
        command.arg("--profile").arg(profile.name());
    }
    command
}

pub(crate) fn target_rustflags_key(target: &str) -> String {
    format!(
        "CARGO_TARGET_{}_RUSTFLAGS",
        target.replace(['-', '.'], "_").to_uppercase()
    )
}

pub(crate) fn resolve_target(
    working_dir: &Path,
    target: &str,
    build_dir: bool,
) -> crate::Result<String> {
    let mut host = None;
    if target == "host-tuple" || build_dir {
        let output = workspace_cargo(working_dir).arg("-vV").output()?;
        if !output.status.success() {
            return Err(format!(
                "could not query Cargo in {}: {}",
                working_dir.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let version = String::from_utf8(output.stdout)?;
        if build_dir {
            let release = version
                .lines()
                .find_map(|line| line.strip_prefix("release: "))
                .or_else(|| {
                    version
                        .lines()
                        .next()?
                        .strip_prefix("cargo ")?
                        .split_whitespace()
                        .next()
                })
                .ok_or("could not determine Cargo version for build-dir")?;
            let release = semver::Version::parse(release)?;
            if release.major == 1 && release.minor < 91 {
                return Err(
                    "build-dir requires Cargo 1.91 or newer in the firmware working directory"
                        .into(),
                );
            }
        }
        host = version
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .map(str::to_owned);
    }
    if target == "host-tuple" {
        return host.ok_or_else(|| "Cargo did not report its host target".into());
    }
    if target.ends_with(".json") {
        return Path::new(target)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
            .ok_or_else(|| "invalid custom target filename".into());
    }
    Ok(target.to_owned())
}

pub(crate) fn firmware_elf_path(selection: &Selection) -> PathBuf {
    let bin = if selection.firmware.cargo.target.contains("-windows-") {
        format!("{}.exe", selection.firmware.resolved_bin)
    } else {
        selection.firmware.resolved_bin.clone()
    };
    selection
        .firmware
        .cargo
        .target_dir
        .join(&selection.firmware.cargo.target)
        .join(
            selection
                .profile
                .as_ref()
                .map_or("debug", Profile::directory),
        )
        .join(bin)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::model::Firmware;

    #[test]
    fn temporary_projects_use_the_project_toolchain_with_an_unusable_ancestor() {
        let root =
            std::env::temp_dir().join(format!("polyfw-fixture-ancestor-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let _cleanup = std::mem::DropGuard::new(&root, |root| {
            let _ = fs::remove_dir_all(root);
        });
        let missing = toml::Value::String(root.join("missing-toolchain").display().to_string());
        fs::write(
            root.join("rust-toolchain.toml"),
            format!("[toolchain]\npath = {missing}\n"),
        )
        .unwrap();
        for test in [
            "commands::tests::host_lint_rejects_mistakes_in_both_platform_and_firmware_tests",
            "scope::tests::a_firmware_can_verify_with_std_helpers_without_checking_unrelated_packages",
            "scope::tests::same_named_dependencies_in_different_workspaces_are_both_verified",
            "scope::tests::external_directory_patterns_include_packages_and_their_own_host_helpers",
            "scope::tests::lint_checks_other_bins_in_the_selected_firmware_package",
        ] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture"])
                .env("TMPDIR", &root)
                .env("TMP", &root)
                .env("TEMP", &root)
                .env("RUSTUP_AUTO_INSTALL", "0")
                .env("CARGO_NET_OFFLINE", "true")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{test}:\n{stdout}\n{stderr}");
            assert!(stdout.contains("1 passed"), "{test}:\n{stdout}\n{stderr}");
        }
    }

    #[test]
    fn firmware_paths_select_the_platform_target_profile_and_resolved_binary() {
        let root = Path::new("repository");
        let mut selection = Selection {
            id: "controller".into(),
            profile: None,
            firmware: Firmware {
                description: Some("test firmware".into()),
                platform: "firmware-family".into(),
                package: "firmware-package".into(),
                bin: None,
                resolved_bin: "firmware-package".into(),
                cargo: CargoContext::fixture(root, "firmware-family"),
                ..Firmware::default()
            },
            platform: Platform {
                working_dir: Some("platform/test".into()),
                target: "thumbv7em-none-eabihf".into(),
                cpu: "cortex-m4".into(),
                chip: "test-chip".into(),
                ..Platform::default()
            },
        };
        let target_dir = root.join("target/firmware/firmware-family/thumbv7em-none-eabihf");
        assert_eq!(
            firmware_elf_path(&selection),
            target_dir.join("debug/firmware-package")
        );
        selection.firmware.resolved_bin = "robot-image".into();
        selection.profile = Some("release".parse().unwrap());
        assert_eq!(
            firmware_elf_path(&selection),
            target_dir.join("release/robot-image")
        );
        selection.profile = Some("dev".parse().unwrap());
        assert_eq!(
            firmware_elf_path(&selection),
            target_dir.join("debug/robot-image")
        );
    }

    #[test]
    fn cargo_uses_the_platform_target_and_compiler_configuration() {
        let host = Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .unwrap();
        assert!(host.status.success());
        let target = String::from_utf8(host.stdout).unwrap().trim().to_owned();
        let root = std::env::temp_dir().join(format!("polyfw-flags-{}", std::process::id()));
        let workspace = root.join("platform/test");
        fs::create_dir_all(workspace.join(".cargo")).unwrap();
        fs::write(
            root.join("rust-toolchain.toml"),
            include_str!("../rust-toolchain.toml"),
        )
        .unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"test-firmware\"\nversion = \"0.1.0\"\n[lib]\npath = \"lib.rs\"\n",
        )
        .unwrap();
        fs::write(workspace.join("lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
        fs::write(
            workspace.join(".cargo/config.toml"),
            format!(
                "[build]\ntarget = {target:?}\n[target.{target}]\n\
                 rustflags = [\"--cfg\", \"platform_configuration\"]\n"
            ),
        )
        .unwrap();
        let platform = Platform {
            working_dir: Some("platform/test".into()),
            target: target.clone(),
            cpu: "generic".into(),
            chip: "test-chip".into(),
            ..Platform::default()
        };
        let mut cargo = CargoContext::fixture(&root, "firmware-family");
        cargo.target = target.clone();
        let mut command = target_cargo(&cargo, &platform);
        let target_dir = root.join("target/firmware/firmware-family");
        assert_eq!(command.get_current_dir(), Some(workspace.as_path()));
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == "CARGO_TARGET_DIR")
                .and_then(|(_, value)| value)
                .map(Path::new),
            Some(target_dir.as_path())
        );

        let output = command
            .args(["rustc", "--quiet", "--lib", "--", "--print", "cfg"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let cfg = String::from_utf8(output.stdout).unwrap();
        assert!(cfg.lines().any(|line| line == "platform_configuration"));

        fs::remove_dir_all(root).unwrap();
    }
}
