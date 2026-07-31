use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use serde::Serialize;
use serde_json::Value;

use crate::config::Config;
use crate::resolve::ProfileSummary;

#[derive(Args, Debug)]
pub struct ProfilesArgs {
    /// Emit machine-readable JSON instead of a table.
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfilesOutput {
    profiles: Vec<ProfileListEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileListEntry {
    id: String,
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    is_default: bool,
    /// Present when patch resolution failed — the row's model/cwd are
    /// the unpatched base values, flagged so consumers don't mistake
    /// them for the resolved shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl From<&ProfileSummary> for ProfileListEntry {
    fn from(profile: &ProfileSummary) -> Self {
        Self {
            id: profile.id.clone(),
            agent: profile.agent.clone(),
            model: profile.model.clone(),
            is_default: profile.is_default,
            error: profile.error.clone(),
        }
    }
}

pub fn run(cfg: Config, config_patches: Vec<Value>, args: ProfilesArgs) -> Result<ExitCode> {
    // `config_patches` carries the `--with-config` overlay from the CLI
    // root, so the listing folds the same patches a launch would — the
    // table/JSON reflects what a launch with these flags resolves to.
    let profiles = profile_entries(&crate::spawn::list_profiles(&cfg, None, &config_patches));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&ProfilesOutput { profiles })?);
    } else {
        print!("{}", render_table(&profiles));
    }

    Ok(ExitCode::SUCCESS)
}

fn profile_entries(profiles: &[ProfileSummary]) -> Vec<ProfileListEntry> {
    profiles.iter().map(ProfileListEntry::from).collect()
}

fn render_table(profiles: &[ProfileListEntry]) -> String {
    if profiles.is_empty() {
        return "No profiles configured.\n".into();
    }

    let rows: Vec<[String; 4]> = profiles
        .iter()
        .map(|profile| {
            // A `!` marker (over the `*` default marker — a broken
            // default is still broken) flags a resolution failure, and
            // the model cell carries the error so the row is never a
            // silent stale-data lie.
            let marker = if profile.error.is_some() {
                "!"
            } else if profile.is_default {
                "*"
            } else {
                ""
            };
            let model = match &profile.error {
                Some(error) => format!("! resolve failed: {error}"),
                None => profile.model.clone().unwrap_or_else(|| "-".into()),
            };
            [marker.into(), profile.id.clone(), profile.agent.clone(), model]
        })
        .collect();
    let headers = ["", "profile", "agent", "model"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (idx, value) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(value.len());
        }
    }

    let mut out = String::new();
    render_row(&mut out, &headers, &widths);
    render_rule(&mut out, &widths);
    for row in &rows {
        render_row(&mut out, row, &widths);
    }

    out
}

fn render_row(out: &mut String, values: &[impl AsRef<str>; 4], widths: &[usize; 4]) {
    out.push_str(&format!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}\n",
        values[0].as_ref(),
        values[1].as_ref(),
        values[2].as_ref(),
        values[3].as_ref(),
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
    ));
}

fn render_rule(out: &mut String, widths: &[usize; 4]) {
    let row = widths.map(|width| "-".repeat(width.max(1)));
    render_row(out, &row, widths);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_marks_default_profile() {
        let out = render_table(&profile_entries(&[
            ProfileSummary {
                id: "engineer".into(),
                agent: "claude-code".into(),
                model: Some("claude-sonnet-4-5".into()),
                cwd: Some("~/code/hyprpilot".into()),
                is_default: true,
                error: None,
                ..Default::default()
            },
            ProfileSummary {
                id: "review".into(),
                agent: "codex".into(),
                model: None,
                cwd: None,
                is_default: false,
                error: None,
                ..Default::default()
            },
        ]));

        assert!(out.contains("*  engineer"));
        assert!(out.contains("review"));
        assert!(out.contains("claude-sonnet-4-5"));
    }

    #[test]
    fn table_flags_errored_row_with_bang_and_error() {
        let out = render_table(&profile_entries(&[ProfileSummary {
            id: "broken".into(),
            agent: "claude-code".into(),
            model: Some("stale-model".into()),
            cwd: None,
            is_default: true,
            error: Some("invalid shape after patches".into()),
            ..Default::default()
        }]));

        // `!` marker wins over the `*` default marker, and the stale
        // model is replaced by the error so it can't read as resolved.
        assert!(out.contains("!  broken"), "errored row leads with `!`: {out}");
        assert!(out.contains("resolve failed: invalid shape after patches"), "{out}");
        assert!(
            !out.contains("stale-model"),
            "stale model must be replaced by the error: {out}"
        );
    }

    #[test]
    fn json_entry_carries_error_field() {
        let profiles = profile_entries(&[ProfileSummary {
            id: "broken".into(),
            agent: "claude-code".into(),
            model: None,
            cwd: None,
            is_default: false,
            error: Some("boom".into()),
            ..Default::default()
        }]);
        let json = serde_json::to_string(&ProfilesOutput { profiles }).unwrap();

        assert!(json.contains("\"error\":\"boom\""), "{json}");
    }

    #[test]
    fn table_omits_cwd_column() {
        let out = render_table(&profile_entries(&[ProfileSummary {
            id: "engineer".into(),
            agent: "claude-code".into(),
            model: None,
            cwd: Some("/tmp/launch".into()),
            is_default: true,
            error: None,
            ..Default::default()
        }]));

        assert!(!out.lines().next().unwrap().contains("cwd"));
        assert!(!out.contains("/tmp/launch"));
    }

    #[test]
    fn json_entries_omit_cwd() {
        let profiles = profile_entries(&[ProfileSummary {
            id: "engineer".into(),
            agent: "claude-code".into(),
            model: None,
            cwd: Some("/tmp/launch".into()),
            is_default: true,
            error: None,
            ..Default::default()
        }]);
        let json = serde_json::to_string(&ProfilesOutput { profiles }).unwrap();

        assert!(!json.contains("cwd"));
        assert!(!json.contains("/tmp/launch"));
    }
}
