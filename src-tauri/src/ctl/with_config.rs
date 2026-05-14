//! `--with-config` flag plumbing shared across spawn-shaped `ctl`
//! subcommands. Each input value is either a file path (extension
//! drives format detection) or `-` for stdin (companion
//! `--with-config-format` declares the format). Parses each into a
//! `serde_json::Value`; the resulting `Vec<Value>` rides the wire
//! verbatim under the `withConfig` RPC field.
//!
//! Format support:
//! - `.toml` → `toml::from_str`
//! - `.json` → `serde_json::from_str`
//! - `.yaml` / `.yml` → `serde_yaml::from_str`
//!
//! Default format for stdin / extension-less inputs is JSON
//! (matches CLI piping ergonomics).

use std::fs;
use std::io::{self, Read};

use anyhow::{anyhow, Context, Result};
use clap::{Args, ValueEnum};
use serde_json::Value;

/// Format hint for `--with-config-format`. Drives stdin parsing
/// and acts as the fallback for paths without a recognised
/// extension.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum WithConfigFormat {
    /// CLI-friendly default — most `jq` / `gh api` / curl-style
    /// pipelines emit JSON, so stdin defaults here.
    #[default]
    Json,
    Toml,
    /// Note: `serde_yaml` is unmaintained upstream; flagged in the
    /// migration runway if a CVE shows up.
    Yaml,
}

/// Shared clap struct flattened into spawn-shaped subcommands.
#[derive(Args, Debug, Clone, Default)]
pub struct WithConfigArgs {
    /// Overlay patch(es) folded onto the daemon's resolved config
    /// before the spawn proceeds. Repeatable; patches apply in
    /// declaration order. Value is a file path (extension drives
    /// format: `.toml` / `.json` / `.yaml` / `.yml`) or `-` for
    /// stdin (paired with `--with-config-format`). See
    /// `config::patch` for `$patch` directive semantics.
    #[arg(long = "with-config", value_name = "PATH|-", action = clap::ArgAction::Append)]
    pub with_config: Vec<String>,

    /// Format for `--with-config -` (stdin) and any path without a
    /// recognised extension. Defaults to JSON — best fit for CLI
    /// piping (`cat patch.json | hyprpilot ctl ... --with-config -`).
    #[arg(long = "with-config-format", value_enum, default_value_t = WithConfigFormat::Json)]
    pub with_config_format: WithConfigFormat,
}

impl WithConfigArgs {
    /// Resolve every `--with-config` input into a `serde_json::Value`
    /// document. Returns `Ok(vec![])` when no flags were passed.
    pub fn into_patches(self) -> Result<Vec<Value>> {
        let mut out = Vec::with_capacity(self.with_config.len());

        for raw in self.with_config {
            out.push(parse_one(&raw, self.with_config_format)?);
        }

        Ok(out)
    }
}

fn parse_one(input: &str, default_format: WithConfigFormat) -> Result<Value> {
    if input == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("reading --with-config from stdin")?;
        return parse_with(&buf, default_format);
    }

    let format = format_for_path(input).unwrap_or(default_format);
    let body = fs::read_to_string(input).with_context(|| format!("reading --with-config file '{input}'"))?;
    parse_with(&body, format)
}

fn format_for_path(path: &str) -> Option<WithConfigFormat> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())?
        .to_ascii_lowercase();
    match ext.as_str() {
        "toml" => Some(WithConfigFormat::Toml),
        "json" => Some(WithConfigFormat::Json),
        "yaml" | "yml" => Some(WithConfigFormat::Yaml),
        _ => None,
    }
}

fn parse_with(body: &str, format: WithConfigFormat) -> Result<Value> {
    match format {
        WithConfigFormat::Json => serde_json::from_str(body).map_err(|e| anyhow!("parse --with-config JSON: {e}")),
        WithConfigFormat::Toml => {
            let v: toml::Value = toml::from_str(body).map_err(|e| anyhow!("parse --with-config TOML: {e}"))?;
            serde_json::to_value(&v).map_err(|e| anyhow!("transcode --with-config TOML → JSON: {e}"))
        }
        WithConfigFormat::Yaml => serde_yaml::from_str(body).map_err(|e| anyhow!("parse --with-config YAML: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, body: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("hyprpilot-with-config-{}-{}", std::process::id(), name));
        let mut f = fs::File::create(&path).expect("create temp file");
        f.write_all(body.as_bytes()).expect("write temp file");
        path
    }

    #[test]
    fn parses_toml_by_extension() {
        let p = write_tmp("toml.toml", "[skills]\ndirs = [\"/tmp/a\"]");
        let v = parse_one(p.to_str().unwrap(), WithConfigFormat::Json).expect("parse");
        assert_eq!(v["skills"]["dirs"][0], "/tmp/a");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn parses_json_by_extension() {
        let p = write_tmp("json.json", r#"{"skills":{"dirs":["/tmp/b"]}}"#);
        let v = parse_one(p.to_str().unwrap(), WithConfigFormat::Toml).expect("parse");
        assert_eq!(v["skills"]["dirs"][0], "/tmp/b");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn parses_yaml_by_extension() {
        let p = write_tmp("yaml.yaml", "skills:\n  dirs:\n    - /tmp/c\n");
        let v = parse_one(p.to_str().unwrap(), WithConfigFormat::Json).expect("parse");
        assert_eq!(v["skills"]["dirs"][0], "/tmp/c");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn falls_back_to_format_hint_for_unknown_extension() {
        let p = write_tmp("config.weird", r#"{"x":1}"#);
        let v = parse_one(p.to_str().unwrap(), WithConfigFormat::Json).expect("parse with hint");
        assert_eq!(v["x"], 1);
        let _ = fs::remove_file(p);
    }

    #[test]
    fn missing_file_errors_with_path() {
        let err = parse_one("/no/such/file.toml", WithConfigFormat::Json).unwrap_err();
        assert!(format!("{err:#}").contains("/no/such/file.toml"));
    }

    #[test]
    fn invalid_format_body_returns_specific_error() {
        let p = write_tmp("bad.json", "not json at all");
        let err = parse_one(p.to_str().unwrap(), WithConfigFormat::Toml).unwrap_err();
        assert!(format!("{err:#}").contains("JSON"));
        let _ = fs::remove_file(p);
    }
}
