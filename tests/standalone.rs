#![cfg(feature = "cli")]
#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn help_version_and_schema_work_without_a_project_or_rust_tools() {
    let fixture = Fixture::new("project-free");
    fs::write(fixture.root.join("polyfw.toml"), "invalid TOML").unwrap();
    for args in [
        vec![],
        vec!["--help"],
        vec!["help", "build"],
        vec!["help", "init"],
        vec!["init", "--help"],
    ] {
        let output = fixture.command().args(args).output().unwrap();
        succeeded(&output);
        assert!(String::from_utf8(output.stdout).unwrap().contains("plfw"));
    }
    let output = fixture.command().arg("--version").output().unwrap();
    succeeded(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        concat!("plfw ", env!("CARGO_PKG_VERSION"))
    );
    let output = fixture
        .command()
        .args(["schema", "--project-root", "missing"])
        .output()
        .unwrap();
    succeeded(&output);
    assert_eq!(
        output.stdout,
        include_bytes!("../assets/polyfw.schema.json")
    );
    let output = fixture
        .command()
        .args(["schema", "--output", "polyfw.schema.json"])
        .output()
        .unwrap();
    succeeded(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read(fixture.root.join("polyfw.schema.json")).unwrap(),
        include_bytes!("../assets/polyfw.schema.json")
    );
}

#[test]
fn info_rejects_unsupported_json_versions_before_opening_a_project() {
    let fixture = Fixture::new("info-json-version");
    fs::write(fixture.root.join("polyfw.toml"), "invalid TOML").unwrap();
    for (args, version) in [
        (vec!["--json=1"], "1"),
        (vec!["--json", "1"], "1"),
        (vec!["--json=9999"], "9999"),
        (vec!["--json", "9999"], "9999"),
    ] {
        let output = fixture.command().arg("info").args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(&format!("invalid value '{version}'")),
            "{stderr}"
        );
        assert!(stderr.contains("possible values: 2, latest"), "{stderr}");
    }
}

#[test]
fn initialization_creates_template_and_state_ignore_without_tools_or_ancestor_discovery() {
    let fixture = Fixture::new("init-current-directory");
    fs::write(
        fixture.root.join("polyfw.toml"),
        "invalid ancestor configuration",
    )
    .unwrap();
    let root = fixture.root.join("firmware-demos");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("Cargo.toml"), "unread Cargo manifest").unwrap();
    fs::write(root.join(".gitignore"), "existing ignores\n").unwrap();

    let output = fixture
        .command()
        .current_dir(&root)
        .arg("init")
        .output()
        .unwrap();
    succeeded(&output);
    let message = String::from_utf8(output.stdout).unwrap();
    let canonical_root = root.canonicalize().unwrap();
    let state = canonical_root.join(".polyfw-state");
    let ignore = state.join(".gitignore");
    for path in [canonical_root.join("polyfw.toml"), ignore.clone()] {
        let expected = format!("created {}", path.display());
        assert!(message.lines().any(|line| line == expected), "{message}");
    }
    assert!(message.contains("TODO"), "{message}");
    assert!(
        message.contains("Add /target/ and /.vscode/launch.json"),
        "{message}"
    );
    assert!(message.contains("plfw info"), "{message}");
    assert_eq!(fs::read(ignore).unwrap(), b"*\n");
    assert_eq!(fs::read_dir(state).unwrap().count(), 1);

    // Exercise version, model and structural validation without invoking Cargo.
    let output = fixture
        .command()
        .current_dir(&root)
        .args(["select", "--clear"])
        .output()
        .unwrap();
    succeeded(&output);
    assert_eq!(
        fs::read_to_string(root.join("Cargo.toml")).unwrap(),
        "unread Cargo manifest"
    );
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        "existing ignores\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("polyfw.toml")).unwrap(),
        "invalid ancestor configuration"
    );
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        [".gitignore", ".polyfw-state", "Cargo.toml", "polyfw.toml"]
    );
}

#[test]
fn initialization_uses_the_exact_explicit_directory_relative_to_the_caller() {
    for (label, absolute, root_first) in [
        ("relative-before", false, true),
        ("relative-after", false, false),
        ("absolute-before", true, true),
        ("absolute-after", true, false),
    ] {
        let fixture = Fixture::new(&format!("init-{label}"));
        fs::write(
            fixture.root.join("polyfw.toml"),
            "invalid ancestor configuration",
        )
        .unwrap();
        let caller = fixture.root.join("caller");
        let destination = fixture.root.join("firmware-demos");
        fs::create_dir(&caller).unwrap();
        fs::create_dir(&destination).unwrap();
        let argument = if absolute {
            destination.clone()
        } else {
            PathBuf::from("../firmware-demos")
        };
        let mut command = fixture.command();
        command.current_dir(&caller);
        if !root_first {
            command.arg("init");
        }
        command.arg("--project-root").arg(argument);
        if root_first {
            command.arg("init");
        }
        succeeded(&command.output().unwrap());
        assert!(destination.join("polyfw.toml").is_file());
        assert_eq!(
            fs::read(destination.join(".polyfw-state/.gitignore")).unwrap(),
            b"*\n"
        );
        assert!(!caller.join("polyfw.toml").exists());
        assert!(!caller.join(".polyfw-state").exists());
        assert_eq!(
            fs::read_to_string(fixture.root.join("polyfw.toml")).unwrap(),
            "invalid ancestor configuration"
        );
    }
}

#[test]
fn initialization_requires_an_existing_directory() {
    let fixture = Fixture::new("init-invalid-root");
    fs::write(fixture.root.join("not a directory"), "existing file").unwrap();
    for root in ["missing/child", "not a directory"] {
        let output = fixture
            .command()
            .args(["init", "--project-root", root])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(root), "{error}");
    }
    assert!(!fixture.root.join("missing").exists());
    assert!(!fixture.root.join("polyfw.toml").exists());
    assert_eq!(
        fs::read_to_string(fixture.root.join("not a directory")).unwrap(),
        "existing file"
    );
}

#[test]
fn initialization_preserves_existing_configuration_files_and_directories() {
    for directory in [false, true] {
        let fixture = Fixture::new(&format!("init-existing-{directory}"));
        let path = fixture.root.join("polyfw.toml");
        let original = b"existing configuration\xff";
        let protected = if directory {
            fs::create_dir(&path).unwrap();
            path.join("keep")
        } else {
            path.clone()
        };
        fs::write(&protected, original).unwrap();
        let output = fixture.command().arg("init").output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("polyfw.toml"), "{error}");
        if !directory {
            assert!(error.contains("already exists"), "{error}");
        }
        assert_eq!(fs::read(protected).unwrap(), original);
        assert!(!fixture.root.join(".polyfw-state").exists());
    }
}

#[cfg(unix)]
#[test]
fn initialization_preserves_existing_configuration_symlinks_including_dangling_ones() {
    for dangling in [false, true] {
        let fixture = Fixture::new(&format!("init-symlink-{dangling}"));
        let target = fixture.root.join("target.toml");
        if !dangling {
            fs::write(&target, "existing configuration").unwrap();
        }
        let path = fixture.root.join("polyfw.toml");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let output = fixture.command().arg("init").output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(fs::read_link(path).unwrap(), target);
        if dangling {
            assert!(!target.exists());
        } else {
            assert_eq!(
                fs::read_to_string(target).unwrap(),
                "existing configuration"
            );
        }
        assert!(!fixture.root.join(".polyfw-state").exists());
    }
}

#[test]
fn initialization_uses_an_existing_state_ignore_file() {
    let fixture = Fixture::new("init-existing-ignore");
    let root = fixture.root.canonicalize().unwrap();
    let state = root.join(".polyfw-state");
    let ignore = state.join(".gitignore");
    fs::create_dir(&state).unwrap();
    fs::write(&ignore, b"selected\r\n").unwrap();

    let output = fixture.command().arg("init").output().unwrap();
    succeeded(&output);
    let message = String::from_utf8(output.stdout).unwrap();
    let created = format!("created {}", root.join("polyfw.toml").display());
    assert_eq!(
        message
            .lines()
            .filter(|line| line.starts_with("created "))
            .collect::<Vec<_>>(),
        [created.as_str()]
    );
    assert!(message.contains(&format!(
        "Using existing ignore rules in {}",
        ignore.display()
    )));
    assert_eq!(fs::read(ignore).unwrap(), b"selected\r\n");
}

#[test]
fn initialization_can_be_retried_after_state_path_conflicts() {
    let fixture = Fixture::new("init-state-path-conflicts");
    let state = fixture.root.canonicalize().unwrap().join(".polyfw-state");
    fs::write(&state, b"existing file").unwrap();
    assert_init_state_conflict(&fixture, &state);
    assert_eq!(fs::read(&state).unwrap(), b"existing file");
    fs::remove_file(&state).unwrap();

    let ignore = state.join(".gitignore");
    fs::create_dir_all(&ignore).unwrap();
    assert_init_state_conflict(&fixture, &ignore);
    fs::remove_dir(&ignore).unwrap();

    succeeded(&fixture.command().arg("init").output().unwrap());
    assert!(fixture.root.join("polyfw.toml").is_file());
    assert_eq!(fs::read(ignore).unwrap(), b"*\n");
}

#[cfg(unix)]
#[test]
fn initialization_rejects_symlinked_state_paths() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("init-state-symlinks");
    let root = fixture.root.canonicalize().unwrap();
    let target = root.join("target-directory");
    fs::create_dir(&target).unwrap();
    let state = root.join(".polyfw-state");
    symlink(&target, &state).unwrap();
    assert_init_state_conflict(&fixture, &state);
    assert_eq!(fs::read_link(&state).unwrap(), target);
    assert!(!target.join(".gitignore").exists());
    fs::remove_file(&state).unwrap();

    fs::create_dir(&state).unwrap();
    let ignore = state.join(".gitignore");
    let target = root.join("target-file");
    fs::write(&target, b"existing rules").unwrap();
    symlink(&target, &ignore).unwrap();
    assert_init_state_conflict(&fixture, &ignore);
    assert_eq!(fs::read_link(ignore).unwrap(), target);
    assert_eq!(fs::read(target).unwrap(), b"existing rules");
}

fn assert_init_state_conflict(fixture: &Fixture, path: &Path) {
    let output = fixture.command().arg("init").output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(!fixture.root.join("polyfw.toml").exists());
}

#[test]
fn a_project_command_outside_a_project_reports_how_to_select_one() {
    let fixture = Fixture::new("missing-project");
    let output = fixture.command().arg("info").output().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("no polyfw.toml found"), "{error}");
    assert!(error.contains("--project-root <DIR>"), "{error}");
}

#[test]
fn configuration_versions_are_checked_before_tools_state_and_unknown_fields() {
    for (label, declarations, expected) in [
        (
            "missing-versions",
            "",
            "missing required top-level config-version",
        ),
        (
            "missing-minimum",
            "config-version = \"0.2.0\"\n",
            "missing required top-level min-polyfw-version",
        ),
        (
            "wrong-version-type",
            "config-version = 1\nmin-polyfw-version = \"0.2.0\"\n",
            "config-version must be a string",
        ),
        (
            "invalid-version",
            "config-version = \"0.1\"\nmin-polyfw-version = \"0.2.0\"\n",
            "not a full SemVer",
        ),
        (
            "inconsistent-versions",
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"999.0.0\"\n",
            "exceeds config-version",
        ),
        (
            "old-config",
            "config-version = \"0.2.0\"\nmin-polyfw-version = \"0.2.0\"\n",
            "is too old",
        ),
        (
            "old-cli",
            "config-version = \"999.0.0\"\nmin-polyfw-version = \"999.0.0\"\n",
            "Upgrade the CLI",
        ),
    ] {
        let fixture = Fixture::new(label);
        fixture.project(true);
        let path = fixture.root.join("polyfw.toml");
        let source = fs::read_to_string(&path).unwrap();
        let body = source.split_once("[config.vscode]").unwrap().1;
        fs::write(
            &path,
            format!("future-field = true\n{declarations}[config.vscode]{body}"),
        )
        .unwrap();
        fs::create_dir_all(fixture.root.join(".polyfw-state")).unwrap();
        fs::write(
            fixture.root.join(".polyfw-state/selected"),
            "original state",
        )
        .unwrap();
        fs::create_dir_all(fixture.root.join(".vscode")).unwrap();
        fs::write(
            fixture.root.join(".vscode/launch.json"),
            "original editor file",
        )
        .unwrap();

        for args in [
            vec!["info", "--json"],
            vec!["select", "--clear"],
            vec!["reset", "firmware"],
            vec!["reset", "firmware", "--select"],
            vec!["build", "--select", "firmware"],
        ] {
            let output = fixture.command().args(args).output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains(expected), "{error}");
            assert!(error.contains(&path.display().to_string()), "{error}");
            assert!(error.contains(env!("CARGO_PKG_VERSION")), "{error}");
            assert!(!error.contains("unknown field"), "{error}");
            assert_eq!(
                fs::read_to_string(fixture.root.join(".polyfw-state/selected")).unwrap(),
                "original state"
            );
            assert_eq!(
                fs::read_to_string(fixture.root.join(".vscode/launch.json")).unwrap(),
                "original editor file"
            );
        }
    }
}

#[test]
fn a_compatible_version_does_not_bypass_strict_configuration_validation() {
    let fixture = Fixture::new("compatible-unknown-field");
    fixture.project(true);
    let path = fixture.root.join("polyfw.toml");
    let source = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("future-field = true\n{source}")).unwrap();
    let output = fixture
        .command()
        .args(["select", "--clear"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("unknown field `future-field`"), "{error}");
}

#[test]
fn clearing_selection_needs_no_cargo_and_obeys_the_projects_vscode_setting() {
    for enabled in [true, false] {
        let fixture = Fixture::new(&format!("clear-{enabled}"));
        fixture.project(enabled);
        fs::create_dir_all(fixture.root.join(".polyfw-state")).unwrap();
        fs::write(fixture.root.join(".polyfw-state/selected"), "invalid state").unwrap();
        fs::create_dir_all(fixture.root.join(".vscode")).unwrap();
        let launch = fixture.root.join(".vscode/launch.json");
        fs::write(&launch, "handwritten configuration").unwrap();
        let output = fixture
            .command()
            .args(["select", "--clear"])
            .output()
            .unwrap();
        succeeded(&output);
        assert!(!fixture.root.join(".polyfw-state/selected").exists());
        assert!(fixture.root.join(".polyfw-state/selection.lock").is_file());
        if enabled {
            assert!(!launch.exists());
        } else {
            assert_eq!(
                fs::read_to_string(launch).unwrap(),
                "handwritten configuration"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn listing_probes_uses_only_probe_rs_and_ignores_project_configuration() {
    let fixture = Fixture::new("probes");
    fixture.probe();
    fs::write(fixture.root.join("polyfw.toml"), "invalid TOML").unwrap();
    let output = fixture
        .command()
        .args(["probes", "--project-root", "missing"])
        .output()
        .unwrap();
    succeeded(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "list\n");
}

#[cfg(unix)]
#[test]
fn resetting_needs_no_cargo_unless_saving_an_enabled_debug_configuration() {
    for enabled in [true, false] {
        let fixture = Fixture::new(&format!("reset-{enabled}"));
        fixture.project(enabled);
        fixture.probe();
        let output = fixture
            .command()
            .args(["reset", "firmware"])
            .output()
            .unwrap();
        succeeded(&output);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "reset\n--chip\ntest-chip\n"
        );
        assert!(!fixture.root.join(".polyfw-state/selected").exists());

        let output = fixture
            .command()
            .args(["reset", "firmware", "--select"])
            .output()
            .unwrap();
        if enabled {
            assert!(!output.status.success());
            assert!(!fixture.root.join(".polyfw-state/selected").exists());
        } else {
            succeeded(&output);
            assert_eq!(
                fs::read_to_string(fixture.root.join(".polyfw-state/selected")).unwrap(),
                "firmware = \"firmware\"\n"
            );
        }
        assert!(!fixture.root.join(".vscode/launch.json").exists());
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "polyfw-standalone-{label}-{} with spaces",
            std::process::id()
        ));
        fs::create_dir_all(root.join("tools")).unwrap();
        Self { root }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_plfw"));
        command
            .current_dir(&self.root)
            .env("PATH", self.root.join("tools"))
            .env("RUSTUP_TOOLCHAIN", "polyfw-test-unavailable")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("CARGO_NET_OFFLINE", "true")
            .env("NO_COLOR", "1");
        command
    }

    fn project(&self, vscode: bool) {
        fs::write(
            self.root.join("polyfw.toml"),
            format!(
                "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n[config.vscode]\nenabled = {vscode}\n\
                 [platforms.model]\ntarget = \"thumbv7em-none-eabihf\"\ntarget-dir = \"target/firmware/model\"\nworking-dir = \"missing\"\nchip = \"test-chip\"\n\
                 [firmwares.firmware]\ndescription = \"Fixture\"\nplatform = \"model\"\n\
                 package = \"application\"\n"
            ),
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn probe(&self) {
        use std::os::unix::fs::PermissionsExt;

        let path = self.root.join("tools/probe-rs");
        fs::write(&path, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
