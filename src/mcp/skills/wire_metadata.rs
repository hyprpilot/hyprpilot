//! Generic YAML-frontmatter → MCP `_meta` passthrough.
//!
//! the sibling `loader.rs` already keeps every `SKILL.md` frontmatter
//! key losslessly on `Skill.frontmatter` (a `serde_yaml::Value`). This
//! module is the ONE place that projects the frontmatter onto the wire.
//!
//! Per the MCP spec `_meta` is a single field keyed by reverse-DNS
//! names; multiple keys are allowed but not required. We emit exactly
//! ONE — [`META_KEY_SKILL`] — and never repeat anything already carried
//! by the spec-compliant `Resource` fields (`title` / `description` /
//! `mimeType` / `size` / `name`). The block is the verbatim frontmatter
//! MINUS `title` + `description` (byte-for-byte equal to
//! `Resource.title` / `Resource.description`) PLUS the runtime-derived
//! `path` + `bundleDir` (which are NOT in the frontmatter). A new
//! frontmatter key reaches the agent verbatim with zero server changes.
//!
//! `name` is deliberately KEPT: `Resource.name` is the SLUG, while a
//! frontmatter `name` is an author-supplied value that may differ — so
//! it is not a spec duplicate. Only fields that are byte-for-byte the
//! spec value (`title`, `description`) are dropped.

use std::path::Path;

use rmcp::model::MetaObject;
use serde_json::{Map, Value};

/// The single namespaced `_meta` key. Carries the merged skill block —
/// frontmatter-verbatim (minus `title`/`description`) plus runtime
/// `path` + `bundleDir`. Reverse-DNS-namespaced per the MCP `_meta`
/// convention.
pub const META_KEY_SKILL: &str = "io.hyprpilot/skill";

/// Losslessly convert parsed YAML frontmatter into a JSON object.
/// Nested maps, arrays, bools, numbers, and strings all convert; key
/// names pass through VERBATIM — no camelCasing. Absent frontmatter
/// (`Value::Null`, e.g. a `SKILL.md` with no `---` fence, or a fence
/// that failed to parse and fell back to `Null` per
/// `loader.rs::split_frontmatter`) converts to an empty object.
///
/// A conversion failure — frontmatter that parsed as YAML but isn't
/// map-shaped at the top level, or a mapping key JSON can't represent
/// (JSON object keys must be strings; YAML mapping keys don't have to
/// be) — logs a warning and returns an empty object. This mirrors the
/// loader's own bad-frontmatter policy: never fail the request over a
/// malformed frontmatter block, just treat it as absent.
#[must_use]
pub fn frontmatter_json(frontmatter: &serde_yaml::Value) -> Map<String, Value> {
    match serde_json::to_value(frontmatter) {
        Ok(Value::Object(map)) => map,
        Ok(Value::Null) => Map::new(),
        Ok(other) => {
            tracing::warn!(
                value = %other,
                "mcp::skills::wire_metadata: frontmatter is not map-shaped — treating as empty"
            );
            Map::new()
        }
        Err(err) => {
            tracing::warn!(
                %err,
                "mcp::skills::wire_metadata: frontmatter -> JSON conversion failed — treating as empty"
            );
            Map::new()
        }
    }
}

/// Build the single merged skill block: the verbatim frontmatter map
/// MINUS `title` + `description` (already carried byte-for-byte by
/// `Resource.title` / `Resource.description`) PLUS the runtime-derived
/// `path` + `bundleDir` (which are NOT in the frontmatter). Frontmatter
/// `name` is KEPT — `Resource.name` is the SLUG, not the same value.
#[must_use]
pub fn skill_block(frontmatter: &Map<String, Value>, path: &Path) -> Map<String, Value> {
    let mut block = frontmatter.clone();
    // The ONLY spec-duplicated keys: dropped because they equal the
    // canonical `Resource.title` / `Resource.description`.
    block.remove("title");
    block.remove("description");
    // Dropped for the same reason, one layer out: the resolved
    // reference MANIFEST carries every declared reference with the
    // canonical path that actually addresses it. The raw array holds
    // the DECLARED spelling (`../references/output-diff.md`), which is
    // meaningless outside its bundle dir and cannot be passed to
    // `load_skill_references` — publishing both would offer a caller
    // two addresses, only one of which works.
    block.remove("references");
    // Runtime-derived, not present in frontmatter.
    block.insert("path".to_string(), Value::String(path.display().to_string()));
    if let Some(parent) = path.parent() {
        block.insert("bundleDir".to_string(), Value::String(parent.display().to_string()));
    }
    super::wire_time::FileStat::read(path).extend(&mut block);
    block
}

/// Wrap the merged skill block under the single namespaced `_meta`
/// key ([`META_KEY_SKILL`]). One key, per the MCP `_meta` convention —
/// spec `Resource` fields are canonical and never repeated here.
#[must_use]
pub fn skill_meta(block: &Map<String, Value>) -> MetaObject {
    let mut meta = Map::new();
    meta.insert(META_KEY_SKILL.to_string(), Value::Object(block.clone()));
    MetaObject(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_json_converts_nested_maps_arrays_bools_and_numbers() {
        let value: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: plan-hard
disable-model-invocation: true
retries: 3
metadata:
  owner: captain
  tags:
    - alpha
    - beta
  weight: 1.5
"#,
        )
        .unwrap();

        let json = frontmatter_json(&value);

        assert_eq!(json.get("name").and_then(Value::as_str), Some("plan-hard"));
        assert_eq!(
            json.get("disable-model-invocation").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(json.get("retries").and_then(Value::as_i64), Some(3));
        let metadata = json.get("metadata").and_then(Value::as_object).unwrap();
        assert_eq!(metadata.get("owner").and_then(Value::as_str), Some("captain"));
        assert_eq!(metadata.get("tags").and_then(Value::as_array).map(|a| a.len()), Some(2));
        assert_eq!(metadata.get("weight").and_then(Value::as_f64), Some(1.5));
    }

    #[test]
    fn frontmatter_json_null_becomes_empty_object() {
        assert_eq!(frontmatter_json(&serde_yaml::Value::Null), Map::new());
    }

    #[test]
    fn frontmatter_json_arbitrary_unknown_key_survives_verbatim() {
        let value: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: my-skill
x-vendor-extension:
  nested:
    - one
    - two
  flag: false
"#,
        )
        .unwrap();

        let json = frontmatter_json(&value);

        let ext = json.get("x-vendor-extension").and_then(Value::as_object).unwrap();
        assert_eq!(
            ext.get("nested"),
            Some(&Value::Array(vec![
                Value::String("one".into()),
                Value::String("two".into())
            ]))
        );
        assert_eq!(ext.get("flag"), Some(&Value::Bool(false)));
    }

    #[test]
    fn skill_block_drops_title_description_keeps_name_adds_path_and_bundle_dir() {
        let value: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: plan-hard
title: Plan hard
description: Deep planning
argument-hint: "[goal]"
x-vendor-extension:
  flag: true
"#,
        )
        .unwrap();
        let block = skill_block(&frontmatter_json(&value), Path::new("/tmp/plan-hard/SKILL.md"));

        // Spec-duplicated keys are gone.
        assert!(!block.contains_key("title"));
        assert!(!block.contains_key("description"));
        // Frontmatter `name` is NOT a spec duplicate (Resource.name is
        // the slug) — it survives.
        assert_eq!(block.get("name").and_then(Value::as_str), Some("plan-hard"));
        // Any other verbatim frontmatter key survives.
        assert_eq!(block.get("argument-hint").and_then(Value::as_str), Some("[goal]"));
        assert_eq!(
            block.get("x-vendor-extension"),
            Some(&serde_json::json!({ "flag": true }))
        );
        // Runtime-derived keys added.
        assert_eq!(
            block.get("path").and_then(Value::as_str),
            Some("/tmp/plan-hard/SKILL.md")
        );
        assert_eq!(block.get("bundleDir").and_then(Value::as_str), Some("/tmp/plan-hard"));
    }

    #[test]
    fn skill_meta_has_single_namespaced_key_without_frontmatter_key() {
        let block = skill_block(
            &frontmatter_json(&serde_yaml::from_str("name: plan-hard\n").unwrap()),
            Path::new("/tmp/plan-hard/SKILL.md"),
        );
        let meta = skill_meta(&block);

        // Exactly one key — no legacy `io.hyprpilot/frontmatter`, no
        // bare `skill`.
        assert_eq!(meta.len(), 1);
        assert!(meta.get("io.hyprpilot/frontmatter").is_none());
        assert!(meta.get("skill").is_none());
        assert_eq!(
            meta.get("io.hyprpilot/skill")
                .and_then(|v| v.get("name"))
                .and_then(Value::as_str),
            Some("plan-hard")
        );
    }

    /// The raw `references` array is dropped: the resolved manifest
    /// carries every declared reference by its CANONICAL path, while the
    /// raw array holds the declared spelling — which is not an address
    /// and cannot be passed to `load_skill_references`.
    #[test]
    fn skill_block_drops_the_raw_references_array() {
        let value: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: git-commit
references:
  - ../references/output-diff.md
"#,
        )
        .unwrap();

        let block = skill_block(&frontmatter_json(&value), Path::new("/tmp/git-commit/SKILL.md"));

        assert!(!block.contains_key("references"));
        let rendered = serde_json::to_string(&block).unwrap();
        assert!(
            !rendered.contains("output-diff.md"),
            "no declared reference path may reach the wire: {rendered}"
        );
    }

    /// A real file contributes its timestamps; a path that does not
    /// resolve simply omits them rather than failing or faking a value.
    #[test]
    fn skill_block_carries_timestamps_for_a_real_file_and_omits_them_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, "body").unwrap();

        let block = skill_block(&Map::new(), &path);
        assert_eq!(block.get("size").and_then(Value::as_u64), Some(4));
        assert!(block.get("modified").and_then(Value::as_str).unwrap().ends_with('Z'));

        let absent = skill_block(&Map::new(), Path::new("/nonexistent-hyprpilot-xyz/SKILL.md"));
        assert!(!absent.contains_key("modified"));
        assert!(!absent.contains_key("size"));
    }

    #[test]
    fn frontmatter_json_conversion_failure_falls_back_to_empty() {
        // JSON object keys must be strings; a YAML mapping key that is
        // itself a sequence has no JSON representation.
        let mut mapping = serde_yaml::Mapping::new();
        let bad_key = serde_yaml::Value::Sequence(vec![serde_yaml::Value::from(1)]);
        mapping.insert(bad_key, serde_yaml::Value::from("x"));
        let value = serde_yaml::Value::Mapping(mapping);

        assert_eq!(frontmatter_json(&value), Map::new());
    }
}
