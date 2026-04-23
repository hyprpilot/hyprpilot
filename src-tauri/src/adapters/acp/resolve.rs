//! Re-export of `ResolvedInstance` from the generic layer.
//!
//! Resolution itself is transport-agnostic — any adapter that
//! resolves an `(agent, profile)` overlay speaks the same shape —
//! so the logic lives in `adapters::profile::ResolvedInstance::from_config`.
//! This shim keeps the ACP-side call sites (`use super::resolve::ResolvedInstance`)
//! short; future refactors drop the shim and import straight from
//! `crate::adapters::profile`.

pub use crate::adapters::profile::ResolvedInstance;
