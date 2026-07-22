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
//! bind = "0.0.0.0:6262"     # default: 127.0.0.1:6262 when unset
//!
//! [remote.tls]
//! # Optional: bring your own cert/key. When unset, the daemon
//! # auto-generates a self-signed cert and persists it to
//! # `$XDG_STATE_HOME/hyprpilot/remote-{cert,key}.pem` on first run.
//! certificate = "~/.config/hyprpilot/remote-cert.pem"
//! key         = "~/.config/hyprpilot/remote-key.pem"
//!
//! # When generating the cert ourselves, captain-supplied SAN list.
//! # Replaces auto-detection of OS hostname + LAN IPv4 (loopback
//! # always preserved). Each entry parsed as IP first, fallback DNS.
//! sans = ["mydaemon.example.com"]
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
    /// `host:port` to bind. Defaults to `127.0.0.1:6262` (loopback
    /// only) when unset; set to `0.0.0.0:6262` to expose on LAN.
    #[garde(skip)]
    pub bind: Option<String>,
    /// TLS material — bring-your-own cert/key paths and/or SAN
    /// overrides for the auto-generated path.
    #[garde(dive)]
    #[merge(strategy = merge::Merge::merge)]
    pub tls: RemoteTlsConfig,
}

/// TLS-side config sub-struct. Two related concerns:
///
/// 1. **Bring-your-own cert** — set `certificate` + `key` to point
///    at PEMs. Daemon uses them verbatim; never writes to disk; never
///    regenerates. Use this when you have a real cert (Let's Encrypt
///    via nginx-front, internal CA, etc.) or a stable self-signed
///    cert with the SANs you want.
/// 2. **Override SANs of the auto-generated cert** — leave
///    `certificate` + `key` unset, set `sans = [...]`. Daemon
///    generates a self-signed cert containing exactly your SANs +
///    loopback. Stable across boots since the auto-detected LAN
///    IPv4 + OS hostname don't contribute.
///
/// Path fields support `~` and `${VAR}` expansion at consume time.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct RemoteTlsConfig {
    /// Path to a captain-supplied TLS certificate (PEM). When set,
    /// `key` must also be set; daemon uses both verbatim and skips
    /// auto-generation entirely.
    #[garde(skip)]
    pub certificate: Option<PathBuf>,
    /// Path to a captain-supplied TLS private key (PEM). Paired with
    /// `certificate` — must be set together.
    #[garde(skip)]
    pub key: Option<PathBuf>,
    /// Captain-supplied SANs for the auto-generated cert. Each entry
    /// is parsed as an IP address first, falling back to a DNS name
    /// on parse failure. When set, this list **replaces** the
    /// auto-detected LAN IPv4 + OS hostname SANs entirely; loopback
    /// (`localhost`, `127.0.0.1`, `::1`) is always preserved so the
    /// daemon can still talk to itself.
    ///
    /// Useful when the captain has a stable DNS name pointing at the
    /// daemon (`mydaemon.example.com`, a hosts-file entry, etc.) —
    /// the cert never regenerates on IP rotation since the
    /// auto-detected IPv4 SAN isn't computed. When unset, the
    /// daemon auto-detects (current behaviour).
    ///
    /// Ignored when `certificate` + `key` are set — captain-supplied
    /// certs come with their own SAN list.
    #[garde(skip)]
    pub sans: Option<Vec<String>>,
}

// Accessors consumed by the now-removed remote WS bridge. The
// `[remote]` config section itself is pruned in K-729.
#[allow(dead_code)]
impl RemoteConfig {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    /// Resolve the bind address with the loopback-default fallback.
    pub fn resolved_bind(&self) -> String {
        self.bind.clone().unwrap_or_else(|| "127.0.0.1:6262".to_string())
    }
}
