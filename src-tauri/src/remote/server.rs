//! Axum HTTP+WS server bring-up. One TLS-bound listener serves:
//! - `/`        — the SPA bundle via Tauri's `asset_resolver`
//! - `/assets/*` (and any other path) — same fallback
//! - `/ws`      — WebSocket upgrade with pair-on-connect
//! - `/healthz` — liveness probe (plain 200 OK)

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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

/// Resolve a `host:port` string into a `SocketAddr`. `0.0.0.0:6262`
/// + `127.0.0.1:6262` are both valid; passes IPv6 forms too.
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
///
/// **Release**: pull bytes from Tauri's `asset_resolver` (the same
/// source the embedded WebView uses) so the browser sees the exact
/// same SPA bundle that ships with the binary.
///
/// **Debug** (`task run` / `tauri dev`): the asset resolver is empty
/// — Tauri runs the embedded WebView against the Vite dev server at
/// `http://localhost:1420`, and `ui/dist/` stays stale until somebody
/// runs `pnpm build` by hand. A remote browser hitting the daemon
/// in this mode used to pick up whatever ancient bundle happened to
/// be on disk (e.g. with the dropped TanStack Vue Virtual recursion
/// loop still in it), giving the captain "the desktop overlay works
/// but the phone runs out of memory" — exactly the symptom that
/// surfaced during the Vue Virtual + boot-snapshot work.
///
/// In debug we proxy through to the Vite dev server so the phone
/// gets the same code the desktop overlay sees. Hot-reload chunks
/// (`/@vite/client`, `/@id/*`, `/@fs/*`, `/.vite/*`, `?v=…` query)
/// land back at the same path. WebSocket HMR is best-effort — the
/// remote browser stays on `wss://<daemon>/ws` for our pair channel,
/// which means HMR is missing on the phone, but the captain reloads
/// to pick up changes.
async fn asset_handler(State(state): State<RemoteState>, uri: Uri) -> Response {
    if cfg!(debug_assertions) {
        match proxy_to_vite(&uri).await {
            Ok(response) => return response,
            Err(err) => {
                // One-shot warn — without this every asset request
                // (10+ on a fresh page load) floods the log when
                // Vite isn't running. The likely cases are
                // (a) captain ran daemon standalone (no `task run`),
                // (b) `task run` died and only the daemon survived.
                // Either way, we silently fall through to the
                // asset_resolver which serves whatever was last
                // `pnpm build`-ed into `ui/dist/`.
                if !VITE_PROXY_WARNED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        %err,
                        "remote: vite dev proxy unreachable on \
                         127.0.0.1:1420; falling through to ui/dist (run \
                         `task run` for live reload, or `pnpm --filter \
                         hyprpilot-ui build` to refresh ui/dist). Future \
                         proxy failures this session will be silent."
                    );
                }
                // Fall through to the production path below.
            }
        }
    }

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

/// Vite dev-server endpoint. Hardcoded to the Tauri default. If the
/// captain runs `tauri dev` against a non-default `devUrl` the proxy
/// won't find Vite — fall through to `asset_resolver`.
const VITE_DEV_HOST: &str = "127.0.0.1";
const VITE_DEV_PORT: u16 = 1420;

/// One-shot guard for the "Vite unreachable" warn. Without this every
/// asset request (10+ per fresh page load) re-logs the same line.
static VITE_PROXY_WARNED: AtomicBool = AtomicBool::new(false);

/// Forward an HTTP/1.1 GET to the Vite dev server and convert the
/// response into an axum Response. `Connection: close` keeps the
/// reader simple — Vite returns the body and EOFs, no chunked
/// encoding to parse. Localhost-only, so latency overhead is
/// negligible. No request body forwarding (Vite's static-asset
/// surface is GET-only).
async fn proxy_to_vite(uri: &Uri) -> anyhow::Result<Response> {
    let path_and_query = uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or_else(|| uri.path());

    let mut stream = TcpStream::connect((VITE_DEV_HOST, VITE_DEV_PORT))
        .await
        .with_context(|| format!("connect vite dev server at {VITE_DEV_HOST}:{VITE_DEV_PORT}"))?;

    let req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {VITE_DEV_HOST}:{VITE_DEV_PORT}\r\n\
         Connection: close\r\nAccept-Encoding: identity\r\nAccept: */*\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .context("write vite dev request")?;
    stream.flush().await.context("flush vite dev request")?;

    let mut buf = Vec::with_capacity(8 * 1024);
    stream
        .read_to_end(&mut buf)
        .await
        .context("read vite dev response")?;

    // Split header/body at the first CRLFCRLF.
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("vite dev response missing header terminator"))?;
    let head_str = std::str::from_utf8(&buf[..header_end]).context("vite dev headers not UTF-8")?;
    let body = buf[header_end + 4..].to_vec();

    let mut lines = head_str.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| anyhow!("vite dev response missing status line"))?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(200);

    let mut content_type: Option<String> = None;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.eq_ignore_ascii_case("content-type") {
            content_type = Some(v.trim().to_string());
        }
    }

    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
    if let Some(ct) = content_type {
        if let Ok(v) = header::HeaderValue::from_str(&ct) {
            response.headers_mut().insert(header::CONTENT_TYPE, v);
        }
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
    Ok(response)
}
