#![forbid(unsafe_code)]

use std::fs;
use std::mem::DropGuard;
use std::process::Command;

use polyfw::{ErrorKind, FirmwareChoice, FirmwareTarget, Options, Profile, ProgressEvent, Project};

#[allow(dead_code)]
mod support;

fn in_isolated_cargo_environment(test: &str, bootstrap: bool) -> bool {
    const CHILD: &str = "POLYFW_TEST_LIBRARY_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return true;
    }
    let mut command =
        support::isolate_cargo_environment(Command::new(std::env::current_exe().unwrap()));
    command
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, test)
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTUP_AUTO_INSTALL", "0")
        .env("RUSTUP_TOOLCHAIN", "polyfw-test-unavailable")
        .env_remove("RUSTC_BOOTSTRAP");
    if bootstrap {
        command.env("RUSTC_BOOTSTRAP", "1");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    false
}

#[test]
fn a_host_program_can_build_with_explicit_parameters_and_opt_into_saved_selection() {
    if !in_isolated_cargo_environment(
        "a_host_program_can_build_with_explicit_parameters_and_opt_into_saved_selection",
        false,
    ) {
        return;
    }

    let root = std::env::temp_dir().join(format!("polyfw-library-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let _cleanup = DropGuard::new(&root, |root| {
        let _ = fs::remove_dir_all(root);
    });
    fs::write(
        root.join("rust-toolchain.toml"),
        include_str!("../rust-toolchain.toml"),
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join(".polyfw-state")).unwrap();
    let state = root.join(".polyfw-state/selected");
    fs::write(&state, "invalid = [").unwrap();
    fs::write(
        root.join("polyfw.toml"),
        r#"
config-version = "0.3.0"
min-polyfw-version = "0.3.0"

[platforms.host]
target = "host-tuple"
chip = "fixture"
target-dir = "built"

[firmwares.demo]
platform = "host"
package = "polyfw-library-demo"
"#,
    )
    .unwrap();

    let project = Project::discover(root.join("src")).unwrap();
    assert_eq!(project.root(), root.canonicalize().unwrap());
    assert_eq!(
        project
            .firmwares()
            .map(|firmware| firmware.id)
            .collect::<Vec<_>>(),
        ["demo"]
    );
    assert_eq!(project.platforms().next().unwrap().1.chip, "fixture");
    assert_eq!(
        project.saved_selection().unwrap_err().kind(),
        ErrorKind::InvalidSelection
    );
    assert!(project.clear_selection().unwrap().success());
    fs::write(&state, "invalid = [").unwrap();

    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"polyfw-library-demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let lock = support::workspace_cargo(&root)
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(lock.status.success(), "{lock:?}");

    let profile: Profile = "release".parse().unwrap();
    let mut events = Vec::new();
    let mut progress = |event: ProgressEvent<'_>| match event {
        ProgressEvent::Started(stage) => events.push((stage.kind, stage.index, None)),
        ProgressEvent::Finished { stage, success, .. } => {
            events.push((stage.kind, stage.index, Some(success)))
        }
        _ => {}
    };
    let built = project
        .build(
            "demo".into(),
            Some(&profile),
            Options::default().with_progress(&mut progress),
        )
        .unwrap();
    assert_eq!(
        events,
        [
            (polyfw::StageKind::Build, 1, None),
            (polyfw::StageKind::Build, 1, Some(true))
        ]
    );
    assert!(built.artifacts[0].exists);
    assert!(built.execution.stages[0].stderr.is_some());
    let queried = project
        .artifact(
            FirmwareChoice::Explicit("demo"),
            Some(&profile),
            Options::default(),
        )
        .unwrap();
    assert_eq!(queried.path, built.artifacts[0].path);
    assert!(queried.path.is_file());
    assert_eq!(fs::read_to_string(&state).unwrap(), "invalid = [");

    let error = project
        .build(
            FirmwareTarget::One(FirmwareChoice::WithSaved(Some("demo"))),
            Some(&profile),
            Options::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidSelection);

    fs::write(
        root.join("src/main.rs"),
        "compile_error!(\"fixture build failure\");",
    )
    .unwrap();
    let mut saved = |selection: &polyfw::SavedSelection| {
        assert_eq!(project.saved_selection().unwrap().as_ref(), Some(selection));
    };
    let error = project
        .build(
            FirmwareTarget::One(FirmwareChoice::Save(Some("demo"))),
            Some(&profile),
            Options::default().on_selection_saved(&mut saved),
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ProcessFailed);
    let failed = error.failed_stage().unwrap();
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(failed.stderr.as_deref().unwrap())
            .contains("fixture build failure")
    );
    assert_eq!(
        project.saved_selection().unwrap().unwrap().profile.as_ref(),
        Some(&profile)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn custom_target_suffix_is_used_before_and_after_the_build() {
    if !in_isolated_cargo_environment(
        "custom_target_suffix_is_used_before_and_after_the_build",
        true,
    ) {
        return;
    }

    let root = std::env::temp_dir().join(format!("polyfw-custom-target-{}", std::process::id()));
    let workspace = root.join("workspace");
    fs::create_dir_all(workspace.join(".cargo")).unwrap();
    fs::create_dir_all(workspace.join("boards")).unwrap();
    fs::create_dir_all(workspace.join("src")).unwrap();
    let _cleanup = DropGuard::new(&root, |root| {
        let _ = fs::remove_dir_all(root);
    });
    fs::write(
        root.join("rust-toolchain.toml"),
        include_str!("../rust-toolchain.toml"),
    )
    .unwrap();
    fs::write(
        workspace.join(".cargo/config.toml"),
        "[unstable]\njson-target-spec = true\n[build]\nrustflags = [\"-Clink-arg=-lc\"]\n",
    )
    .unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        r#"
[package]
name = "firmware"
version = "0.1.0"
edition = "2024"

[profile.custom]
inherits = "release"
"#,
    )
    .unwrap();
    fs::write(
        root.join("polyfw.toml"),
        r#"
config-version = "0.3.0"
min-polyfw-version = "0.3.0"

[platforms.custom]
target = "target-alias.json"
chip = "fixture"
working-dir = "workspace"
target-dir = "built"

[firmwares.demo]
platform = "custom"
package = "firmware"
"#,
    )
    .unwrap();

    let output = support::workspace_command(&workspace, "rustc")
        .args(["-Z", "unstable-options", "--print", "target-spec-json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut spec: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    spec["exe-suffix"] = ".elf".into();
    let spec_path = "boards/demo-windows-board.json";
    fs::write(
        workspace.join(spec_path),
        serde_json::to_vec(&spec).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(spec_path, workspace.join("target-alias.json")).unwrap();

    // These lang items let the fixture link for a target with no precompiled core.
    let lang_items = r#"
#![feature(no_core, lang_items)]
#![no_core]
#[lang = "pointee_sized"] pub trait PointeeSized {}
#[lang = "meta_sized"] pub trait MetaSized: PointeeSized {}
#[lang = "sized"] pub trait Sized: MetaSized {}
#[lang = "copy"] pub trait Copy {}
"#;
    fs::write(workspace.join("src/lib.rs"), lang_items).unwrap();
    fs::write(
        workspace.join("src/main.rs"),
        format!(
            "#![no_main]\n{lang_items}\n#[unsafe(no_mangle)] pub extern \"C\" fn main() -> i32 {{ 0 }}"
        ),
    )
    .unwrap();
    let output = support::workspace_cargo(&workspace)
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let project = Project::open(&root).unwrap();
    let profile: Profile = "custom".parse().unwrap();
    let expected = project
        .root()
        .join("built/demo-windows-board/custom/firmware.elf");
    let lookup = || {
        project
            .artifact(
                FirmwareChoice::Explicit("demo"),
                Some(&profile),
                Options::default(),
            )
            .unwrap()
    };
    let predicted = lookup();
    assert_eq!(predicted.path, expected);
    assert!(!predicted.exists);
    project
        .save_selection(Some("demo"), Some(&profile), Options::default())
        .unwrap();
    assert!(!project.root().join("built").exists());

    let built = project
        .build("demo".into(), Some(&profile), Options::default())
        .unwrap();
    let [artifact] = built.artifacts.as_slice() else {
        panic!("expected one artifact for demo");
    };
    assert_eq!(artifact.path, expected);
    assert!(artifact.exists && artifact.path.is_file());
    let queried = lookup();
    assert_eq!(queried.path, artifact.path);
    assert!(queried.exists);
    let info = project
        .info(polyfw::InfoQuery::Overview, Options::default())
        .unwrap();
    let reported = info.firmwares["demo"]
        .artifacts
        .iter()
        .find(|artifact| artifact.profile.as_ref() == Some(&profile))
        .unwrap();
    assert_eq!(reported.path, artifact.path);
    assert!(reported.exists);

    let launch: serde_json::Value =
        serde_json::from_slice(&fs::read(project.root().join(".vscode/launch.json")).unwrap())
            .unwrap();
    for entry in launch["configurations"].as_array().unwrap() {
        let path = entry["coreConfigs"][0]["programBinary"].as_str().unwrap();
        assert_eq!(
            std::path::Path::new(path),
            artifact.path,
            "{}",
            entry["request"]
        );
    }
}
