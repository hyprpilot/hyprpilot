use async_trait::async_trait;
use serde_json::Value;

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// Status namespace: `status/get`, `status/subscribe`.
///
/// Reads from the shared `StatusBroadcast` via `HandlerCtx.status`.
/// `status/subscribe` is the only method in the whole RPC surface that
/// returns a `HandlerOutcome::Subscribed` — the server pins the receiver
/// onto the connection task and fans `status/changed` notifications out
/// as they arrive.
pub struct StatusHandler;

#[async_trait]
impl RpcHandler for StatusHandler {
    fn namespace(&self) -> &'static str {
        "status"
    }

    async fn handle(&self, method: &str, _params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "status/get" => {
                let snapshot = ctx.status.get();
                let v = serde_json::to_value(snapshot).expect("StatusResult serializes");
                Ok(HandlerOutcome::Reply(v))
            }
            "status/subscribe" => {
                if ctx.already_subscribed {
                    return Err(RpcError::invalid_request(
                        "connection already subscribed to status/changed",
                    ));
                }
                let (snapshot, rx) = ctx.status.subscribe();
                let v = serde_json::to_value(snapshot).expect("StatusResult serializes");
                Ok(HandlerOutcome::Subscribed(v, rx))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}
