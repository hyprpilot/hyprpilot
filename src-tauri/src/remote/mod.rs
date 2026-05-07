//! Remote bridge — TLS axum HTTP+WS server alongside the Tauri
//! overlay + unix socket. Lets a phone (or any browser on the LAN)
//! load the Vue overlay and operate the daemon as a remote.
//!
//! Pair-on-connect: phone opens a WS, daemon mints a 4-word BIP39
//! code, holds the connection in `pending` state, emits a
//! `remote:pair-request` Tauri event. The desktop overlay opens a
//! palette modal where the captain types (or scans) the code; on
//! match the WS upgrades to authenticated. No tokens persist —
//! reconnect = re-pair.

pub mod cert;
pub mod commands;
pub mod pair;
pub mod server;
pub mod ws;
