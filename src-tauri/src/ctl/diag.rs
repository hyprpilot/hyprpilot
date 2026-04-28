//! `ctl diag *` — operator diagnostics. Read-only structural snapshot.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{request_value, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum DiagSubcommand {
    /// Pretty-print a structural snapshot of the daemon. With
    /// `--output <path>`, writes the JSON to a file instead of stdout.
    Snapshot {
        /// Write the snapshot to this path. Omit to print to stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

impl CtlDispatch for DiagSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            DiagSubcommand::Snapshot { output } => snapshot(client, output),
        }
    }
}

fn snapshot(client: &CtlClient, output: Option<PathBuf>) -> Result<()> {
    let value = request_value(client, "diag/snapshot", &Value::Null)?;
    let pretty = serde_json::to_string_pretty(&value)?;
    match output {
        Some(path) => {
            std::fs::write(&path, pretty.as_bytes()).with_context(|| format!("write {}", path.display()))?;
            eprintln!("snapshot written to {}", path.display());
        }
        None => println!("{pretty}"),
    }
    Ok(())
}
