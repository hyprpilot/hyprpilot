//! Completion sources. Each module exports a unit struct that
//! implements [`CompletionSource`] from the parent module.

pub mod commands;
pub mod path;
pub mod ripgrep;
pub mod skills;
