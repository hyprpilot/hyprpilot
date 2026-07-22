//! Launcher adapter layer.
//!
//! Resolves a profile from layered config and assembles the vendor's
//! native CLI invocation. `cli` owns spawn orchestration; `profile`
//! owns the flat `ResolvedInstance` view built from a
//! `(Config, profile_id?)` pair.

pub(crate) mod cli;
pub mod profile;

pub use crate::resolve::ProfileSummary;
