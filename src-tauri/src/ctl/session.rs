//! `ctl submit` / `ctl cancel` / `ctl session-info` — top-level
//! shortcuts mapping to the `session/*` namespace on the wire.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::emit;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitParams {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
}

pub(super) fn submit(
    client: &CtlClient,
    text: Vec<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<()> {
    emit(
        client,
        "session/submit",
        &SubmitParams {
            text: text.join(" "),
            agent_id,
            profile_id,
        },
    )
}

pub(super) fn cancel(client: &CtlClient) -> Result<()> {
    emit(client, "session/cancel", &Value::Null)
}

pub(super) fn info(client: &CtlClient) -> Result<()> {
    emit(client, "session/info", &Value::Null)
}
