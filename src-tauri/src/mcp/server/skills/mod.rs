//! Skills feature for the hyprpilot MCP server.
//!
//! Today's only shipped feature: expose resolved per-instance skills as
//! `hyprpilot://skills/<slug>` resources and a handful of helper tools.
//! Future features (workspace introspection, codebase navigation, …)
//! plug into the same `HyprpilotServer` in `super::serve` rather than
//! getting their own subcommands.

pub mod manifest;
pub mod references;
