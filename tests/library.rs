#![forbid(unsafe_code)]

use std::fs;
use std::mem::DropGuard;
use std::process::Command;

use polyfw::{ErrorKind, FirmwareChoice, FirmwareTarget, Options, Profile, ProgressEvent, Project};

#[allow(dead_code)]
mod support;

#[test]
fn a_host_program_can_build_with_explicit_parameters_and_opt_into_saved_selection() {
    const CHILD: &str = "POLYFW_TEST_LIBRARY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = support::isolate_cargo_environment(Command::new(
            std::env::current_exe().unwrap(),
        ))
        .args([
            "--exact",
            "a_host_program_can_build_with_explicit_parameters_and_opt_into_saved_selection",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTUP_AUTO_INSTALL", "0")
        .env("RUSTUP_TOOLCHAIN", "polyfw-test-unavailable")
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
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
config-version = "0.2.0"
min-polyfw-version = "0.2.0"

[platforms.host]
target = "host-tuple"
cpu = "native"
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
