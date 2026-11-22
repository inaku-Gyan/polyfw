#![cfg(feature = "cli")]
#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

mod support;

struct Example {
    root: PathBuf,
}

impl Example {
    fn new(name: &str) -> Self {
        static NEXT_COPY: AtomicUsize = AtomicUsize::new(0);
        let copy = NEXT_COPY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "polyfw-example-{name}-{}-{copy}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        copy_project(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("demos")
                .join(name),
            &root,
        );
        Self { root }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = support::polyfw();
        command
            .current_dir(&self.root)
            .args(args)
            .env("RUSTUP_TOOLCHAIN", "polyfw-test-unavailable")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("NO_COLOR", "1");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        succeeded(self.command(args).output().unwrap())
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        serde_json::from_slice(&self.run(args).stdout).unwrap()
    }

    fn assert_test_fails(&self, scope: &str, marker: &str) {
        failed_test(self.command(&["test", scope]).output().unwrap(), marker);
    }

    fn add_failing_test(&self, path: &str, marker: &str) {
        self.append_source(
            path,
            &format!("\n#[test]\nfn scope_sentinel() {{\n    panic!({marker:?});\n}}\n"),
        );
    }

    fn append_source(&self, path: &str, addition: &str) {
        let path = self.root.join(path);
        let source = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{source}{addition}")).unwrap();
    }
}

impl Drop for Example {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_project(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if matches!(
            entry.file_name().to_str(),
            Some("target" | ".polyfw-state" | ".vscode")
        ) {
            continue;
        }
        let path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_project(&entry.path(), &path);
        } else {
            fs::copy(entry.path(), path).unwrap();
        }
    }
}

fn succeeded(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn failed(output: Output, marker: &str) -> Output {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(marker) || stderr.contains(marker),
        "missing failure marker {marker:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    output
}

fn failed_test(output: Output, marker: &str) -> Output {
    let output = failed(output, marker);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(marker),
        "missing test panic marker {marker:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn assert_verify_stages(output: &Output, expected: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stages = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("  $ cargo "))
        .map(|command| match command.split_whitespace().next().unwrap() {
            "fmt" => "fmt",
            "clippy" if command.contains("--target host-tuple") => "host clippy",
            "clippy" => "mcu clippy",
            "test" => "test",
            _ => panic!("unexpected verify command: {command}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(stages, expected, "stderr:\n{stderr}");
}

fn artifact<'a>(report: &'a serde_json::Value, id: &str, profile: &str) -> &'a serde_json::Value {
    report["firmwares"][id]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["profile"] == profile)
        .unwrap_or_else(|| panic!("missing {profile} artifact for {id}"))
}

fn assert_artifact_absent(report: &serde_json::Value, id: &str, profile: &str) {
    let artifact = artifact(report, id, profile);
    assert_eq!(artifact["exists"], false, "{id} ({profile})");
    let path = Path::new(artifact["path"].as_str().unwrap());
    assert!(!path.exists(), "{} must not exist", path.display());
}

fn assert_arm_artifact(report: &serde_json::Value, id: &str, profile: &str) {
    let firmware = &report["firmwares"][id];
    assert_eq!(firmware["target"], "thumbv7em-none-eabihf");
    let artifact = artifact(report, id, profile);
    assert_eq!(artifact["exists"], true);
    let path = Path::new(artifact["path"].as_str().unwrap());
    assert!(path.is_absolute());
    assert_eq!(
        path,
        Path::new(firmware["target_dir"].as_str().unwrap())
            .join("thumbv7em-none-eabihf")
            .join(if profile == "dev" { "debug" } else { profile })
            .join(firmware["bin"].as_str().unwrap())
    );
    let elf = fs::read(path).unwrap();
    assert!(
        elf.len() >= 52,
        "ELF header missing from {}",
        path.display()
    );
    assert_eq!(&elf[..6], b"\x7fELF\x01\x01", "32-bit little-endian ELF");
    assert_eq!(&elf[16..18], &[2, 0], "ELF must be an executable");
    assert_eq!(&elf[18..20], &[40, 0], "ELF must target ARM");
    let entry = u32::from_le_bytes(elf[24..28].try_into().unwrap());
    assert_eq!(entry & 1, 1, "Cortex-M entry point must use Thumb mode");
    assert!(
        (0x0800_0000..0x0806_0000).contains(&entry),
        "entry point {entry:#x} must lie in the configured flash region"
    );
}

#[test]
fn minimal_example_discovers_selects_and_builds_the_reported_artifact() {
    let example = Example::new("minimal");
    let output = succeeded(
        example
            .command(&["info", "--json"])
            .current_dir(example.root.join("src"))
            .output()
            .unwrap(),
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        Path::new(report["repository_root"].as_str().unwrap()),
        example.root
    );
    assert!(report["saved_selection"].is_null());
    assert_eq!(report["firmwares"]["hello"]["package"], "hello");
    for artifact in report["firmwares"]["hello"]["artifacts"]
        .as_array()
        .unwrap()
    {
        assert_eq!(artifact["exists"], false);
    }

    example.run(&["select", "hello", "--release"]);
    example.run(&["build"]);
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "hello");
    assert_eq!(report["saved_selection"]["profile"], "release");
    assert_arm_artifact(&report, "hello", "release");
    assert_eq!(report["platforms"]["dm-mc02"]["cpu"], "cortex-m7");
    assert_eq!(report["platforms"]["dm-mc02"]["chip"], "STM32H723VGTx");
    assert!(!example.root.join(".vscode/launch.json").exists());
    example.run(&["verify", "--all-firmwares"]);
    example.run(&["select", "--clear"]);
    assert!(!example.root.join(".polyfw-state/selected").exists());
}

#[test]
fn minimal_example_verify_stops_after_a_formatting_failure_without_rewriting_sources() {
    let example = Example::new("minimal");
    example.append_source("src/lib.rs", "\npub fn format_sentinel( ){ }\n");
    let path = example.root.join("src/lib.rs");
    let source = fs::read(&path).unwrap();

    let output = failed(
        example.command(&["verify", "hello"]).output().unwrap(),
        "Diff in",
    );
    assert_verify_stages(&output, &["fmt"]);
    assert_eq!(fs::read(path).unwrap(), source);
}

#[test]
fn minimal_example_verify_stops_after_an_mcu_lint_failure() {
    let example = Example::new("minimal");
    example.append_source(
        "src/lib.rs",
        "\n#[cfg(target_os = \"none\")]\npub fn mcu_lint_sentinel(values: &[u8]) -> bool {\n    values.len() == 0\n}\n",
    );

    let output = failed(
        example.command(&["verify", "hello"]).output().unwrap(),
        "clippy::len_zero",
    );
    assert_verify_stages(&output, &["fmt", "mcu clippy"]);
}

#[test]
fn minimal_example_verify_stops_after_a_host_lint_failure() {
    let example = Example::new("minimal");
    example.append_source(
        "src/lib.rs",
        "\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn host_lint_sentinel() {\n        let values: &[u8] = &[];\n        assert!(values.len() == 0);\n    }\n}\n",
    );

    let output = failed(
        example.command(&["verify", "hello"]).output().unwrap(),
        "clippy::len_zero",
    );
    assert_verify_stages(&output, &["fmt", "mcu clippy", "host clippy"]);
}

#[test]
fn minimal_example_verify_reports_a_host_test_failure_after_formatting_and_linting() {
    let example = Example::new("minimal");
    example.add_failing_test("src/lib.rs", "verify host test sentinel");

    let output = failed_test(
        example.command(&["verify", "hello"]).output().unwrap(),
        "verify host test sentinel",
    );
    assert_verify_stages(&output, &["fmt", "mcu clippy", "host clippy", "test"]);
}

#[test]
fn multi_firmware_example_switches_and_checks_only_the_selected_scope() {
    let example = Example::new("multi-firmware");
    let report = example.json(&["info", "--json=1"]);
    assert_eq!(report, example.json(&["info", "--json"]));
    assert_eq!(report["firmwares"].as_object().unwrap().len(), 2);
    for id in ["led-demo", "buzzer-demo"] {
        let details = example.json(&["info", "--firmware", id, "--json", "1"]);
        assert_eq!(
            details,
            example.json(&["info", "--json=latest", "--firmware", id])
        );
        let dependencies = details["firmwares"][id]["local_dependencies"]
            .as_array()
            .unwrap();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0]["package"], "common");
    }

    example.run(&["select", "led-demo", "--release"]);
    example.run(&["build", "--all-firmwares", "--release"]);
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "led-demo");
    for id in ["led-demo", "buzzer-demo"] {
        assert_arm_artifact(&report, id, "release");
    }
    example.run(&["select", "buzzer-demo", "--dev"]);
    example.run(&["build"]);
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "buzzer-demo");
    assert_eq!(report["saved_selection"]["profile"], "dev");
    assert_arm_artifact(&report, "buzzer-demo", "dev");
    assert_artifact_absent(&report, "led-demo", "dev");
    example.run(&["verify", "--all-firmwares"]);

    example.add_failing_test(
        "firmwares/buzzer-demo/src/lib.rs",
        "unselected firmware test",
    );
    example.run(&["test", "led-demo"]);
    example.assert_test_fails("--all-firmwares", "unselected firmware test");
}

#[test]
fn multi_firmware_example_waits_for_concurrent_selection_writes() {
    let example = Example::new("multi-firmware");
    let config = example.root.join("polyfw.toml");
    let source = fs::read_to_string(&config).unwrap();
    fs::write(&config, source.replace("enabled = false", "enabled = true")).unwrap();
    example.run(&["select", "led-demo", "--dev"]);

    let selection = example.root.join(".polyfw-state/selected");
    let launch = example.root.join(".vscode/launch.json");
    let lock_path = example.root.join(".polyfw-state/selection.lock");
    let run_while_locked = |args: &[&str]| {
        let lock = fs::File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        lock.lock().unwrap();
        let previous_selection = fs::read(&selection).unwrap();
        let previous_launch = fs::read(&launch).unwrap();
        let child = example
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let (complete, completed) = mpsc::channel();
        thread::scope(|scope| {
            scope.spawn(move || complete.send(child.wait_with_output()).unwrap());
            let early_result = completed.recv_timeout(Duration::from_secs(1));
            let selection_while_locked = fs::read(&selection);
            let launch_while_locked = fs::read(&launch);
            drop(lock);

            assert!(
                matches!(early_result, Err(mpsc::RecvTimeoutError::Timeout)),
                "{args:?} finished with the selection lock held: {early_result:?}"
            );
            assert_eq!(selection_while_locked.unwrap(), previous_selection);
            assert_eq!(launch_while_locked.unwrap(), previous_launch);
            succeeded(completed.recv().unwrap().unwrap());
        });
    };

    run_while_locked(&["select", "buzzer-demo", "--release"]);
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "buzzer-demo");
    assert_eq!(report["saved_selection"]["profile"], "release");
    let launch_config: serde_json::Value =
        serde_json::from_slice(&fs::read(&launch).unwrap()).unwrap();
    assert_eq!(
        launch_config["configurations"][0]["name"],
        "Launch buzzer-demo (release)"
    );

    run_while_locked(&["select", "--clear"]);
    assert!(!selection.exists());
    assert!(!launch.exists());
}

#[test]
fn multi_firmware_example_retains_a_new_selection_when_its_build_fails() {
    let example = Example::new("multi-firmware");
    example.run(&["select", "led-demo", "--dev"]);
    example.append_source(
        "firmwares/buzzer-demo/src/main.rs",
        "\ncompile_error!(\"selected build failure sentinel\");\n",
    );

    failed(
        example
            .command(&["build", "buzzer-demo", "--select", "--release"])
            .output()
            .unwrap(),
        "selected build failure sentinel",
    );
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "buzzer-demo");
    assert_eq!(report["saved_selection"]["profile"], "release");
    assert_artifact_absent(&report, "buzzer-demo", "release");
    assert!(!example.root.join(".vscode/launch.json").exists());
}

#[test]
fn multi_workspace_example_reports_external_members_and_shared_scopes() {
    let example = Example::new("multi-workspace");
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["firmwares"].as_object().unwrap().len(), 4);
    assert_eq!(report["platforms"]["board-a"]["registered_firmwares"], 3);
    assert_eq!(report["platforms"]["board-b"]["registered_firmwares"], 1);
    for (platform, description, cpu, chip) in [
        ("board-a", "board-A", "cortex-m7", "STM32H723VGTx"),
        ("board-b", "board-B", "cortex-m4", "STM32F407IG"),
    ] {
        assert_eq!(
            report["platforms"][platform]["target"],
            "thumbv7em-none-eabihf"
        );
        assert_eq!(report["platforms"][platform]["cpu"], cpu);
        assert_eq!(report["platforms"][platform]["chip"], chip);
        assert_eq!(report["platforms"][platform]["description"], description);
    }
    for (id, platform, package, expected) in [
        ("buzzer", "board-a", "buzzer", &["patterns", "timing"][..]),
        ("screen", "board-b", "screen", &["layout", "timing"][..]),
        ("led-1", "board-a", "led", &["patterns", "timing"][..]),
        ("led-2", "board-a", "led", &["patterns", "timing"][..]),
    ] {
        let details = example.json(&["info", "--firmware", id, "--json"]);
        let firmware = &details["firmwares"][id];
        assert_eq!(firmware["platform"], platform);
        assert_eq!(firmware["package"], package);
        assert_eq!(firmware["bin"], id);
        assert_eq!(
            Path::new(firmware["workspace"].as_str().unwrap()),
            example.root.join("bsp").join(platform)
        );
        assert_eq!(firmware["working_dir"], firmware["workspace"]);
        assert_eq!(
            Path::new(firmware["target_dir"].as_str().unwrap()),
            example.root.join("target/firmware").join(platform)
        );
        let dependencies = firmware["local_dependencies"].as_array().unwrap();
        assert_eq!(
            dependencies
                .iter()
                .map(|dependency| dependency["package"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        for dependency in dependencies {
            let name = dependency["package"].as_str().unwrap();
            assert_eq!(
                Path::new(dependency["manifest_path"].as_str().unwrap()),
                Path::new("shared").join(name).join("Cargo.toml")
            );
        }
    }

    let output = example.run(&["verify", "--all-firmwares"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    for name in [
        "pulse_patterns_have_nonzero_on_and_off_durations",
        "fast_pulses_have_a_shorter_cycle_than_slow_pulses",
        "fast_blinks_have_shorter_on_and_off_intervals_than_slow_blinks",
    ] {
        assert!(
            stdout.contains(&format!("test tests::{name} ... ok")),
            "host test {name} did not run successfully\nstdout:\n{stdout}"
        );
    }
    let cargo = succeeded(
        support::workspace_cargo(&example.root)
            .arg("-vV")
            .output()
            .unwrap(),
    );
    let version = String::from_utf8(cargo.stdout).unwrap();
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    assert!(
        example
            .root
            .join("target")
            .join(host)
            .join("debug")
            .is_dir()
    );
}

#[test]
fn multi_workspace_example_builds_selected_bins_and_all_firmwares() {
    let example = Example::new("multi-workspace");
    example.run(&["select", "led-1", "--release"]);
    example.run(&["build"]);
    let report = example.json(&["info", "--json"]);
    assert_arm_artifact(&report, "led-1", "release");
    for id in ["buzzer", "screen", "led-2"] {
        assert_artifact_absent(&report, id, "release");
    }

    example.run(&["select", "led-2", "--release"]);
    example.run(&["build"]);
    let report = example.json(&["info", "--json"]);
    assert_arm_artifact(&report, "led-2", "release");
    assert_arm_artifact(&report, "led-1", "release");

    example.run(&["build", "--all-firmwares", "--release"]);
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "led-2");
    assert_eq!(report["saved_selection"]["profile"], "release");
    for id in ["buzzer", "screen", "led-1", "led-2"] {
        assert_arm_artifact(&report, id, "release");
    }
}

#[test]
fn multi_workspace_example_cleans_package_images_and_all_registered_output_contexts() {
    let example = Example::new("multi-workspace");
    let ids = ["buzzer", "screen", "led-1", "led-2"];
    example.run(&["build", "--all-firmwares", "--dev"]);
    example.run(&["build", "--all-firmwares", "--release"]);
    let report = example.json(&["info", "--json"]);
    for id in ids {
        for profile in ["dev", "release"] {
            assert_arm_artifact(&report, id, profile);
        }
    }

    example.run(&["clean", "led-1"]);
    let report = example.json(&["info", "--json"]);
    for id in ["led-1", "led-2"] {
        assert_artifact_absent(&report, id, "dev");
    }
    for id in ["buzzer", "screen"] {
        assert_arm_artifact(&report, id, "dev");
    }
    for id in ids {
        assert_arm_artifact(&report, id, "release");
    }

    example.run(&["clean", "--all-firmwares"]);
    let report = example.json(&["info", "--json"]);
    for id in ids {
        for profile in ["dev", "release"] {
            assert_artifact_absent(&report, id, profile);
        }
    }

    example.run(&["build", "led-2", "--dev"]);
    let report = example.json(&["info", "--json"]);
    assert_arm_artifact(&report, "led-2", "dev");
    assert_artifact_absent(&report, "led-1", "dev");
}

fn assert_shared_test_scope(package: &str, included: &[&str], excluded: &[&str]) {
    let example = Example::new("multi-workspace");
    let marker = format!("{package} test");
    example.add_failing_test(&format!("shared/{package}/src/lib.rs"), &marker);
    for id in included {
        example.assert_test_fails(id, &marker);
    }
    for id in excluded {
        example.run(&["test", id]);
    }
}

#[test]
fn multi_workspace_example_tests_patterns_only_for_buzzer_and_led_firmwares() {
    assert_shared_test_scope("patterns", &["buzzer", "led-1", "led-2"], &["screen"]);
}

#[test]
fn multi_workspace_example_tests_layout_only_for_the_screen_firmware() {
    assert_shared_test_scope("layout", &["screen"], &["buzzer", "led-1", "led-2"]);
}

#[test]
fn multi_workspace_example_tests_timing_for_direct_and_transitive_dependents() {
    assert_shared_test_scope("timing", &["buzzer", "screen", "led-1", "led-2"], &[]);
}
