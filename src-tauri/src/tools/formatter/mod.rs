//! Tool-call formatter pipeline. Wire shape lives in `types`;
//! dispatch (per-(adapter, wire) → kind default → "other") in
//! `registry`. Default kind formatters live in `kinds/`; per-adapter
//! overrides live in their respective adapter modules under
//! `crate::adapters::acp::agents::<vendor>::formatters/`.
//!
//! `build_default_registry()` registers the kind defaults then asks
//! every ACP agent to register its overrides.

pub mod kinds;
pub mod registry;
pub mod shared;
pub mod types;

use registry::FormatterRegistry;

/// Construct the runtime registry: kind defaults first, then every
/// ACP agent's per-vendor overrides.
pub fn build_default_registry() -> FormatterRegistry {
    let mut reg = FormatterRegistry::new();
    kinds::register_all(&mut reg);
    crate::adapters::acp::agents::register_all_formatters(&mut reg);
    reg
}
