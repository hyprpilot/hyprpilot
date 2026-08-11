//! Filesystem timestamps for the skills wire shape.
//!
//! Every skill and reference the server serves carries when it was last
//! modified and, where the platform records one, when it was created —
//! so an agent can tell a convention it read last week from one that
//! changed an hour ago.
//!
//! Access time is deliberately absent. It records READS rather than
//! writes, and every `relatime` mount (the Linux default) updates it
//! lazily, so it answers neither "when did this change" nor "when was
//! this added" — it would be a confidently wrong answer to both.

use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value};

/// Format an instant as an RFC 3339 UTC string (`2026-08-11T14:28:59Z`).
///
/// The consumer is a language model, so a readable instant beats an
/// epoch integer it has to convert before it can reason about staleness.
/// Second precision: sub-second resolution says nothing useful about a
/// file an author edits by hand.
#[must_use]
pub fn rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Size and timestamps for one file, from a single `metadata` call.
///
/// Every field is optional and an absent one is OMITTED from the wire
/// rather than serialized as null or back-filled from a neighbour:
/// `created` is unsupported on some filesystems, and substituting
/// `modified` there would answer a different question than the one the
/// key names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileStat {
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub created: Option<String>,
}

impl FileStat {
    /// Stat `path`, mapping every failure to an absent field.
    ///
    /// A file that cannot be stat'd still serves its body — losing a
    /// timestamp must never fail the request that carries the content.
    #[must_use]
    pub fn read(path: &Path) -> Self {
        let Ok(meta) = std::fs::metadata(path) else {
            return Self::default();
        };
        Self {
            size: Some(meta.len()),
            modified: meta.modified().ok().map(rfc3339),
            created: meta.created().ok().map(rfc3339),
        }
    }

    /// Insert the populated fields into a JSON object. Absent fields are
    /// skipped entirely, so a consumer can treat key presence as the
    /// availability signal.
    pub fn extend(&self, map: &mut Map<String, Value>) {
        if let Some(size) = self.size {
            map.insert("size".into(), Value::from(size));
        }
        if let Some(modified) = &self.modified {
            map.insert("modified".into(), Value::String(modified.clone()));
        }
        if let Some(created) = &self.created {
            map.insert("created".into(), Value::String(created.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn formats_the_epoch_and_a_known_instant_as_utc() {
        assert_eq!(rfc3339(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // `date -u -d @1786458539` => 2026-08-11T14:28:59Z.
        let known = SystemTime::UNIX_EPOCH + Duration::from_secs(1_786_458_539);
        assert_eq!(rfc3339(known), "2026-08-11T14:28:59Z");
    }

    /// A leap day is the classic off-by-one in hand-rolled civil-date
    /// math; this pins that we are not hand-rolling it.
    #[test]
    fn handles_a_leap_day() {
        // 1709164800 = 2024-02-29T00:00:00Z.
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_164_800);
        assert_eq!(rfc3339(leap), "2024-02-29T00:00:00Z");
    }

    /// Sub-second precision is truncated, not rounded up — a file
    /// written at `x.999` must not report the next second.
    #[test]
    fn truncates_rather_than_rounds_subsecond_precision() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_786_458_539_999);
        assert_eq!(rfc3339(t), "2026-08-11T14:28:59Z");
    }

    #[test]
    fn reads_size_and_modified_for_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.md");
        std::fs::write(&path, "hello").unwrap();

        let stat = FileStat::read(&path);

        assert_eq!(stat.size, Some(5));
        let modified = stat.modified.expect("mtime is available on every supported platform");
        assert!(modified.ends_with('Z'), "must be UTC: {modified}");
        assert!(modified.len() == 20, "RFC 3339 seconds precision: {modified}");
    }

    /// A missing file yields every field absent rather than an error —
    /// the caller still serves whatever content it holds.
    #[test]
    fn a_missing_file_yields_every_field_absent() {
        let stat = FileStat::read(Path::new("/nonexistent-hyprpilot-probe-xyz"));

        assert_eq!(stat, FileStat::default());
        let mut map = Map::new();
        stat.extend(&mut map);
        assert!(map.is_empty(), "absent fields must not reach the wire at all");
    }

    #[test]
    fn extend_omits_absent_fields_and_writes_present_ones() {
        let stat = FileStat {
            size: Some(42),
            modified: Some("2026-08-11T09:08:59Z".into()),
            created: None,
        };
        let mut map = Map::new();
        stat.extend(&mut map);

        assert_eq!(map.get("size").and_then(Value::as_u64), Some(42));
        assert_eq!(
            map.get("modified").and_then(Value::as_str),
            Some("2026-08-11T09:08:59Z")
        );
        assert!(!map.contains_key("created"), "None must be omitted, not null");
    }
}
