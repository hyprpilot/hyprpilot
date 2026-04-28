//! Custom `merge` crate strategies for the config tree.
//!
//! `overwrite_some` is the per-field default for `Option<T>` leaves —
//! "right wins when Some, else keep left". The crate's stock
//! `option::overwrite_none` is the opposite (left wins). Our layering
//! semantics is later-layer-overrides-earlier so we need overwrite_some
//! everywhere a leaf is `Option<T>`.
//!
//! The three keyed-Vec strategies wrap the keyed-list helpers so
//! `#[derive(Merge)]` plugs them in via the `#[merge(strategy = ...)]`
//! attribute. Algorithm: override by key, append new keys in order.
//! Whole-entry replace — partial field merge inside an entry would
//! surprise.

use super::{AgentConfig, MCPDefinition, ProfileConfig};

/// Right wins on Some; else keep left. Mirrors the old
/// `Option::merge` blanket impl in our hand-rolled trait.
pub(crate) fn overwrite_some<T>(left: &mut Option<T>, right: Option<T>) {
    if right.is_some() {
        *left = right;
    }
}

/// Keyed-Vec merge: for each entry in `left`, if `right` has an entry
/// with the same key, swap in right's. Append right entries whose
/// key isn't in left. Consumes `right`.
fn merge_keyed_by<T, K, F>(left: &mut Vec<T>, right: Vec<T>, key: F)
where
    K: Eq,
    F: Fn(&T) -> K,
{
    let mut overrides: Vec<Option<T>> = right.into_iter().map(Some).collect();
    let mut out: Vec<T> = Vec::with_capacity(left.len() + overrides.len());

    let lefts = std::mem::take(left);
    for b in lefts {
        let bk = key(&b);
        if let Some(idx) = overrides.iter().position(|o| o.as_ref().is_some_and(|t| key(t) == bk)) {
            out.push(overrides[idx].take().expect("non-None by position predicate"));
        } else {
            out.push(b);
        }
    }

    for t in overrides.into_iter().flatten() {
        out.push(t);
    }

    *left = out;
}

pub(crate) fn merge_agents_by_id(left: &mut Vec<AgentConfig>, right: Vec<AgentConfig>) {
    merge_keyed_by(left, right, |a| a.id.clone());
}

pub(crate) fn merge_profiles_by_id(left: &mut Vec<ProfileConfig>, right: Vec<ProfileConfig>) {
    merge_keyed_by(left, right, |p| p.id.clone());
}

pub(crate) fn merge_mcps_by_name(left: &mut Vec<MCPDefinition>, right: Vec<MCPDefinition>) {
    merge_keyed_by(left, right, |m| m.name.clone());
}
