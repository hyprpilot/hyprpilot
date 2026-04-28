//! `ctl agents *` — read-only operations over the `[[agents]]`
//! registry.

use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum AgentsSubcommand {
    /// List configured agents.
    List,
}

impl CtlDispatch for AgentsSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            AgentsSubcommand::List => list(client),
        }
    }
}

fn list(client: &CtlClient) -> Result<()> {
    emit(client, "agents/list", &Value::Null)
}
