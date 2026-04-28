//! `ctl toggle` — top-level shortcut mapping to `window/toggle` on
//! the wire.

use anyhow::Result;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::emit;

pub(super) fn toggle(client: &CtlClient) -> Result<()> {
    emit(client, "window/toggle", &Value::Null)
}
