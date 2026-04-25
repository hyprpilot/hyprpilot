//! Filesystem + terminal primitives used by the ACP adapter.
//!
//! The types here carry no ACP-specific policy beyond accepting the
//! crate's request shapes as tool-call args. `adapters::acp::client::AcpClient`
//! is the adapter that owns the error mapping into
//! `agent_client_protocol::Error`.

pub mod fs;
pub mod sandbox;
pub mod terminal;

pub use fs::{FsError, FsTools};
pub use sandbox::{Sandbox, SandboxError};
pub use terminal::{TerminalError, TerminalToolEvent, TerminalToolEventKind, TerminalToolStream, Terminals};
