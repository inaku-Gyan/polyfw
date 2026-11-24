use std::path::{Path, PathBuf};
use std::process::Command;

use super::model::{CargoContext, Selection};
use crate::profile::Profile;

pub(crate) fn workspace_cargo(workspace: &Path) -> Command {
    let command = workspace_command(workspace, "cargo");
    #[cfg(test)]
    let command = crate::test_cargo_env::isolate_cargo_environment(command);
    command
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
        command.env("CARGO_TARGET_DIR", cargo_output_directory(directory));
    }
    if let Some(directory) = &context.build_dir_override {
        command.env("CARGO_BUILD_BUILD_DIR", cargo_output_directory(directory));
    }
    command
}

#[cfg(not(windows))]
fn cargo_output_directory(directory: &Path) -> &Path {
    directory
}

#[cfg(windows)]
fn cargo_output_directory(directory: &Path) -> &Path {
    // Leave room for an 8.3 child name within the legacy Win32 path limit.
    const MAX_WIN32_DIRECTORY_LEN: usize = 260 - 12;

    // MinGW can lose the directory of an object file passed with a verbatim path.
    let Some(ordinary) = directory
        .to_str()
        .and_then(|path| path.strip_prefix(r"\\?\"))
    else {
        return directory;
    };
    let Some((drive, names)) = ordinary.split_once('\\') else {
        return directory;
    };
    if !matches!(drive.as_bytes(), [b'A'..=b'Z' | b'a'..=b'z', b':'])
        || ordinary.encode_utf16().count() > MAX_WIN32_DIRECTORY_LEN
    {
        return directory;
    }
    if names.is_empty()
        || names
            .strip_suffix('\\')
            .unwrap_or(names)
            .split('\\')
            .all(is_regular_windows_name)
    {
        Path::new(ordinary)
    } else {
        directory
    }
}

#[cfg(windows)]
fn is_regular_windows_name(name: &str) -> bool {
    if name.is_empty()
        || name.starts_with(' ')
        || name.ends_with([' ', '.'])
        || name.contains(|c| {
            matches!(
                c,
                '\0'..='\x1f' | '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*'
            )
        })
    {
        return false;
    }

    // A DOS device name can precede a space or one or more extensions.
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let device_names = ["AUX", "CON", "CONIN$", "CONOUT$", "NUL", "PRN"];
    !device_names
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
        && !stem.split_at_checked(3).is_some_and(|(prefix, number)| {
            (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
                && (matches!(number.as_bytes(), [b'0'..=b'9']) || matches!(number, "¹" | "²" | "³"))
        })
}

pub(crate) fn target_cargo(context: &CargoContext, subcommand: &str, target: &str) -> Command {
    let mut command = cargo_command(context);
    command.args([subcommand, "--target", target]);
    command
}

pub(crate) fn resolve_target(
    working_dir: &Path,
    target: &str,
    build_dir: bool,
) -> crate::InternalResult<String> {
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
    let bin = if selection.cargo.target.contains("-windows-") {
        format!("{}.exe", selection.resolved_bin)
    } else {
        selection.resolved_bin.clone()
    };
    selection
        .cargo
        .target_dir
        .join(&selection.cargo.target)
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
    use crate::config::model::{Firmware, Platform};

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
            "project::workflow::tests::host_lint_rejects_mistakes_in_both_platform_and_firmware_tests",
            "cargo::scope::tests::a_firmware_can_verify_with_std_helpers_without_checking_unrelated_packages",
            "cargo::scope::tests::same_named_dependencies_in_different_workspaces_are_both_verified",
            "cargo::scope::tests::external_directory_patterns_include_packages_and_their_own_host_helpers",
            "cargo::scope::tests::lint_checks_other_bins_in_the_selected_firmware_package",
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
            cargo: CargoContext::fixture(root, "firmware-family"),
            resolved_bin: "firmware-package".into(),
            firmware: Firmware {
                description: Some("test firmware".into()),
                platform: "firmware-family".into(),
                package: "firmware-package".into(),
                bin: None,
                ..Firmware::default()
            },
            platform: Platform {
                working_dir: Some("platform/test".into()),
                target: "thumbv7em-none-eabihf".into(),
                chip: "test-chip".into(),
                ..Platform::default()
            },
        };
        let target_dir = root.join("target/firmware/firmware-family/thumbv7em-none-eabihf");
        assert_eq!(
            firmware_elf_path(&selection),
            target_dir.join("debug/firmware-package")
        );
        selection.resolved_bin = "robot-image".into();
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
    fn cargo_fixtures_use_their_own_compiler_flags() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cargo::runner::tests::cargo_uses_the_platform_target_and_compiler_configuration",
                "project::workflow::tests::grouped_checks_preserve_distinct_execution_contexts_in_one_workspace",
                "cargo::scope::tests::lint_checks_other_bins_in_the_selected_firmware_package",
            ])
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env("RUSTFLAGS", "--cfg ambient_fixture_flag")
            .env(
                "CARGO_TARGET_THUMBV7EM_NONE_EABIHF_RUSTFLAGS",
                "--polyfw-ambient-invalid-flag",
            )
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(stdout.contains("3 passed"), "{stdout}\n{stderr}");
    }

    #[cfg(windows)]
    #[test]
    fn cargo_uses_ordinary_windows_paths_for_output_overrides() {
        let context = CargoContext {
            working_dir: PathBuf::from(r"\\?\C:\project"),
            target_dir_override: Some(PathBuf::from(r"\\?\C:\project\firmware output")),
            build_dir_override: Some(PathBuf::from(r"\\?\D:\build cache")),
            ..CargoContext::default()
        };
        let command = cargo_command(&context);
        assert_eq!(
            command.get_current_dir(),
            Some(context.working_dir.as_path())
        );
        for (key, expected) in [
            ("CARGO_TARGET_DIR", r"C:\project\firmware output"),
            ("CARGO_BUILD_BUILD_DIR", r"D:\build cache"),
        ] {
            let directory = command
                .get_envs()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| value);
            assert_eq!(directory, Some(Path::new(expected).as_os_str()), "{key}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn cargo_preserves_paths_with_special_windows_semantics() {
        let long = format!(r"\\?\C:\{}", "😀".repeat(123));
        for name in [
            r"C:\ordinary\output",
            r"\\?\UNC\server\share\output",
            r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\output",
            r"\\.\COM1",
            r"\\?\C:drive-relative",
            r"\\?\C:\unresolved\..\output",
            r"\\?\C:\literal.\output",
            r"\\?\C:\literal \output",
            r"\\?\C:\ literal\output",
            r"\\?\C:\output\\cache",
            r"\\?\C:\output/cache",
            r"\\?\C:\output:stream",
            r"\\?\C:\nul .tar.gz\output",
            r"\\?\C:\LpT²\output",
            &long,
        ] {
            let path = Path::new(name);
            assert_eq!(cargo_output_directory(path).as_os_str(), path.as_os_str());
        }
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
            include_str!("../../rust-toolchain.toml"),
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
                "[build]\ntarget = \"ignored-default-target\"\n[target.{target}]\n\
                 rustflags = [\"--cfg\", \"platform_configuration\"]\n"
            ),
        )
        .unwrap();
        let mut cargo = CargoContext::fixture(&root, "firmware-family");
        cargo.target = target.clone();
        let mut command = target_cargo(&cargo, "rustc", &target);
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
            .args(["--quiet", "--lib", "--", "--print", "cfg"])
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
