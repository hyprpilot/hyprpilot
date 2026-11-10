//! Directory layers: `.hyprpilot.<ext>` files met walking up from the
//! launch directory, folded after the config-layer overlays. A file is
//! read only when `project_config.trusted` covers the folder holding it —
//! a directory layer can add stdio MCP servers and agent commands, so a
//! cloned repository shipping one must not run anything on its own.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use garde::Validate;
use globset::{Glob, GlobSet, GlobSetBuilder};
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::extensions::validate_globs;
use super::merge_strategies::overwrite_some;
use crate::paths;

const STEM: &str = ".hyprpilot";

/// `project_config` — which folders may contribute directory layers.
/// Read from the root config and the overlays only; a directory layer
/// that sets it fails the load, so a folder can never trust itself.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ProjectConfig {
    /// Globs over folder paths; `~` and `${VAR}` expand first. `*`
    /// crosses `/` as it does in `$match.profile`. `~/src/x/**` misses
    /// `~/src/x` itself, and globset drops an empty alternative, so
    /// `~/src/x{,/**}` misses it too: list `~/src/x` and `~/src/x/**`.
    #[garde(custom(validate_globs))]
    pub trusted: Option<Vec<String>>,
}

impl ProjectConfig {
    fn matcher(&self) -> Result<Option<GlobSet>> {
        let Some(patterns) = self.trusted.as_ref().filter(|patterns| !patterns.is_empty()) else {
            return Ok(None);
        };
        let mut builder = GlobSetBuilder::new();
        for raw in patterns {
            let expanded = paths::expand_env_value(raw, "project_config.trusted", |name| std::env::var(name).ok());
            builder.add(Glob::new(&expanded).with_context(|| format!("project_config.trusted: invalid glob '{raw}'"))?);
        }

        Ok(Some(builder.build()?))
    }
}

/// One `.hyprpilot.*` file met on the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryLayer {
    pub path: PathBuf,
    /// `false` when no trusted glob covers the folder; the file is
    /// reported and never read.
    pub trusted: bool,
}

/// The `.hyprpilot.*` files from `/` down to `anchor`, outermost folder
/// first. Within a folder the plain file comes first, then one
/// `.hyprpilot.<overlay>.*` per overlay in `overlays` order. Nothing is
/// trusted means no walk at all.
pub fn discover(anchor: &Path, overlays: &[String], project: &ProjectConfig) -> Result<Vec<DirectoryLayer>> {
    let Some(trusted) = project.matcher()? else {
        return Ok(Vec::new());
    };
    let stems: Vec<String> = std::iter::once(STEM.to_string())
        .chain(overlays.iter().map(|overlay| format!("{STEM}.{overlay}")))
        .collect();
    let mut dirs: Vec<&Path> = anchor.ancestors().collect();
    dirs.reverse();

    let mut layers = Vec::new();
    for dir in dirs {
        let dir_trusted = trusted.is_match(dir);
        for stem in &stems {
            let found = paths::existing_config_files(dir, stem);
            if !dir_trusted {
                layers.extend(found.into_iter().map(|path| DirectoryLayer { path, trusted: false }));
                continue;
            }
            let single = paths::single_config_file(found, || {
                format!("multiple '{stem}' config files exist in {}", dir.display())
            })?;
            layers.extend(single.map(|path| DirectoryLayer { path, trusted: true }));
        }
    }

    Ok(layers)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn trusting(patterns: &[&str]) -> ProjectConfig {
        ProjectConfig {
            trusted: Some(patterns.iter().map(|p| p.to_string()).collect()),
        }
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, "").expect("write");
    }

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let anchor = root.path().join("outer/inner");
        fs::create_dir_all(&anchor).expect("mkdir");

        (root, anchor)
    }

    #[test]
    fn nothing_trusted_skips_the_walk() {
        let (_root, anchor) = tree();
        touch(&anchor.join(".hyprpilot.yaml"));

        for project in [ProjectConfig::default(), trusting(&[])] {
            assert!(discover(&anchor, &[], &project).expect("discover").is_empty());
        }
    }

    #[test]
    fn walks_outermost_first_with_overlays_after_the_plain_file() {
        let (root, anchor) = tree();
        let outer = root.path().join("outer");
        touch(&outer.join(".hyprpilot.toml"));
        touch(&anchor.join(".hyprpilot.work.json"));
        touch(&anchor.join(".hyprpilot.yaml"));
        touch(&anchor.join(".hyprpilot.personal.yml"));
        let project = trusting(&[&outer.display().to_string(), &format!("{}/**", outer.display())]);

        let paths: Vec<PathBuf> = discover(&anchor, &["personal".into(), "work".into()], &project)
            .expect("discover")
            .into_iter()
            .inspect(|layer| assert!(layer.trusted))
            .map(|layer| layer.path)
            .collect();

        assert_eq!(
            paths,
            vec![
                outer.join(".hyprpilot.toml"),
                anchor.join(".hyprpilot.yaml"),
                anchor.join(".hyprpilot.personal.yml"),
                anchor.join(".hyprpilot.work.json"),
            ]
        );
    }

    #[test]
    fn inactive_overlay_variants_are_ignored() {
        let (root, anchor) = tree();
        touch(&anchor.join(".hyprpilot.work.yaml"));
        let project = trusting(&[&format!("{}/**", root.path().display())]);

        assert!(discover(&anchor, &["personal".into()], &project)
            .expect("discover")
            .is_empty());
    }

    #[test]
    fn untrusted_folders_are_reported_not_trusted() {
        let (root, anchor) = tree();
        let outer = root.path().join("outer");
        touch(&outer.join(".hyprpilot.yaml"));
        touch(&anchor.join(".hyprpilot.yaml"));
        let project = trusting(&[&format!("{}/**", outer.display())]);

        let layers = discover(&anchor, &[], &project).expect("discover");

        assert_eq!(
            layers,
            vec![
                DirectoryLayer {
                    path: outer.join(".hyprpilot.yaml"),
                    trusted: false,
                },
                DirectoryLayer {
                    path: anchor.join(".hyprpilot.yaml"),
                    trusted: true,
                },
            ]
        );
    }

    #[test]
    fn two_formats_in_one_trusted_folder_fail() {
        let (root, anchor) = tree();
        touch(&anchor.join(".hyprpilot.toml"));
        touch(&anchor.join(".hyprpilot.yaml"));
        let project = trusting(&[&format!("{}/**", root.path().display())]);

        let err = discover(&anchor, &[], &project).expect_err("ambiguous");

        assert!(err.to_string().contains("keep exactly one of"), "{err}");
    }

    #[test]
    fn trusted_globs_expand_env_vars() {
        let (root, anchor) = tree();
        touch(&anchor.join(".hyprpilot.yaml"));
        std::env::set_var("HYPRPILOT_TEST_TRUSTED_ROOT", root.path());
        let project = trusting(&["${HYPRPILOT_TEST_TRUSTED_ROOT}/**"]);

        let layers = discover(&anchor, &[], &project).expect("discover");
        std::env::remove_var("HYPRPILOT_TEST_TRUSTED_ROOT");

        assert_eq!(layers.len(), 1);
        assert!(layers[0].trusted);
    }
}
