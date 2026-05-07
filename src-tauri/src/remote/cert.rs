//! Self-signed TLS certificate provisioning. On first start with
//! `[remote] enabled = true` and no captain-supplied cert, the
//! daemon generates a self-signed cert + key and persists them under
//! `state_dir()/remote-{cert,key}.pem`. Subsequent starts reuse the
//! same files. Captain TOFU on the phone the first time they pair.
//!
//! SANs cover loopback addresses + the OS hostname + every detected
//! non-loopback IPv4 the daemon's interfaces carry, so the cert
//! works against whichever address the captain ends up using.

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
/// pair exists under `state_dir()/`, generating it on first run.
pub fn resolve_or_generate(cfg: &crate::config::RemoteConfig) -> Result<TlsMaterial> {
    if let (Some(cert_path), Some(key_path)) = (&cfg.tls_cert, &cfg.tls_key) {
        let cert = paths::resolve_user(&cert_path.to_string_lossy());
        let key = paths::resolve_user(&key_path.to_string_lossy());
        return load_pair(&cert, &key);
    }
    let (cert_path, key_path) = persistent_paths()?;
    if cert_path.exists() && key_path.exists() {
        return load_pair(&cert_path, &key_path);
    }
    let material = generate()?;
    fs::create_dir_all(cert_path.parent().expect("state_dir has a parent"))
        .with_context(|| format!("create state dir for {}", cert_path.display()))?;
    fs::write(&cert_path, &material.cert_pem).with_context(|| format!("write {}", cert_path.display()))?;
    fs::write(&key_path, &material.key_pem).with_context(|| format!("write {}", key_path.display()))?;
    tracing::info!(
        cert = %cert_path.display(),
        key = %key_path.display(),
        "remote: generated self-signed TLS material"
    );
    Ok(material)
}

fn persistent_paths() -> Result<(PathBuf, PathBuf)> {
    let dir = paths::state_dir();
    Ok((dir.join("remote-cert.pem"), dir.join("remote-key.pem")))
}

fn load_pair(cert_path: &Path, key_path: &Path) -> Result<TlsMaterial> {
    let cert_pem = fs::read(cert_path).with_context(|| format!("read tls_cert {}", cert_path.display()))?;
    let key_pem = fs::read(key_path).with_context(|| format!("read tls_key {}", key_path.display()))?;
    Ok(TlsMaterial { cert_pem, key_pem })
}

fn generate() -> Result<TlsMaterial> {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "hyprpilot");
    dn.push(DnType::OrganizationName, "hyprpilot remote bridge");
    params.distinguished_name = dn;
    params.subject_alt_names = build_sans();

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
