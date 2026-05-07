//! Axum HTTP+WS server bring-up. One TLS-bound listener serves:
//! - `/`        — the SPA bundle via Tauri's `asset_resolver`
//! - `/assets/*` (and any other path) — same fallback
//! - `/ws`      — WebSocket upgrade with pair-on-connect
//! - `/healthz` — liveness probe (plain 200 OK)

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;

use crate::adapters::Adapter;
use crate::remote::cert::TlsMaterial;
use crate::remote::pair::PairStore;
use crate::remote::session::SessionTokens;
use crate::remote::ws;
use crate::rpc::status::StatusBroadcast;
use crate::rpc::RpcDispatcher;

/// Shared state every axum handler reads. Mirrors the fields
/// `RpcState` carries for the unix socket path so per-connection
/// dispatch reuses the same `RpcDispatcher` + adapter handles.
#[derive(Clone)]
pub struct RemoteState {
    pub app: tauri::AppHandle,
    pub status: Arc<StatusBroadcast>,
    pub dispatcher: Arc<RpcDispatcher>,
    pub adapter: Arc<dyn Adapter>,
    pub config: Arc<std::sync::RwLock<crate::config::Config>>,
    pub skills: Arc<crate::skills::SkillsRegistry>,
    pub mcps: Arc<crate::mcp::MCPsRegistry>,
    pub pairs: PairStore,
    pub sessions: SessionTokens,
    pub started_at: std::time::Instant,
}

/// Build the axum router. Public for testability — production
/// callers go through `serve()`.
pub fn router(state: RemoteState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws_handler))
        .fallback(any(asset_handler))
        .with_state(state)
}

/// Spawn the TLS axum listener on the configured bind. Returns
/// when the listener fails to bind (config error / port in use);
/// otherwise runs forever in the background.
pub async fn serve(bind: SocketAddr, tls: TlsMaterial, state: RemoteState) -> Result<()> {
    let tls_cfg = RustlsConfig::from_pem(tls.cert_pem, tls.key_pem)
        .await
        .context("rustls: load TLS material")?;

    let app = router(state);
    tracing::info!(%bind, "remote: TLS listener up");
    axum_server::bind_rustls(bind, tls_cfg)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("axum_server: serve loop terminated")?;
    Ok(())
}

/// Resolve a `host:port` string into a `SocketAddr`. `0.0.0.0:7423`
/// + `127.0.0.1:7423` are both valid; passes IPv6 forms too.
pub fn parse_bind(raw: &str) -> Result<SocketAddr> {
    SocketAddr::from_str(raw).map_err(|e| anyhow!("invalid bind '{raw}': {e}"))
}

/// `GET /healthz` — plain 200 OK. Lets the captain confirm the
/// listener is up without going through the WS upgrade dance.
async fn healthz() -> &'static str {
    "ok"
}

/// `GET /ws` — WebSocket upgrade. Every accepted upgrade enters
/// `pending` state and waits for the captain to confirm a pair
/// code on the desktop. See `ws::handle_socket`.
async fn ws_handler(
    State(state): State<RemoteState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| async move {
        ws::handle_socket(socket, state, addr).await;
    })
}

/// SPA fallback — every non-WS, non-healthz request lands here.
/// We pull bytes from Tauri's `asset_resolver` (the same source
/// the embedded WebView uses), so the browser sees the exact same
/// SPA bundle.
async fn asset_handler(State(state): State<RemoteState>, uri: Uri) -> Response {
    let path = uri.path();
    let lookup = if path == "/" { "/index.html" } else { path };

    match state.app.asset_resolver().get(lookup.to_string()) {
        Some(asset) => {
            let mut response = Response::new(axum::body::Body::from(asset.bytes));
            if let Ok(value) = header::HeaderValue::from_str(&asset.mime_type) {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
            response
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
