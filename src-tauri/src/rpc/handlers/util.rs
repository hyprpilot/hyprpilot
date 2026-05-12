use serde_json::Value;

use crate::adapters::{Adapter, AdapterError, InstanceKey, SpawnSpec};
use crate::rpc::protocol::RpcError;

pub(super) fn parse_params<T: serde::de::DeserializeOwned>(params: Value, method: &str) -> Result<T, RpcError> {
    serde_json::from_value::<T>(params).map_err(|e| RpcError::invalid_params(format!("{method} params: {e}")))
}

pub(super) fn params_or_default<T: serde::de::DeserializeOwned + Default>(
    params: Value,
    method: &str,
) -> Result<T, RpcError> {
    if params.is_null() {
        return Ok(T::default());
    }
    parse_params(params, method)
}

pub(super) fn map_adapter_err(err: AdapterError) -> RpcError {
    match err {
        AdapterError::InvalidRequest(m) => RpcError::invalid_params(m),
        AdapterError::Unsupported(m) => RpcError::method_not_found(&m),
        AdapterError::Backend(m) => RpcError::internal_error(m),
    }
}

/// `restore`-aware spawn: when `restore = true`, prefer
/// `restore_latest_session(spec)` and fall through to `spawn(spec)`
/// when no matching session exists. When `restore = false`, plain
/// `spawn(spec)`. Wraps the identical 7-line pattern that previously
/// appeared three times across `prompts/send`, `instances/focus
/// { ensure: true }`, and `instances/spawn`.
pub(super) async fn spawn_or_restore(
    adapter: &dyn Adapter,
    spec: SpawnSpec,
    restore: bool,
) -> Result<InstanceKey, RpcError> {
    if restore {
        if let Some(key) = adapter.restore_latest_session(&spec).await.map_err(map_adapter_err)? {
            return Ok(key);
        }
    }
    adapter.spawn(spec).await.map_err(map_adapter_err)
}
