use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use serde::Serialize;

use crate::adapters::ProfileSummary;
use crate::config::Config;

#[derive(Args, Debug)]
pub struct ProfilesArgs {
    /// Emit machine-readable JSON instead of a table.
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfilesOutput {
    profiles: Vec<ProfileSummary>,
}

pub fn run(cfg: Config, args: ProfilesArgs) -> Result<ExitCode> {
    let profiles = crate::adapters::cli::list_profiles(&cfg);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&ProfilesOutput { profiles })?);
    } else {
        print!("{}", render_table(&profiles));
    }

    Ok(ExitCode::SUCCESS)
}

fn render_table(profiles: &[ProfileSummary]) -> String {
    if profiles.is_empty() {
        return "No profiles configured.\n".into();
    }

    let rows: Vec<[String; 5]> = profiles
        .iter()
        .map(|profile| {
            [
                if profile.is_default { "*" } else { "" }.into(),
                profile.id.clone(),
                profile.agent.clone(),
                profile.model.clone().unwrap_or_else(|| "-".into()),
                profile.cwd.clone().unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    let headers = ["", "profile", "agent", "model", "cwd"];
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

fn render_row(out: &mut String, values: &[impl AsRef<str>; 5], widths: &[usize; 5]) {
    out.push_str(&format!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}\n",
        values[0].as_ref(),
        values[1].as_ref(),
        values[2].as_ref(),
        values[3].as_ref(),
        values[4].as_ref(),
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4],
    ));
}

fn render_rule(out: &mut String, widths: &[usize; 5]) {
    let row = widths.map(|width| "-".repeat(width.max(1)));
    render_row(out, &row, widths);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_marks_default_profile() {
        let out = render_table(&[
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
        ]);

        assert!(out.contains("*  engineer"));
        assert!(out.contains("review"));
        assert!(out.contains("claude-sonnet-4-5"));
    }
}
