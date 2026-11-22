use std::collections::BTreeMap;
use std::path::Path;

use polyfw::ProjectInfo;
use serde::Serialize;

#[derive(Serialize)]
struct InfoV1<'a> {
    repository_root: &'a Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_selection: Option<Selection<'a>>,
    platforms: BTreeMap<&'a str, Platform<'a>>,
    firmwares: BTreeMap<&'a str, Firmware<'a>>,
}

#[derive(Serialize)]
struct Selection<'a> {
    firmware: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<&'a str>,
}

#[derive(Serialize)]
struct Platform<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    registered_firmwares: usize,
    target: &'a str,
    cpu: &'a str,
    chip: &'a str,
}

#[derive(Serialize)]
struct Firmware<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    platform: &'a str,
    package: &'a str,
    bin: &'a str,
    workspace: &'a Path,
    working_dir: &'a Path,
    target_dir: &'a Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_dir: Option<&'a Path>,
    target: &'a str,
    check_platform: bool,
    artifacts: Vec<Artifact<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_dependencies: Option<Vec<Dependency<'a>>>,
}

#[derive(Serialize)]
struct Artifact<'a> {
    profile: &'a str,
    path: &'a Path,
    exists: bool,
}

#[derive(Serialize)]
struct Dependency<'a> {
    package: &'a str,
    manifest_path: &'a Path,
    target: &'static str,
}

pub(crate) fn info_v1(report: &ProjectInfo) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&InfoV1 {
        repository_root: &report.root,
        saved_selection: report.saved_selection.as_ref().map(|saved| Selection {
            firmware: &saved.firmware,
            profile: saved.profile.as_ref().map(polyfw::Profile::name),
        }),
        platforms: report
            .platforms
            .iter()
            .map(|(id, platform)| {
                (
                    id.as_str(),
                    Platform {
                        description: platform.description.as_deref(),
                        registered_firmwares: platform.registered_firmwares,
                        target: &platform.target,
                        cpu: &platform.cpu,
                        chip: &platform.chip,
                    },
                )
            })
            .collect(),
        firmwares: report
            .firmwares
            .iter()
            .map(|(id, firmware)| {
                (
                    id.as_str(),
                    Firmware {
                        description: firmware.description.as_deref(),
                        platform: &firmware.platform,
                        package: &firmware.package,
                        bin: &firmware.bin,
                        workspace: &firmware.workspace,
                        working_dir: &firmware.working_dir,
                        target_dir: &firmware.target_dir,
                        build_dir: firmware.build_dir.as_deref(),
                        target: &firmware.target,
                        check_platform: firmware.check_platform,
                        artifacts: firmware
                            .artifacts
                            .iter()
                            .map(|artifact| Artifact {
                                profile: artifact
                                    .profile
                                    .as_ref()
                                    .map_or("dev", polyfw::Profile::name),
                                path: &artifact.path,
                                exists: artifact.exists,
                            })
                            .collect(),
                        local_dependencies: firmware.local_dependencies.as_ref().map(|packages| {
                            packages
                                .iter()
                                .map(|package| Dependency {
                                    package: &package.name,
                                    manifest_path: &package.manifest_path,
                                    target: if package.embedded { "embedded" } else { "host" },
                                })
                                .collect()
                        }),
                    },
                )
            })
            .collect(),
    })
}
