//! Self-signed TLS certificate provisioning. On first start with
//! `[remote] enabled = true` and no captain-supplied cert, the
//! daemon generates a self-signed cert + key and persists them under
//! `state_dir()/remote-{cert,key}.pem`. Subsequent starts reuse the
//! same files — but only when the SANs the daemon would produce
//! today still match what's in the cert. Roaming captains end up on
//! a different LAN IP than they were on at first generation, and a
//! cert without that IP makes browsers silently reject the TLS
//! handshake (no log on the daemon — rustls drops it before the WS
//! upgrade). To handle that, we drop a sidecar file with the SANs
//! the cert was generated against and recompute on every boot;
//! mismatch → regenerate the pair. TOFU prompts the phone again
//! on real rotation but stays quiet across normal restarts.
//!
//! SANs cover loopback addresses + the OS hostname + every detected
//! non-loopback IPv4 the daemon's interfaces carry.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};

use crate::paths;

/// Loaded PEM-encoded TLS material. `cert` may carry a chain;
/// `axum-server` accepts both a single leaf and a fullchain.
#[derive(Clone)]
pub struct TlsMaterial {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
}

/// Resolve the daemon's TLS material against config. When the
/// captain supplies `[remote] tls_cert` + `tls_key`, both are
/// expanded + read. Otherwise we ensure the persistent self-signed
/// pair exists under `state_dir()/`, regenerating when the SANs
/// have drifted (IP rotation, hostname change).
pub fn resolve_or_generate(cfg: &crate::config::RemoteConfig) -> Result<TlsMaterial> {
    if let (Some(cert_path), Some(key_path)) = (&cfg.tls_cert, &cfg.tls_key) {
        let cert = paths::resolve_user(&cert_path.to_string_lossy());
        let key = paths::resolve_user(&key_path.to_string_lossy());
        return load_pair(&cert, &key);
    }
    let CertPaths { cert, key, sans } = persistent_paths()?;
    let desired = build_sans();
    let desired_text = sans_text(&desired);

    if cert.exists() && key.exists() && sans.exists() {
        match fs::read_to_string(&sans) {
            Ok(stored) if stored == desired_text => return load_pair(&cert, &key),
            Ok(_) => tracing::info!(
                "remote: SAN set drifted from persisted cert (IP / hostname rotation) — regenerating"
            ),
            Err(err) => tracing::warn!(%err, "remote: failed to read SAN sidecar — regenerating"),
        }
    }
    let material = generate(&desired)?;
    fs::create_dir_all(cert.parent().expect("state_dir has a parent"))
        .with_context(|| format!("create state dir for {}", cert.display()))?;
    fs::write(&cert, &material.cert_pem).with_context(|| format!("write {}", cert.display()))?;
    fs::write(&key, &material.key_pem).with_context(|| format!("write {}", key.display()))?;
    fs::write(&sans, desired_text.as_bytes()).with_context(|| format!("write {}", sans.display()))?;
    tracing::info!(
        cert = %cert.display(),
        key = %key.display(),
        sans = %desired_text.replace('\n', ", "),
        "remote: generated self-signed TLS material"
    );
    Ok(material)
}

struct CertPaths {
    cert: PathBuf,
    key: PathBuf,
    sans: PathBuf,
}

fn persistent_paths() -> Result<CertPaths> {
    let dir = paths::state_dir();
    Ok(CertPaths {
        cert: dir.join("remote-cert.pem"),
        key: dir.join("remote-key.pem"),
        sans: dir.join("remote-cert.sans"),
    })
}

fn load_pair(cert_path: &Path, key_path: &Path) -> Result<TlsMaterial> {
    let cert_pem = fs::read(cert_path).with_context(|| format!("read tls_cert {}", cert_path.display()))?;
    let key_pem = fs::read(key_path).with_context(|| format!("read tls_key {}", key_path.display()))?;
    Ok(TlsMaterial { cert_pem, key_pem })
}

fn generate(sans: &[SanType]) -> Result<TlsMaterial> {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "hyprpilot");
    dn.push(DnType::OrganizationName, "hyprpilot remote bridge");
    params.distinguished_name = dn;
    params.subject_alt_names = sans.to_vec();

    let key = KeyPair::generate().context("generate TLS keypair")?;
    let cert = params.self_signed(&key).context("self-sign certificate")?;
    Ok(TlsMaterial {
        cert_pem: cert.pem().into_bytes(),
        key_pem: key.serialize_pem().into_bytes(),
    })
}

fn build_sans() -> Vec<SanType> {
    let mut sans: Vec<SanType> = Vec::new();
    sans.push(SanType::DnsName(
        "localhost".try_into().expect("localhost is valid DNS"),
    ));
    sans.push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    sans.push(SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)));

    if let Ok(host) = hostname::get() {
        if let Some(name) = host.to_str() {
            // Bare hostname AND `<hostname>.local` so OS-level mDNS
            // resolution (when present) still validates.
            if let Ok(name_dns) = name.to_string().try_into() {
                sans.push(SanType::DnsName(name_dns));
            }
            let dotted = format!("{name}.local");
            if let Ok(dotted_dns) = dotted.try_into() {
                sans.push(SanType::DnsName(dotted_dns));
            }
        }
    }

    // Non-loopback IPv4s on local interfaces — captain typically
    // pairs by IP. Best-effort: failure to enumerate just narrows
    // the SAN set; phone TOFU still lets it through.
    if let Some(ips) = local_ipv4s() {
        for ip in ips {
            sans.push(SanType::IpAddress(IpAddr::V4(ip)));
        }
    }

    sans
}

/// Canonical sidecar serialisation: one SAN per line, sorted, in a
/// stable string form. Round-trip safe for byte-equal comparison
/// across boots — `build_sans()` order is deterministic but sort
/// gives belt-and-suspenders.
fn sans_text(sans: &[SanType]) -> String {
    let mut lines: Vec<String> = sans
        .iter()
        .map(|s| match s {
            SanType::DnsName(name) => format!("DNS:{}", name.as_ref() as &str),
            SanType::IpAddress(ip) => format!("IP:{ip}"),
            other => format!("OTHER:{other:?}"),
        })
        .collect();
    lines.sort();
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Enumerate non-loopback IPv4 addresses on the daemon's interfaces.
/// Pure-stdlib best-effort via `UdpSocket::connect` to a public IP —
/// the OS picks the source address it'd use, no packets transmit.
fn local_ipv4s() -> Option<Vec<Ipv4Addr>> {
    use std::net::UdpSocket;
    // 1.1.1.1 is a routable address; `connect` on UDP just sets the
    // destination, no traffic. The OS resolves the source IP it'd
    // pick to reach there — typically the captain's primary LAN IP.
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("1.1.1.1:80").ok()?;
    let local = sock.local_addr().ok()?;
    match local.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(vec![v4]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sans_text_is_deterministic_and_sorted() {
        let a = vec![
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))),
            SanType::DnsName("localhost".try_into().unwrap()),
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ];
        let b = vec![
            SanType::DnsName("localhost".try_into().unwrap()),
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))),
        ];
        assert_eq!(sans_text(&a), sans_text(&b));
        let expected = "DNS:localhost\nIP:127.0.0.1\nIP:192.168.1.5\n";
        assert_eq!(sans_text(&a), expected);
    }

    #[test]
    fn sans_text_diff_when_ip_rotates() {
        let on_24 = vec![
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(192, 168, 24, 64))),
        ];
        let on_30 = vec![
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(192, 168, 30, 56))),
        ];
        assert_ne!(sans_text(&on_24), sans_text(&on_30));
    }
}
