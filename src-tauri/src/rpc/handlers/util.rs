use serde::Deserialize;
use serde_json::Value;

use crate::adapters::AdapterError;
use crate::rpc::protocol::RpcError;

/// Shared param struct for handlers that only route by instance id
/// (`commands/list`, `modes/list`, `models/list`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstanceIdOnly {
    pub(super) instance_id: String,
}

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
