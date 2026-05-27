//! Strategic merge patch for `Config` overlays — the engine behind
//! the `--with-config` flag (`ctl`) and the `withConfig` RPC field.
//!
//! Walks two `serde_json::Value` trees and folds the right into the
//! left, honouring kustomize-style `$patch` directives. The merger
//! is independent of the `Config` type and operates purely on JSON;
//! the caller is responsible for the round-trip `Config → Value →
//! merge → Value → Config` and re-validation.
//!
//! ## Default rules (no directives present)
//!
//! - **Two objects**: merge field-by-field; recurse on overlapping
//!   keys, take right's value for non-overlapping keys.
//! - **Two arrays of objects, both elements carry an `id`**: keyed
//!   merge — override-by-id, append new ids. Matches the merge
//!   crate's `merge_profiles_by_id` / `merge_agents_by_id`
//!   strategies on the typed side.
//! - **Two arrays of primitives**: append + dedupe (right's entries
//!   concatenated onto left's; duplicates collapsed).
//! - **Mixed types or scalars**: right wins (`overwrite_some` shape).
//!
//! ## Directives (kustomize-style)
//!
//! - `{"$patch": "replace", ...}` inside an object → drop left,
//!   return right with the `$patch` key stripped.
//! - `[{"id": "...", "$patch": "delete"}]` inside an array → drop
//!   the keyed left entry. Identifier key is the `id` field.
//! - `{"$deleteFromPrimitiveList/<field>": [v1, v2]}` inside an
//!   object → remove the listed primitive values from
//!   `left.<field>`.
//!
//! Reach for a wider directive set (`retainKeys`, `setElementOrder`,
//! deleteByPath, …) only when a captain hits a concrete case we
//! can't express with the three above.

use serde_json::{Map, Value};

/// Strip a `$patch` directive from an object map. Returns the
/// directive value (typically `"replace"`).
fn take_patch_directive(obj: &mut Map<String, Value>) -> Option<String> {
    obj.remove("$patch").and_then(|v| match v {
        Value::String(s) => Some(s),
        _ => None,
    })
}

/// Extract `$deleteFromPrimitiveList/<field>` siblings from `right`
/// and apply each to `left`. Mutates both — siblings are removed
/// from `right` so the subsequent default merge doesn't re-process
/// them.
fn apply_primitive_list_deletes(left: &mut Map<String, Value>, right: &mut Map<String, Value>) {
    // Two-pass: collect directive keys (immutable borrow during
    // iteration), then remove each + apply to left.
    let directive_keys: Vec<String> = right
        .keys()
        .filter(|k| k.starts_with("$deleteFromPrimitiveList/"))
        .cloned()
        .collect();

    for dk in directive_keys {
        let Some(to_remove) = right.remove(&dk) else { continue };
        let field = dk.trim_start_matches("$deleteFromPrimitiveList/").to_string();
        let Value::Array(values_to_remove) = to_remove else {
            continue;
        };

        if let Some(Value::Array(existing)) = left.get_mut(&field) {
            existing.retain(|v| !values_to_remove.contains(v));
        }
    }
}

/// `true` iff every element of the array is a JSON object that
/// carries a string `id` field. The signature for "keyed Vec on the
/// Rust side" — matches `[[agents]]` (id: claude-code) and
/// `[[profiles]]` (id: strict).
fn is_keyed_object_array(arr: &[Value]) -> bool {
    !arr.is_empty()
        && arr.iter().all(|v| match v {
            Value::Object(o) => matches!(o.get("id"), Some(Value::String(_))),
            _ => false,
        })
}

/// Keyed-Vec merge: override left entry when right has same id,
/// append new ids. Mirrors `merge_strategies::merge_keyed_by`.
/// Right entries with `$patch: delete` drop the matching left
/// entry instead of overriding.
fn merge_keyed_arrays(mut left: Vec<Value>, right: Vec<Value>) -> Vec<Value> {
    let id_of = |v: &Value| -> Option<String> {
        v.as_object()
            .and_then(|o| o.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
    };

    // Pass 1 — process deletes.
    let (deletes, overrides): (Vec<Value>, Vec<Value>) = right
        .into_iter()
        .partition(|v| v.as_object().and_then(|o| o.get("$patch")).and_then(|p| p.as_str()) == Some("delete"));

    let delete_ids: Vec<String> = deletes.iter().filter_map(id_of).collect();
    left.retain(|l| match id_of(l) {
        Some(id) => !delete_ids.contains(&id),
        None => true,
    });

    // Pass 2 — override / append. Two-list zip-merge.
    let mut overrides: Vec<Option<Value>> = overrides.into_iter().map(Some).collect();
    let mut out: Vec<Value> = Vec::with_capacity(left.len() + overrides.len());

    for l in left {
        let lk = id_of(&l);
        if let Some(idx) = overrides.iter().position(|o| {
            o.as_ref()
                .and_then(id_of)
                .zip(lk.as_ref())
                .is_some_and(|(rk, lk)| rk == *lk)
        }) {
            // Override — merge left's existing object with right's
            // override so a partial right entry doesn't blow away
            // fields it didn't mention.
            let right_value = overrides[idx].take().expect("non-None by position predicate");
            out.push(merge_values(l, right_value));
        } else {
            out.push(l);
        }
    }

    for r in overrides.into_iter().flatten() {
        out.push(r);
    }

    out
}

/// Append + dedupe for primitive arrays. Order: left's items first,
/// then right's not-already-seen items. Dedupe uses JSON equality.
fn merge_primitive_arrays(mut left: Vec<Value>, right: Vec<Value>) -> Vec<Value> {
    for r in right {
        if !left.contains(&r) {
            left.push(r);
        }
    }
    left
}

/// Top-level dispatch — recurses through the tree.
pub fn merge_values(left: Value, right: Value) -> Value {
    match (left, right) {
        (Value::Object(mut left_obj), Value::Object(mut right_obj)) => {
            // Object-level `$patch: replace` — drop left wholesale.
            if let Some(directive) = take_patch_directive(&mut right_obj) {
                if directive == "replace" {
                    return Value::Object(right_obj);
                }
            }

            // Apply primitive-list deletes BEFORE field merge so the
            // remaining right keys merge cleanly.
            apply_primitive_list_deletes(&mut left_obj, &mut right_obj);

            for (k, rv) in right_obj {
                let merged = match left_obj.remove(&k) {
                    Some(lv) => merge_values(lv, rv),
                    None => rv,
                };
                left_obj.insert(k, merged);
            }

            Value::Object(left_obj)
        }

        (Value::Array(left_arr), Value::Array(right_arr)) => {
            // Array-level `$patch: replace` sentinel — `[{"$patch":
            // "replace"}, ...rest]` means "drop left, use rest".
            let mut filtered_right: Vec<Value> = Vec::with_capacity(right_arr.len());
            let mut explicit_replace = false;

            for v in right_arr {
                let is_replace_sentinel = v.as_object().and_then(|o| {
                    if o.len() == 1 {
                        o.get("$patch").and_then(|p| p.as_str())
                    } else {
                        None
                    }
                }) == Some("replace");

                if is_replace_sentinel {
                    explicit_replace = true;
                } else {
                    filtered_right.push(v);
                }
            }

            if explicit_replace {
                return Value::Array(filtered_right);
            }

            // No replace sentinel — decide between keyed and
            // primitive merge based on shape. If either side is a
            // keyed-object array, prefer keyed merge.
            if is_keyed_object_array(&left_arr) || is_keyed_object_array(&filtered_right) {
                Value::Array(merge_keyed_arrays(left_arr, filtered_right))
            } else {
                Value::Array(merge_primitive_arrays(left_arr, filtered_right))
            }
        }

        // Type mismatch or scalar — right wins.
        (_, right) => right,
    }
}

#[cfg(test)]
fn merge_patches(base: Value, patches: Vec<Value>) -> Value {
    patches.into_iter().fold(base, merge_values)
}

/// Fold profile-shaped patches into a resolved profile value,
/// filtered by each patch's optional `$match.profile` glob.
///
/// Each patch is an object whose body is a partial `ProfileConfig`
/// shape. An optional `$match: { profile: "<glob>" }` sibling at the
/// top of the patch object filters which profiles the patch applies
/// to — the directive is stripped before merging so it never lands
/// on the profile shape itself. Unset `$match` (or missing `profile`
/// inside it) means "applies to every profile".
///
/// Non-object patch values silently skip — the caller is expected to
/// have validated the input shape at config-load time (garde +
/// serde), but defensive skipping keeps the helper total.
pub fn apply_profile_patches(profile: Value, patches: &[Value], profile_id: &str) -> Value {
    patches.iter().cloned().fold(profile, |acc, patch_value| {
        let Value::Object(mut patch_obj) = patch_value else {
            return acc;
        };

        // Strip the optional `$match` directive before merging.
        if let Some(match_value) = patch_obj.remove("$match") {
            if !match_matches_profile(&match_value, profile_id) {
                return acc;
            }
        }

        merge_values(acc, Value::Object(patch_obj))
    })
}

/// Backwards-readable alias for root `[[patches]]` call sites.
pub fn apply_root_patches_to_profile(profile: Value, patches: &[Value], profile_id: &str) -> Value {
    apply_profile_patches(profile, patches, profile_id)
}

/// `true` when `match_value` is `null` / absent-but-defaulted, OR its
/// `profile` field is a glob that matches `profile_id`. Anything that
/// fails to parse falls back to `true` so a malformed directive
/// doesn't silently swallow patches — validation surfaces shape
/// errors at config-load via garde.
fn match_matches_profile(match_value: &Value, profile_id: &str) -> bool {
    let Some(obj) = match_value.as_object() else {
        return true;
    };
    let Some(profile_glob) = obj.get("profile").and_then(|v| v.as_str()) else {
        return true;
    };
    globset::Glob::new(profile_glob)
        .ok()
        .map(|g| g.compile_matcher())
        .is_some_and(|m| m.is_match(profile_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn object_field_merge_overlays_scalars() {
        let base = json!({ "a": 1, "b": 2 });
        let patch = json!({ "b": 20, "c": 3 });
        assert_eq!(merge_values(base, patch), json!({ "a": 1, "b": 20, "c": 3 }));
    }

    #[test]
    fn object_field_merge_recurses() {
        let base = json!({ "nested": { "a": 1, "b": 2 } });
        let patch = json!({ "nested": { "b": 20 } });
        assert_eq!(merge_values(base, patch), json!({ "nested": { "a": 1, "b": 20 } }));
    }

    #[test]
    fn object_patch_replace_drops_left() {
        let base = json!({ "agents": { "default": "x", "extra": "kept-on-left" } });
        let patch = json!({ "agents": { "$patch": "replace", "default": "y" } });
        assert_eq!(merge_values(base, patch), json!({ "agents": { "default": "y" } }));
    }

    #[test]
    fn keyed_array_merges_by_id_and_appends_new() {
        let base = json!([
            { "id": "a", "value": 1 },
            { "id": "b", "value": 2 },
        ]);
        let patch = json!([
            { "id": "b", "value": 20 },
            { "id": "c", "value": 3 },
        ]);
        assert_eq!(
            merge_values(base, patch),
            json!([
                { "id": "a", "value": 1 },
                { "id": "b", "value": 20 },
                { "id": "c", "value": 3 },
            ])
        );
    }

    #[test]
    fn keyed_array_merge_preserves_unmentioned_fields_on_existing_entry() {
        let base = json!([{ "id": "a", "x": 1, "y": 2 }]);
        let patch = json!([{ "id": "a", "y": 20 }]);
        assert_eq!(merge_values(base, patch), json!([{ "id": "a", "x": 1, "y": 20 }]));
    }

    #[test]
    fn keyed_array_patch_delete_removes_entry() {
        let base = json!([
            { "id": "a", "v": 1 },
            { "id": "b", "v": 2 },
        ]);
        let patch = json!([{ "id": "a", "$patch": "delete" }]);
        assert_eq!(merge_values(base, patch), json!([{ "id": "b", "v": 2 }]));
    }

    #[test]
    fn primitive_array_appends_and_dedupes() {
        let base = json!(["a", "b"]);
        let patch = json!(["b", "c"]);
        assert_eq!(merge_values(base, patch), json!(["a", "b", "c"]));
    }

    #[test]
    fn array_patch_replace_sentinel_drops_left() {
        let base = json!(["a", "b", "c"]);
        let patch = json!([{ "$patch": "replace" }, "x", "y"]);
        assert_eq!(merge_values(base, patch), json!(["x", "y"]));
    }

    #[test]
    fn delete_from_primitive_list_removes_matching_values() {
        let base = json!({ "skills": { "dirs": ["a", "b", "c"] } });
        let patch = json!({
            "skills": { "$deleteFromPrimitiveList/dirs": ["b"] }
        });
        assert_eq!(merge_values(base, patch), json!({ "skills": { "dirs": ["a", "c"] } }));
    }

    #[test]
    fn delete_from_primitive_list_then_merge_other_siblings() {
        let base = json!({ "skills": { "dirs": ["a", "b"], "other": 1 } });
        let patch = json!({
            "skills": {
                "$deleteFromPrimitiveList/dirs": ["a"],
                "other": 10,
            }
        });
        assert_eq!(
            merge_values(base, patch),
            json!({ "skills": { "dirs": ["b"], "other": 10 } })
        );
    }

    #[test]
    fn type_mismatch_right_wins() {
        let base = json!({ "a": 1 });
        let patch = json!({ "a": [1, 2, 3] });
        assert_eq!(merge_values(base, patch), json!({ "a": [1, 2, 3] }));
    }

    #[test]
    fn scalar_right_wins() {
        assert_eq!(merge_values(json!(1), json!(2)), json!(2));
        assert_eq!(merge_values(json!("a"), json!("b")), json!("b"));
    }

    /// `Option<Vec<T>>` clear-by-null — when a captain writes
    /// `{"skills": null}` in a patch, the merger replaces the
    /// existing array with `null` (right wins on type mismatch).
    /// After serde round-trip, `Config.skills` deserializes to
    /// `None` — the explicit "clear this field" semantic the
    /// captain expects from a patch, distinct from config-layer
    /// `overwrite_some` which keeps the left when the right is
    /// `None` (because `None` there means "layer didn't mention
    /// it" — opposite intent from a patch's explicit null).
    #[test]
    fn option_vec_null_clears_existing_array() {
        let base = serde_json::json!({ "skills": [{ "dir": "/tmp/a" }] });
        let patch = serde_json::json!({ "skills": null });
        assert_eq!(merge_values(base, patch), serde_json::json!({ "skills": null }));
    }

    #[test]
    fn merge_patches_folds_left_to_right() {
        let base = json!({ "a": 1 });
        let p1 = json!({ "b": 2 });
        let p2 = json!({ "b": 20, "c": 3 });
        assert_eq!(merge_patches(base, vec![p1, p2]), json!({ "a": 1, "b": 20, "c": 3 }));
    }

    /// "Add an MCP to the profile's existing mcps list" — the
    /// canonical captain ask from the design interview.
    #[test]
    fn canonical_add_mcp_to_profile() {
        let base = json!({
            "profiles": [
                { "id": "strict", "agent": "claude-code", "mcps": ["/etc/mcp/a.json"] }
            ]
        });
        let patch = json!({
            "profiles": [
                { "id": "strict", "mcps": ["/etc/mcp/b.json"] }
            ]
        });
        assert_eq!(
            merge_values(base, patch),
            json!({
                "profiles": [
                    {
                        "id": "strict",
                        "agent": "claude-code",
                        "mcps": ["/etc/mcp/a.json", "/etc/mcp/b.json"],
                    }
                ]
            })
        );
    }

    // ── apply_root_patches_to_profile ─────────────────────────────

    #[test]
    fn root_patch_without_match_applies_to_every_profile() {
        let profile = json!({ "id": "personal/claude/opus", "agent": "claude-code" });
        let patches = vec![json!({ "model": "opus[1m]" })];
        assert_eq!(
            apply_root_patches_to_profile(profile, &patches, "personal/claude/opus"),
            json!({ "id": "personal/claude/opus", "agent": "claude-code", "model": "opus[1m]" })
        );
    }

    #[test]
    fn root_patch_with_matching_glob_applies() {
        let profile = json!({ "id": "personal/claude/opus", "agent": "claude-code" });
        let patches = vec![json!({
            "$match": { "profile": "personal/*" },
            "mode": "plan"
        })];
        assert_eq!(
            apply_root_patches_to_profile(profile, &patches, "personal/claude/opus"),
            json!({ "id": "personal/claude/opus", "agent": "claude-code", "mode": "plan" })
        );
    }

    #[test]
    fn root_patch_with_non_matching_glob_skips() {
        let profile = json!({ "id": "work/claude/opus", "agent": "claude-code" });
        let patches = vec![json!({
            "$match": { "profile": "personal/*" },
            "mode": "plan"
        })];
        assert_eq!(
            apply_root_patches_to_profile(profile, &patches, "work/claude/opus"),
            json!({ "id": "work/claude/opus", "agent": "claude-code" }),
            "personal/* glob must not match work/* profile"
        );
    }

    #[test]
    fn root_patches_fold_left_to_right_with_per_patch_match() {
        // Three patches: one unscoped (applies to all), two scoped
        // to opposite profile families. Verify ordering + scoping
        // compose correctly.
        let patches = vec![
            json!({ "system_prompt": ["/base.md"] }),
            json!({
                "$match": { "profile": "personal/*" },
                "mcps": [{ "file": "/personal.json" }]
            }),
            json!({
                "$match": { "profile": "work/*" },
                "mcps": [{ "file": "/work.json" }]
            }),
        ];

        let personal = apply_root_patches_to_profile(
            json!({ "id": "personal/claude/opus" }),
            &patches,
            "personal/claude/opus",
        );
        assert_eq!(
            personal,
            json!({
                "id": "personal/claude/opus",
                "system_prompt": ["/base.md"],
                "mcps": [{ "file": "/personal.json" }]
            })
        );

        let work = apply_root_patches_to_profile(json!({ "id": "work/claude/opus" }), &patches, "work/claude/opus");
        assert_eq!(
            work,
            json!({
                "id": "work/claude/opus",
                "system_prompt": ["/base.md"],
                "mcps": [{ "file": "/work.json" }]
            })
        );
    }

    #[test]
    fn root_patch_strips_match_directive_before_merge() {
        // The directive must NOT land on the profile shape — it's
        // metadata for the matcher, not a profile field.
        let profile = json!({ "id": "x" });
        let patches = vec![json!({
            "$match": { "profile": "x" },
            "agent": "claude-code"
        })];
        assert_eq!(
            apply_root_patches_to_profile(profile, &patches, "x"),
            json!({ "id": "x", "agent": "claude-code" }),
            "$match must be consumed and absent from the merged result"
        );
    }

    #[test]
    fn root_patch_non_object_value_is_skipped_silently() {
        // Defensive — config-load validation should reject malformed
        // patches, but if one slips through the helper must not panic.
        let profile = json!({ "id": "x" });
        let patches = vec![json!("not-an-object")];
        assert_eq!(apply_root_patches_to_profile(profile.clone(), &patches, "x"), profile);
    }

    /// Replace one profile's mcps list wholesale (kustomize
    /// `$patch: replace` at the array level).
    #[test]
    fn replace_one_profiles_mcps_via_array_sentinel() {
        let base = json!({
            "profiles": [
                { "id": "strict", "mcps": ["/old.json", "/older.json"] }
            ]
        });
        let patch = json!({
            "profiles": [
                {
                    "id": "strict",
                    "mcps": [{ "$patch": "replace" }, "/fresh.json"],
                }
            ]
        });
        assert_eq!(
            merge_values(base, patch),
            json!({
                "profiles": [
                    { "id": "strict", "mcps": ["/fresh.json"] }
                ]
            })
        );
    }
}
