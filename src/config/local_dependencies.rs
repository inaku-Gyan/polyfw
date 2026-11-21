use std::path::{Component, Path, PathBuf};

use glob::{MatchOptions, Pattern};

use crate::InternalResult as Result;

pub(crate) struct LocalDependencies {
    directories: Vec<DirectoryPattern>,
}

struct DirectoryPattern {
    base: PathBuf,
    include_base: bool,
    patterns: Vec<Pattern>,
}

impl LocalDependencies {
    pub(crate) fn new(project_root: &Path, patterns: &[String]) -> Result<Self> {
        let directories = patterns
            .iter()
            .map(|pattern| {
                DirectoryPattern::new(project_root, pattern).map_err(|error| {
                    format!("invalid local-dependencies pattern {pattern:?}: {error}").into()
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self { directories })
    }

    pub(crate) fn contains(&self, directory: &Path) -> Result<bool> {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        for entry in &self.directories {
            if let Ok(relative) = directory.strip_prefix(&entry.base) {
                if relative.as_os_str().is_empty() {
                    if entry.include_base {
                        return Ok(true);
                    }
                    continue;
                }
                let relative = relative.to_str().ok_or_else(|| {
                    format!("package directory is not UTF-8: {}", directory.display())
                })?;
                if entry
                    .patterns
                    .iter()
                    .any(|pattern| pattern.matches_with(relative, options))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

impl DirectoryPattern {
    fn new(project_root: &Path, source: &str) -> Result<Self> {
        Pattern::new(source)?;
        let path = Path::new(source);
        let mut base = if path.is_absolute() {
            PathBuf::new()
        } else {
            project_root.to_owned()
        };
        let mut remainder = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => base.push(component),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !remainder.is_empty() {
                        return Err(".. must precede any wildcard".into());
                    }
                    base.push(component);
                }
                Component::Normal(part) => {
                    let part = part.to_str().ok_or("directory pattern is not UTF-8")?;
                    if remainder.is_empty() && !part.contains(['*', '?', '[']) {
                        base.push(part);
                    } else {
                        remainder.push(part);
                    }
                }
            }
        }
        let base = match base.canonicalize() {
            Ok(canonical) => canonical,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut normalized = PathBuf::new();
                for component in base.components() {
                    if component == Component::ParentDir {
                        normalized.pop();
                    } else {
                        normalized.push(component);
                    }
                }
                normalized
            }
            Err(error) => return Err(format!("{}: {error}", base.display()).into()),
        };
        let include_base = remainder.iter().all(|part| *part == "**");
        let mut source = remainder.join("/");
        let mut patterns = vec![Pattern::new(&source)?];
        // A trailing /** also includes the directory before its separator.
        while let Some(parent) = source.strip_suffix("/**") {
            patterns.push(Pattern::new(parent)?);
            source = parent.to_owned();
        }
        Ok(Self {
            base,
            include_base,
            patterns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_globs_distinguish_exact_immediate_and_recursive_packages() {
        let root = std::env::current_dir().unwrap().join("project[one]");
        for (pattern, accepted, rejected) in [
            (
                "./**",
                vec!["", "shared", "shared/one", ".hidden/one"],
                vec![],
            ),
            (
                "shared",
                vec!["shared"],
                vec!["shared/one", "shared/one/two"],
            ),
            ("./shared/./", vec!["shared"], vec!["shared/one"]),
            (
                "shared/*",
                vec!["shared/one"],
                vec!["shared", "shared/one/two"],
            ),
            (
                "shared/**",
                vec!["shared", "shared/one", "shared/one/two"],
                vec!["other"],
            ),
            (
                "shared/*/**",
                vec!["shared/one", "shared/one/two"],
                vec!["shared"],
            ),
            (
                "shared/**/**",
                vec!["shared", "shared/one", "shared/one/two"],
                vec!["other"],
            ),
            (
                "**/one",
                vec!["one", "shared/one"],
                vec!["shared/two", "one/two"],
            ),
            (
                "shared/[ab]?",
                vec!["shared/a1", "shared/b2"],
                vec!["shared/c1", "shared/a1/two"],
            ),
        ] {
            let matcher = LocalDependencies::new(&root, &[pattern.into()]).unwrap();
            for path in accepted {
                assert!(
                    matcher.contains(&root.join(path)).unwrap(),
                    "{pattern}: {path}"
                );
            }
            for path in rejected {
                assert!(
                    !matcher.contains(&root.join(path)).unwrap(),
                    "{pattern}: {path}"
                );
            }
            assert!(
                !matcher.contains(&root.with_extension("external")).unwrap(),
                "{pattern}"
            );
        }
    }

    #[test]
    fn relative_and_absolute_patterns_can_select_external_directories() {
        let root = std::env::current_dir().unwrap().join("project");
        let external = root.parent().unwrap().join("external");
        for pattern in [PathBuf::from("../external/**"), external.join("**")] {
            let matcher =
                LocalDependencies::new(&root, &[pattern.to_str().unwrap().into()]).unwrap();
            assert!(matcher.contains(&external).unwrap());
            assert!(matcher.contains(&external.join("one/two")).unwrap());
            assert!(!matcher.contains(&root).unwrap());
        }
        assert!(
            !LocalDependencies::new(&root, &[])
                .unwrap()
                .contains(&root)
                .unwrap()
        );
        for pattern in ["[", "shared/**suffix", "shared/**/../other"] {
            let error = LocalDependencies::new(&root, &[pattern.into()])
                .err()
                .unwrap();
            assert!(error.to_string().contains(pattern), "{error}");
        }
    }
}
