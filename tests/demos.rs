#![cfg(feature = "cli")]
#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

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
        let mut command = support::plfw();
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

fn assert_check_stages(output: &Output, expected: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stages = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("  $ cargo "))
        .map(|command| match command.split_whitespace().next().unwrap() {
            "fmt" => "fmt",
            "clippy" if command.contains("--target host-tuple") => "host clippy",
            "clippy" => "mcu clippy",
            "test" => "test",
            _ => panic!("unexpected check command: {command}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(stages, expected, "stderr:\n{stderr}");
}

fn artifact<'a>(report: &'a serde_json::Value, id: &str, profile: &str) -> &'a serde_json::Value {
    report["firmwares"][id]["artifacts"]
        .get(profile)
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
    let flash_end = if firmware["chip"] == "STM32F401RETx" {
        0x0808_0000
    } else {
        0x0806_0000
    };
    assert!(
        (0x0800_0000..flash_end).contains(&entry),
        "entry point {entry:#x} must lie in the configured flash region"
    );
}

#[test]
fn minimal_example_discovers_selects_and_builds_the_reported_artifact() {
    let example = Example::new("minimal");
    let output = succeeded(
        example
            .command(&["info", "--json=2"])
            .current_dir(example.root.join("src"))
            .output()
            .unwrap(),
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        Path::new(report["project_root"].as_str().unwrap()),
        example.root
    );
    assert_eq!(report["format_version"], 2);
    assert!(report.get("repository_root").is_none());
    assert!(report.get("saved_selection").is_none());
    let firmware = &report["firmwares"]["blinky"];
    assert_eq!(firmware["package"], "blinky");
    assert_eq!(firmware["bin"], "blinky");
    assert!(firmware.get("check_platform").is_none());
    assert!(firmware.get("cpu").is_none());
    assert!(report["platforms"]["nucleo-f401re"].get("cpu").is_none());
    for artifact in firmware["artifacts"].as_object().unwrap().values() {
        assert_eq!(artifact["exists"], false);
    }

    example.run(&["select", "blinky", "--release"]);
    example.run(&["build"]);
    let report = example.json(&["info", "--json"]);
    assert_eq!(report["saved_selection"]["firmware"], "blinky");
    assert_eq!(report["saved_selection"]["profile"], "release");
    assert_arm_artifact(&report, "blinky", "release");
    assert_eq!(
        report["platforms"]["nucleo-f401re"]["chip"],
        "STM32F401RETx"
    );
    assert!(!example.root.join(".vscode/launch.json").exists());
    example.run(&["select", "--clear"]);
    assert!(!example.root.join(".polyfw-state/selected").exists());
}

#[test]
fn minimal_example_verifies_the_hardware_only_binary() {
    let example = Example::new("minimal");
    let output = example.run(&["verify", "--all-firmwares"]);
    assert_check_stages(&output, &["fmt", "mcu clippy", "test"]);
}

#[test]
fn minimal_example_verify_stops_after_a_formatting_failure_without_rewriting_sources() {
    let example = Example::new("minimal");
    example.append_source("src/main.rs", "\npub fn format_sentinel( ){ }\n");
    let path = example.root.join("src/main.rs");
    let source = fs::read(&path).unwrap();

    let output = failed(
        example.command(&["verify", "blinky"]).output().unwrap(),
        "Diff in",
    );
    assert_check_stages(&output, &["fmt"]);
    assert_eq!(fs::read(path).unwrap(), source);
}

#[test]
fn minimal_example_verify_stops_after_an_mcu_lint_failure() {
    let example = Example::new("minimal");
    example.append_source(
        "src/main.rs",
        "\n#[cfg(target_os = \"none\")]\npub fn mcu_lint_sentinel(values: &[u8]) -> bool {\n    values.len() == 0\n}\n",
    );

    let output = failed(
        example.command(&["verify", "blinky"]).output().unwrap(),
        "clippy::len_zero",
    );
    assert_check_stages(&output, &["fmt", "mcu clippy"]);
}

#[test]
fn verify_stops_after_a_host_lint_failure() {
    let example = Example::new("multi-firmware");
    example.append_source(
        "firmwares/led-demo/src/lib.rs",
        "\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn host_lint_sentinel() {\n        let values: &[u8] = &[];\n        assert!(values.len() == 0);\n    }\n}\n",
    );

    let output = failed(
        example.command(&["verify", "led-demo"]).output().unwrap(),
        "clippy::len_zero",
    );
    assert_check_stages(&output, &["fmt", "mcu clippy", "host clippy"]);
}

#[test]
fn verify_reports_a_host_test_failure_after_formatting_and_linting() {
    let example = Example::new("multi-firmware");
    example.add_failing_test("firmwares/led-demo/src/lib.rs", "verify host test sentinel");

    let output = failed_test(
        example.command(&["verify", "led-demo"]).output().unwrap(),
        "verify host test sentinel",
    );
    assert_check_stages(&output, &["fmt", "mcu clippy", "host clippy", "test"]);
}

fn normalize_project_paths(value: &mut serde_json::Value, root: &Path) {
    use serde_json::Value;

    match value {
        Value::String(text) => {
            let path = Path::new(text);
            let relative = path.strip_prefix(root);
            // Cargo can report ordinary paths while the fixture root is verbatim.
            #[cfg(windows)]
            let relative = relative.or_else(|error| {
                let ordinary = root.to_str().unwrap().strip_prefix(r"\\?\").map(Path::new);
                match ordinary.filter(|path| path.is_absolute()) {
                    Some(root) => path.strip_prefix(root),
                    None => Err(error),
                }
            });
            if let Ok(relative) = relative {
                let mut normalized = String::from("$PROJECT");
                for component in relative.components() {
                    normalized.push('/');
                    normalized.push_str(component.as_os_str().to_str().unwrap());
                }
                *text = normalized;
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| normalize_project_paths(value, root)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|value| normalize_project_paths(value, root)),
        _ => {}
    }
}

#[test]
fn multi_firmware_info_matches_the_json_v2_contract() {
    use serde_json::{Value, json};

    let example = Example::new("multi-firmware");
    let config_path = example.root.join("polyfw.toml");
    let config = fs::read_to_string(&config_path).unwrap().replace(
        "[platforms.dm-mc02]",
        "[platforms.dm-mc02]\ntarget-dir = \"images\"\nbuild-dir = \"intermediates\"",
    );
    fs::write(&config_path, &config).unwrap();
    let report = |args: &[&str]| {
        let mut value = example.json(args);
        normalize_project_paths(&mut value, &example.root);
        value
    };

    let artifact = |id: &str, directory: &str, exists: bool| {
        json!({
            "path": format!("$PROJECT/images/thumbv7em-none-eabihf/{directory}/{id}"),
            "exists": exists,
        })
    };
    let firmware = |id: &str| {
        json!({
            "platform": "dm-mc02",
            "package": id,
            "bin": id,
            "workspace": "$PROJECT",
            "working_dir": "$PROJECT",
            "target_dir": "$PROJECT/images",
            "build_dir": "$PROJECT/intermediates",
            "manifest_path": format!("$PROJECT/firmwares/{id}/Cargo.toml"),
            "target": "thumbv7em-none-eabihf",
            "chip": "STM32H723VGTx",
            "artifacts": {
                "dev": artifact(id, "debug", false),
                "release": artifact(id, "release", false),
            },
        })
    };
    let mut overview = json!({
        "format_version": 2,
        "project_root": "$PROJECT",
        "platforms": {
            "dm-mc02": {
                "registered_firmwares": 2,
                "target": "thumbv7em-none-eabihf",
                "chip": "STM32H723VGTx",
            },
        },
        "firmwares": {
            "led-demo": firmware("led-demo"),
            "buzzer-demo": firmware("buzzer-demo"),
        },
    });
    let detail = |overview: &Value, id: &str, dependencies: Value| {
        let mut detail = overview.clone();
        let mut firmware = overview["firmwares"][id].clone();
        firmware["local_dependencies"] = dependencies;
        detail["firmwares"] = json!({ id: firmware });
        detail
    };

    assert_eq!(report(&["info", "--json"]), overview);
    assert_eq!(report(&["info", "--json=2"]), overview);
    let common = json!([{
        "package": "common",
        "manifest_path": "$PROJECT/common/Cargo.toml",
        "target_clippy": true,
    }]);
    for id in ["led-demo", "buzzer-demo"] {
        let expected = detail(&overview, id, common.clone());
        assert_eq!(report(&["info", "--firmware", id, "--json", "2"]), expected);
        assert_eq!(report(&["info", "--firmware", id, "--json"]), expected);
    }

    let config = config
        .replace(
            "[platforms.dm-mc02]",
            "[platforms.dm-mc02]\ndescription = \"\"",
        )
        .replace(
            "[firmwares.led-demo]",
            "[firmwares.led-demo]\ndescription = \"LED demo\"",
        );
    fs::write(&config_path, &config).unwrap();
    example.run(&["select", "led-demo"]);
    overview["platforms"]["dm-mc02"]["description"] = json!("");
    overview["firmwares"]["led-demo"]["description"] = json!("LED demo");
    overview["saved_selection"] = json!({ "firmware": "led-demo" });
    assert_eq!(report(&["info", "--json=2"]), overview);

    let config = config.replace(
        "[config.vscode]",
        "[config]\nlocal-dependencies = []\n\n[config.vscode]",
    );
    fs::write(&config_path, config).unwrap();
    example.append_source(
        "Cargo.toml",
        "\n[profile.bench-debug]\ninherits = \"release\"\n",
    );
    example.run(&["select", "buzzer-demo", "--profile", "bench-debug"]);
    let image = example
        .root
        .join("images/thumbv7em-none-eabihf/bench-debug/buzzer-demo");
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    fs::write(image, "placeholder image").unwrap();

    overview["saved_selection"] = json!({ "firmware": "buzzer-demo", "profile": "bench-debug" });
    overview["firmwares"]["led-demo"]["artifacts"]["bench-debug"] =
        artifact("led-demo", "bench-debug", false);
    overview["firmwares"]["buzzer-demo"]["artifacts"]["bench-debug"] =
        artifact("buzzer-demo", "bench-debug", true);
    assert_eq!(report(&["info", "--json"]), overview);
    assert_eq!(
        report(&["info", "--firmware", "led-demo", "--json=2"]),
        detail(&overview, "led-demo", json!([])),
    );
}

#[test]
fn multi_firmware_example_switches_and_checks_only_the_selected_scope() {
    let example = Example::new("multi-firmware");
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
fn multi_firmware_example_respects_target_conditions_for_checks() {
    let example = Example::new("multi-firmware");
    let manifest = example.root.join("firmwares/led-demo/Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap().replace("\r\n", "\n");
    let mcu_manifest = source.replace(
        "[dependencies]\ncommon = { path = \"../../common\" }\n",
        "[target.thumbv7em-none-eabihf.dependencies]\ncommon = { path = \"../../common\" }\n",
    );
    assert_ne!(source, mcu_manifest, "the demo's common dependency changed");
    fs::write(&manifest, mcu_manifest).unwrap();
    example.append_source(
        "firmwares/led-demo/Cargo.toml",
        "\n[target.'cfg(not(target_os = \"none\"))'.dependencies]\n\
         simulator = { path = \"../../simulator\" }\n",
    );
    fs::create_dir_all(example.root.join("simulator/src")).unwrap();
    fs::write(
        example.root.join("simulator/Cargo.toml"),
        "[package]\nname = \"simulator\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        example.root.join("simulator/src/lib.rs"),
        "pub fn value() -> u8 {\n    std::io::Cursor::new([42]).into_inner()[0]\n}\n\n\
         #[test]\nfn simulator_reads_its_default_value() {\n    assert_eq!(value(), 42);\n}\n",
    )
    .unwrap();
    let source_path = example.root.join("firmwares/led-demo/src/lib.rs");
    let source = fs::read_to_string(&source_path)
        .unwrap()
        .replace("\r\n", "\n");
    fs::write(
        &source_path,
        source.replace(
            "pub fn value() -> u8 {\n    common::value()\n}",
            "#[cfg(target_os = \"none\")]\npub use common::value;\n\n\
             #[cfg(not(target_os = \"none\"))]\npub use simulator::value;",
        ),
    )
    .unwrap();
    example.append_source(
        "common/src/lib.rs",
        "\n#[test]\nfn mcu_dependency_runs_its_own_host_test() {\n    assert_eq!(value(), 42);\n}\n",
    );
    succeeded(
        support::workspace_cargo(&example.root)
            .args(["update", "--workspace"])
            .output()
            .unwrap(),
    );

    let report = example.json(&["info", "--firmware", "led-demo", "--json"]);
    assert_eq!(
        report["firmwares"]["led-demo"]["local_dependencies"],
        serde_json::json!([
            {
                "package": "common",
                "manifest_path": example.root.join("common/Cargo.toml"),
                "target_clippy": true,
            },
            {
                "package": "simulator",
                "manifest_path": example.root.join("simulator/Cargo.toml"),
                "target_clippy": false,
            },
        ])
    );
    let output = example.run(&["lint", "led-demo"]);
    assert_check_stages(&output, &["mcu clippy", "host clippy"]);

    let output = example.run(&["test", "led-demo"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    for name in [
        "simulator_reads_its_default_value",
        "mcu_dependency_runs_its_own_host_test",
    ] {
        assert!(
            stdout.contains(&format!("test {name} ... ok")),
            "host test {name} did not run successfully\nstdout:\n{stdout}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn multi_firmware_example_waits_for_concurrent_selection_writes() {
    use std::mem::DropGuard;
    use std::os::unix::fs::MetadataExt;
    use std::process::Stdio;
    use std::thread;
    use std::time::{Duration, Instant};

    let example = Example::new("multi-firmware");
    let config = example.root.join("polyfw.toml");
    let source = fs::read_to_string(&config).unwrap();
    fs::write(&config, source.replace("enabled = false", "enabled = true")).unwrap();
    example.run(&["select", "led-demo", "--dev"]);

    let selection = example.root.join(".polyfw-state/selected");
    let launch = example.root.join(".vscode/launch.json");
    let lock_path = example.root.join(".polyfw-state/selection.lock");
    let run_while_locked = |args: &[&str]| {
        let lock = fs::File::options().write(true).open(&lock_path).unwrap();
        lock.lock().unwrap();
        let inode_suffix = format!(":{}", lock.metadata().unwrap().ino());
        let previous_selection = fs::read(&selection).unwrap();
        let previous_launch = fs::read(&launch).unwrap();
        let writer = example
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let writer_pid = writer.id().to_string();
        let mut writer = DropGuard::new(writer, |mut writer| {
            let _ = writer.kill();
            let _ = writer.wait();
        });

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if writer.try_wait().unwrap().is_some() {
                let output = DropGuard::dismiss(writer).wait_with_output().unwrap();
                panic!(
                    "{args:?} exited before waiting for {}: {output:?}",
                    lock_path.display()
                );
            }
            let locks = fs::read_to_string("/proc/locks").unwrap();
            let waiting = locks.lines().any(|record| {
                let mut fields = record.split_whitespace();
                // A blocked entry has an arrow before the lock type, policy and mode.
                fields.nth(1) == Some("->")
                    && fields.nth(3) == Some(writer_pid.as_str())
                    && fields
                        .next()
                        .is_some_and(|file| file.ends_with(&inode_suffix))
            });
            if waiting {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{args:?} never reached {}",
                lock_path.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fs::read(&selection).unwrap(), previous_selection);
        assert_eq!(fs::read(&launch).unwrap(), previous_launch);
        drop(lock);

        let deadline = Instant::now() + Duration::from_secs(30);
        while writer.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "{args:?} did not finish after lock release"
            );
            thread::sleep(Duration::from_millis(10));
        }
        succeeded(DropGuard::dismiss(writer).wait_with_output().unwrap());
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
    for (platform, description, chip) in [
        ("board-a", "board-A", "STM32H723VGTx"),
        ("board-b", "board-B", "STM32F407IG"),
    ] {
        assert_eq!(
            report["platforms"][platform]["target"],
            "thumbv7em-none-eabihf"
        );
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
        for field in ["target", "chip"] {
            assert_eq!(firmware[field], details["platforms"][platform][field]);
        }
        assert_eq!(
            Path::new(firmware["manifest_path"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            example
                .root
                .join("firmwares")
                .join(package)
                .join("Cargo.toml")
        );
        assert_eq!(
            Path::new(firmware["workspace"].as_str().unwrap()),
            example.root.join("bsp").join(platform)
        );
        assert_eq!(firmware["working_dir"], firmware["workspace"]);
        let target_dir = example.root.join("target/firmware").join(platform);
        fs::create_dir_all(&target_dir).unwrap();
        assert_eq!(
            Path::new(firmware["target_dir"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            target_dir
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
                example.root.join("shared").join(name).join("Cargo.toml")
            );
            assert_eq!(dependency["target_clippy"], true);
            assert!(dependency.get("target").is_none());
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
fn a_dependency_member_uses_its_owners_toolchain_for_host_tests() {
    let example = Example::new("multi-workspace");
    let member = example.root.join("shared/patterns");
    let unavailable_toolchain = member.join("unavailable-toolchain");
    fs::create_dir(&unavailable_toolchain).unwrap();
    let path = toml::Value::String(unavailable_toolchain.display().to_string());
    fs::write(
        member.join("rust-toolchain.toml"),
        format!("[toolchain]\npath = {path}\n"),
    )
    .unwrap();
    failed(
        support::workspace_cargo(&member)
            .arg("--version")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .output()
            .unwrap(),
        "unavailable-toolchain",
    );

    let details = example.json(&["info", "--firmware", "led-1", "--json"]);
    let dependencies = details["firmwares"]["led-1"]["local_dependencies"]
        .as_array()
        .unwrap();
    assert!(
        dependencies
            .iter()
            .any(|dependency| dependency["package"] == "patterns"),
        "{details}"
    );
    let output = example.run(&["test", "led-1"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("test tests::pulse_patterns_have_nonzero_on_and_off_durations ... ok"),
        "{stdout}"
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
