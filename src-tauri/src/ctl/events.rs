//! `ctl events *` — connection-scoped event subscription. Streams
//! every `events/notify` notification the daemon emits as one JSON
//! line per event. Live-only — no replay, no reconnect; Ctrl-C
//! exits.

use std::io::{BufRead, Write};

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use tracing::warn;

use crate::ctl::client::CtlClient;
use crate::ctl::CtlDispatch;
use crate::rpc::protocol::{EventsNotifyNotification, Outcome};

#[derive(Subcommand, Debug, Clone)]
pub enum EventsSubcommand {
    /// Stream events. Optional comma-separated `--topics` filter and
    /// `--instance` filter scope the stream; firehose otherwise.
    Tail {
        /// Comma-separated topic filter (e.g. `instances.changed,state.changed`).
        #[arg(long, value_delimiter = ',')]
        topics: Option<Vec<String>>,
        /// Instance id filter — only events bound to this instance.
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeParams {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    topics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

impl CtlDispatch for EventsSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            EventsSubcommand::Tail { topics, instance_id } => tail(client, topics.unwrap_or_default(), instance_id),
        }
    }
}

fn tail(client: &CtlClient, topics: Vec<String>, instance_id: Option<String>) -> Result<()> {
    let mut conn = client.connect()?;
    let params = serde_json::to_value(SubscribeParams { topics, instance_id }).expect("serialize SubscribeParams");

    let initial = match conn.call("events/subscribe", params)? {
        Outcome::Success { result } => result,
        Outcome::Error { error } => {
            anyhow::bail!("rpc error {}: {}", error.code, error.message);
        }
    };
    // Echo the subscription id once so a reviewer can spot the
    // connection in the daemon logs.
    eprintln!("subscribed: {initial}");
    let _ = std::io::stdout().flush();

    let mut reader = conn.into_reader();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<EventsNotifyNotification>(trimmed) {
                    Ok(notif) => {
                        let v = serde_json::to_value(&notif.params).expect("EventsNotifyParams serializes");
                        println!("{v}");
                        let _ = std::io::stdout().flush();
                    }
                    Err(_) => {
                        warn!("ctl events tail: unexpected line from daemon: {trimmed}");
                        continue;
                    }
                }
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context("read events/notify"));
            }
        }
    }
}
