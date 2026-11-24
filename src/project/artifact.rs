use std::path::{Component, Path, PathBuf};

use crate::cargo::{model::Selection, runner::firmware_elf_path};
use crate::{Error, ErrorKind, Result};

use super::Context;

impl Context {
    pub(super) fn validate_artifact(&self, selection: &Selection, path: &Path) -> Result<()> {
        let resolved_path = path_for_comparison(path);
        let mut conflicts = Vec::new();
        for id in self.cargo.keys() {
            if id == &selection.id {
                continue;
            }
            let mut other = self.selection(id)?;
            other.profile.clone_from(&selection.profile);
            let other_path = firmware_elf_path(&other, None);
            if (other_path == path || path_for_comparison(&other_path) == resolved_path)
                && (selection.platform.chip != other.platform.chip
                    || !same_build(selection, &other))
            {
                conflicts.push(format!("`{id}`"));
            }
        }
        if conflicts.is_empty() {
            return Ok(());
        }
        Err(Error::new(
            ErrorKind::AmbiguousArtifact,
            format!(
                "ambiguous firmware image at {}. Firmware `{}` shares this path with {} across \
                 different Cargo builds or chips. Configure distinct `target-dir` values",
                path.display(),
                selection.id,
                conflicts.join(", ")
            ),
        ))
    }
}

fn same_build(left: &Selection, right: &Selection) -> bool {
    let left_build_dir = left
        .cargo
        .build_dir
        .as_deref()
        .unwrap_or(&left.cargo.target_dir);
    let right_build_dir = right
        .cargo
        .build_dir
        .as_deref()
        .unwrap_or(&right.cargo.target_dir);
    left.resolved_bin.package_id == right.resolved_bin.package_id
        && left.resolved_bin.name == right.resolved_bin.name
        && left.cargo.workspace == right.cargo.workspace
        && left.cargo.working_dir == right.cargo.working_dir
        && left.cargo.target == right.cargo.target
        && (left_build_dir == right_build_dir
            || path_for_comparison(left_build_dir) == path_for_comparison(right_build_dir))
}

fn path_for_comparison(path: &Path) -> PathBuf {
    // The editor can refer to an image before Cargo has created its output directories.
    let Some((ancestor, mut resolved)) = path
        .ancestors()
        .find_map(|ancestor| Some((ancestor, ancestor.canonicalize().ok()?)))
    else {
        return path.to_owned();
    };
    let tail = path.strip_prefix(ancestor).expect("ancestor is a prefix");
    for component in tail.components() {
        if component == Component::ParentDir {
            resolved.pop();
        } else {
            resolved.push(component.as_os_str());
            if let Ok(canonical) = resolved.canonicalize() {
                resolved = canonical;
            }
        }
    }
    resolved
}
