//! Generic YAML-frontmatter → MCP `_meta` passthrough.
//!
//! `src/skills/loader.rs` already keeps every `SKILL.md` frontmatter
//! key losslessly on `Skill.frontmatter` (a `serde_yaml::Value`). This
//! module is the ONE place that projects the whole frontmatter onto
//! the wire — resource `_meta` (`super::super::serve::skill_meta` call
//! sites) and tool `frontmatter` output both read through
//! [`frontmatter_json`] / [`skill_meta`], so a new frontmatter field
//! reaches the agent with zero server changes. See the "Skills MCP
//! metadata passthrough" design doc for the carrier decision.

use rmcp::model::Meta;
use serde_json::{Map, Value};

use super::super::serve::LoadedSkill;

/// Namespaced `_meta` key carrying the ENTIRE frontmatter map,
/// verbatim keys — no camelCasing. The consumer interprets; renaming
/// is interpretation we deliberately don't do server-side.
pub const META_KEY_FRONTMATTER: &str = "io.hyprpilot/frontmatter";

/// Namespaced `_meta` key carrying today's curated/derived skill view
/// (name / interaction / argument-hint / disable-model-invocation /
/// references / path / bundleDir). Kept alongside the raw frontmatter
/// for consumers that want the pre-derived shape without re-deriving
/// `references` / `path` / `bundleDir` themselves.
pub const META_KEY_SKILL: &str = "io.hyprpilot/skill";

/// Losslessly convert parsed YAML frontmatter into a JSON object.
/// Nested maps, arrays, bools, numbers, and strings all convert; key
/// names pass through VERBATIM — no camelCasing. Absent frontmatter
/// (`Value::Null`, e.g. a `SKILL.md` with no `---` fence, or a fence
/// that failed to parse and fell back to `Null` per
/// `skills/loader.rs::split_frontmatter`) converts to an empty object.
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
                "mcp::server::skills::metadata: frontmatter is not map-shaped — treating as empty"
            );
            Map::new()
        }
        Err(err) => {
            tracing::warn!(
                %err,
                "mcp::server::skills::metadata: frontmatter -> JSON conversion failed — treating as empty"
            );
            Map::new()
        }
    }
}

/// Build the `_meta` object for a loaded skill's MCP resource: the
/// verbatim whole frontmatter under [`META_KEY_FRONTMATTER`] plus
/// today's curated view under [`META_KEY_SKILL`]. Both keys are
/// namespaced per the MCP spec's `_meta` convention (reverse-DNS
/// prefix + `/name`) rather than riding as a bare `skill` key — see
/// the design doc's carrier decision.
#[must_use]
pub fn skill_meta(skill: &LoadedSkill) -> Meta {
    let mut meta = Map::new();
    meta.insert(
        META_KEY_FRONTMATTER.to_string(),
        Value::Object(skill.frontmatter_json.clone()),
    );
    meta.insert(
        META_KEY_SKILL.to_string(),
        serde_json::to_value(&skill.metadata).expect("skill metadata serializes"),
    );
    Meta(meta)
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
