//! `[remote]` config — the TLS axum HTTP+WS server that lets a phone
//! (or any browser on the LAN) load the Vue overlay and operate the
//! daemon as a remote.
//!
//! Off by default. When enabled, the daemon binds an axum listener
//! alongside its existing unix socket + Tauri overlay. Authentication
//! is per-connection: phone opens a WS, daemon holds it pending while
//! the captain confirms a 4-word BIP39 pair code on the desktop. No
//! tokens persist; reconnect = re-pair.
//!
//! ```toml
//! [remote]
//! enabled = true            # off by default
//! bind = "0.0.0.0:7423"     # default: 127.0.0.1:7423 when unset
//!
//! # Optional: bring your own cert/key. When unset, the daemon
//! # auto-generates a self-signed cert and persists it to
//! # `$XDG_STATE_HOME/hyprpilot/remote-{cert,key}.pem` on first run.
//! tls_cert = "~/.config/hyprpilot/remote-cert.pem"
//! tls_key  = "~/.config/hyprpilot/remote-key.pem"
//! ```

use std::path::PathBuf;

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use crate::config::merge_strategies::overwrite_some;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct RemoteConfig {
    /// Enable the remote bridge. Off by default — captains opt in.
    #[garde(skip)]
    pub enabled: Option<bool>,
    /// `host:port` to bind. Defaults to `127.0.0.1:7423` (loopback
    /// only) when unset; set to `0.0.0.0:7423` to expose on LAN.
    #[garde(skip)]
    pub bind: Option<String>,
    /// Path to the TLS certificate (PEM). When unset, the daemon
    /// auto-generates a self-signed cert at first start and persists
    /// it under `state_dir()/remote-cert.pem`. `~` / env-var
    /// expansion happens at consume time.
    #[garde(skip)]
    pub tls_cert: Option<PathBuf>,
    /// Path to the TLS private key (PEM). Paired with `tls_cert` —
    /// must be set when `tls_cert` is set, otherwise the daemon's
    /// auto-generated key is used.
    #[garde(skip)]
    pub tls_key: Option<PathBuf>,
}

impl RemoteConfig {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    /// Resolve the bind address with the loopback-default fallback.
    pub fn resolved_bind(&self) -> String {
        self.bind.clone().unwrap_or_else(|| "127.0.0.1:7423".to_string())
    }
}
