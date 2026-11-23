use std::collections::BTreeMap;
use std::path::Path;

use polyfw::{FirmwareInfo, ProjectInfo};
use serde::Serialize;

#[derive(Serialize)]
struct InfoV1<'a> {
    repository_root: &'a Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_selection: Option<Selection<'a>>,
    platforms: BTreeMap<&'a str, Platform<'a>>,
    firmwares: BTreeMap<&'a str, FirmwareV1<'a>>,
}

#[derive(Serialize)]
struct InfoV2<'a> {
    format_version: u32,
    project_root: &'a Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_selection: Option<Selection<'a>>,
    platforms: BTreeMap<&'a str, Platform<'a>>,
    firmwares: BTreeMap<&'a str, FirmwareV2<'a>>,
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
struct FirmwareFields<'a> {
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
}

impl<'a> From<&'a FirmwareInfo> for FirmwareFields<'a> {
    fn from(firmware: &'a FirmwareInfo) -> Self {
        Self {
            description: firmware.description.as_deref(),
            platform: &firmware.platform,
            package: &firmware.package,
            bin: &firmware.bin,
            workspace: &firmware.workspace,
            working_dir: &firmware.working_dir,
            target_dir: &firmware.target_dir,
            build_dir: firmware.build_dir.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct FirmwareV1<'a> {
    #[serde(flatten)]
    fields: FirmwareFields<'a>,
    target: &'a str,
    check_platform: bool,
    artifacts: Vec<ArtifactV1<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_dependencies: Option<Vec<DependencyV1<'a>>>,
}

#[derive(Serialize)]
struct FirmwareV2<'a> {
    #[serde(flatten)]
    fields: FirmwareFields<'a>,
    manifest_path: &'a Path,
    target: &'a str,
    cpu: &'a str,
    chip: &'a str,
    check_platform: bool,
    artifacts: BTreeMap<&'a str, ArtifactV2<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_dependencies: Option<Vec<DependencyV2<'a>>>,
}

#[derive(Serialize)]
struct ArtifactV1<'a> {
    profile: &'a str,
    path: &'a Path,
    exists: bool,
}

#[derive(Serialize)]
struct ArtifactV2<'a> {
    path: &'a Path,
    exists: bool,
}

#[derive(Serialize)]
struct DependencyV1<'a> {
    package: &'a str,
    manifest_path: &'a Path,
    target: &'static str,
}

#[derive(Serialize)]
struct DependencyV2<'a> {
    package: &'a str,
    manifest_path: &'a Path,
    target_clippy: bool,
}

pub(crate) fn info_v1(report: &ProjectInfo) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&InfoV1 {
        repository_root: &report.root,
        saved_selection: saved_selection(report),
        platforms: platforms(report),
        firmwares: report
            .firmwares
            .iter()
            .map(|(id, firmware)| {
                (
                    id.as_str(),
                    FirmwareV1 {
                        fields: firmware.into(),
                        target: &firmware.target,
                        check_platform: firmware.check_platform,
                        artifacts: firmware
                            .artifacts
                            .iter()
                            .map(|artifact| ArtifactV1 {
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
                                .map(|package| DependencyV1 {
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

pub(crate) fn info_v2(report: &ProjectInfo) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&InfoV2 {
        format_version: 2,
        project_root: &report.root,
        saved_selection: saved_selection(report),
        platforms: platforms(report),
        firmwares: report
            .firmwares
            .iter()
            .map(|(id, firmware)| {
                let platform = &report.platforms[&firmware.platform];
                (
                    id.as_str(),
                    FirmwareV2 {
                        fields: firmware.into(),
                        manifest_path: &firmware.manifest_path,
                        target: &platform.target,
                        cpu: &platform.cpu,
                        chip: &platform.chip,
                        check_platform: firmware.check_platform,
                        artifacts: firmware
                            .artifacts
                            .iter()
                            .map(|artifact| {
                                let profile = artifact
                                    .profile
                                    .as_ref()
                                    .map_or("dev", polyfw::Profile::name);
                                (
                                    profile,
                                    ArtifactV2 {
                                        path: &artifact.path,
                                        exists: artifact.exists,
                                    },
                                )
                            })
                            .collect(),
                        local_dependencies: firmware.local_dependencies.as_ref().map(|packages| {
                            let mut dependencies = packages
                                .iter()
                                .map(|package| DependencyV2 {
                                    package: &package.name,
                                    manifest_path: &package.absolute_manifest_path,
                                    target_clippy: package.embedded,
                                })
                                .collect::<Vec<_>>();
                            dependencies.sort_by_key(|dependency| {
                                (dependency.package, dependency.manifest_path)
                            });
                            dependencies
                        }),
                    },
                )
            })
            .collect(),
    })
}

fn saved_selection(report: &ProjectInfo) -> Option<Selection<'_>> {
    report.saved_selection.as_ref().map(|saved| Selection {
        firmware: &saved.firmware,
        profile: saved.profile.as_ref().map(polyfw::Profile::name),
    })
}

fn platforms(report: &ProjectInfo) -> BTreeMap<&str, Platform<'_>> {
    report
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
        .collect()
}
