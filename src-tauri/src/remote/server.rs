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
use once_cell::sync::Lazy;

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

/// Shared `reqwest::Client` for the dev proxy. Connection pooling
/// keeps successive asset requests fast; chunked decoding,
/// header-aware framing, and content-length handling come for free
/// (raw `tokio::TcpStream` parsing missed all three — the previous
/// hand-rolled proxy concatenated chunk-size hex lines into the
/// response body and stripped every header except `Content-Type`,
/// which clobbered ETag / Cache-Control / Content-Length).
static VITE_PROXY_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        // Cap the round-trip — Vite responses for static dev assets
        // are local and quick. A genuine stall (Vite hung mid-build,
        // captain on a flaky filesystem) should fall through to
        // `asset_resolver` rather than freeze the request loop.
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// Forward a GET to the Vite dev server and rewrap as an axum
/// Response. `reqwest` handles every framing detail the previous
/// hand-rolled raw-TCP version mishandled — chunked transfer
/// encoding, content-length, header-aware body termination — so
/// JS / CSS bodies arrive intact instead of laced with chunk-size
/// hex lines. Headers other than the hop-by-hop set
/// (`Connection` / `Transfer-Encoding` / `Keep-Alive` / `Upgrade`)
/// are forwarded verbatim — Vite's `Cache-Control` / `ETag` /
/// `Last-Modified` reach the browser, so its module-graph caching
/// keeps working.
async fn proxy_to_vite(uri: &Uri) -> anyhow::Result<Response> {
    let path_and_query = uri.path_and_query().map(|p| p.as_str()).unwrap_or_else(|| uri.path());
    let url = format!("http://{VITE_DEV_HOST}:{VITE_DEV_PORT}{path_and_query}");

    let upstream = VITE_PROXY_CLIENT
        .get(&url)
        .send()
        .await
        .with_context(|| format!("vite dev fetch {url}"))?;

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);

    // Snapshot upstream headers BEFORE consuming the body — `bytes()`
    // takes ownership.
    let mut forwarded_headers = axum::http::HeaderMap::new();
    for (name, value) in upstream.headers() {
        // Skip hop-by-hop. Forwarding them confuses axum's hyper
        // layer when it re-frames the outbound response.
        let s = name.as_str();
        if s.eq_ignore_ascii_case("connection")
            || s.eq_ignore_ascii_case("transfer-encoding")
            || s.eq_ignore_ascii_case("keep-alive")
            || s.eq_ignore_ascii_case("upgrade")
            || s.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            header::HeaderName::from_bytes(s.as_bytes()),
            header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            forwarded_headers.append(n, v);
        }
    }

    let body = upstream.bytes().await.context("read vite dev response body")?;

    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = forwarded_headers;
    if !response.headers().contains_key(header::CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
    }
    Ok(response)
}
