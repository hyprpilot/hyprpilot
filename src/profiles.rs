use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use serde::Serialize;

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
}

impl From<&ProfileSummary> for ProfileListEntry {
    fn from(profile: &ProfileSummary) -> Self {
        Self {
            id: profile.id.clone(),
            agent: profile.agent.clone(),
            model: profile.model.clone(),
            is_default: profile.is_default,
        }
    }
}

pub fn run(cfg: Config, args: ProfilesArgs) -> Result<ExitCode> {
    let profiles = profile_entries(&crate::spawn::list_profiles(&cfg, None, &[]));
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
            [
                if profile.is_default { "*" } else { "" }.into(),
                profile.id.clone(),
                profile.agent.clone(),
                profile.model.clone().unwrap_or_else(|| "-".into()),
            ]
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
            },
            ProfileSummary {
                id: "review".into(),
                agent: "codex".into(),
                model: None,
                cwd: None,
                is_default: false,
            },
        ]));

        assert!(out.contains("*  engineer"));
        assert!(out.contains("review"));
        assert!(out.contains("claude-sonnet-4-5"));
    }

    #[test]
    fn table_omits_cwd_column() {
        let out = render_table(&profile_entries(&[ProfileSummary {
            id: "engineer".into(),
            agent: "claude-code".into(),
            model: None,
            cwd: Some("/tmp/launch".into()),
            is_default: true,
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
        }]);
        let json = serde_json::to_string(&ProfilesOutput { profiles }).unwrap();

        assert!(!json.contains("cwd"));
        assert!(!json.contains("/tmp/launch"));
    }
}
