//! `--transport http` — serving an in-tree MCP server over Streamable
//! HTTP instead of a pipe.
//!
//! rmcp ships the protocol half as [`StreamableHttpService`], a
//! `tower_service::Service` that binds no socket — every `TcpListener`
//! in its source is test code — so the HTTP server under it is ours.
//! axum because it is the integration rmcp documents and tests against,
//! and because the accept loop, the graceful drain and the middleware
//! seam the token check sits in are all things it already owns.
//!
//! **The handler is built once and cloned per request.** Under MCP
//! `2026-07-28` every request is served statelessly (SEP-2567 removed
//! sessions) and rmcp calls the service factory for each one — so a
//! factory that CONSTRUCTED a handler would rescan every skill root on
//! every call. Each handler's state already lives behind `Arc`, so the
//! clone shares one cache, one session table and one subscription
//! registry across every request.
//!
//! **What a stateless request cannot do** is outlive its response: the
//! peer it carries dies with it. Anything that notifies from outside a
//! request — the skills watcher, the harness exit hook — therefore
//! reaches only clients holding a `subscriptions/listen` stream, whose
//! sink is registered in that shared registry.
//! `Transport::result_ttl_ms` is what keeps that honest.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::ServerHandler;

use super::serve_args::ServeArgs;

/// Serve `handler` over Streamable HTTP until SIGTERM or SIGHUP.
///
/// Returns once in-flight connections have drained, so the caller's own
/// teardown — reaping the harness session table, releasing the skills
/// watcher — runs in the same place it does on the stdio path.
pub(super) async fn serve_http<H>(handler: H, args: &ServeArgs, server_name: &str) -> anyhow::Result<()>
where
    H: ServerHandler + Clone + Send + Sync + 'static,
{
    let listener = bind(args).await?;
    serve_bound(handler, args, server_name, listener).await
}

/// Open the listener named by `--listen`.
///
/// Split from [`serve_http`] so a caller — a test, above all — can learn
/// the bound address before the server starts running on it. `:0` is a
/// real address to ask for; it is only useless as a DEFAULT.
async fn bind(args: &ServeArgs) -> anyhow::Result<tokio::net::TcpListener> {
    let listen = args
        .listen
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("mcp: --transport http requires --listen <ADDR>"))?;

    tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|err| anyhow::anyhow!("mcp: could not listen on {listen}: {err}"))
}

async fn serve_bound<H>(
    handler: H,
    args: &ServeArgs,
    server_name: &str,
    listener: tokio::net::TcpListener,
) -> anyhow::Result<()>
where
    H: ServerHandler + Clone + Send + Sync + 'static,
{
    let token = args.token()?;
    let bound = listener.local_addr()?;

    let config = server_config(bound, args.allow_remote);
    let cancel = config.cancellation_token.clone();
    // Cloned per request, never rebuilt — see the module docs.
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let mut app = axum::Router::new().fallback_service(service);
    if let Some(expected) = token.clone() {
        // In front of rmcp rather than inside a handler, because it has
        // to cover every method — including the ones rmcp answers on its
        // own before any handler runs.
        app = app.layer(axum::middleware::from_fn(move |request: Request, next: Next| {
            let expected = expected.clone();
            async move {
                if presented(request.headers(), &expected) {
                    next.run(request).await
                } else {
                    unauthorized()
                }
            }
        }));
    }

    tracing::info!(
        server = %server_name,
        addr = %bound,
        authenticated = token.is_some(),
        allow_remote = args.allow_remote,
        "mcp: serving over http"
    );
    if token.is_none() {
        // Loud, because the captain chose it and the consequence is not
        // obvious: every local process can reach this port, and on the
        // harness that means running arbitrary binaries as this user.
        tracing::warn!(
            server = %server_name,
            addr = %bound,
            "mcp: no token configured — every local process can call this server"
        );
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|err| anyhow::anyhow!("mcp: http server failed: {err}"))?;

    // Terminates the open SSE streams. A `subscriptions/listen` parked
    // on its cancellation token holds no request open after this, so the
    // caller's teardown is not waiting on a client that never hangs up.
    cancel.cancel();

    Ok(())
}

/// The transport policy, which is mostly about who is allowed to talk to
/// this port.
fn server_config(bound: std::net::SocketAddr, allow_remote: bool) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default();
    // Plain tool calls come back as JSON rather than a one-event SSE
    // stream. rmcp falls back to SSE by itself the moment a handler
    // emits anything before its result, which is exactly what a
    // `subscriptions/listen` stream does — so this costs that path
    // nothing.
    config.json_response = true;

    if allow_remote {
        // The captain asked for a reachable server, so `Host` cannot
        // stay pinned to loopback names — otherwise the bind succeeds
        // and every request is refused, which reads as a bug.
        config = config.disable_allowed_hosts();
    }

    // ORIGIN validation, which rmcp leaves OFF by default (an empty list
    // disables it) and which `allowed_hosts` does not substitute for: a
    // `Host` check does not stop a cross-origin `fetch()` from a page
    // the captain happens to have open, and against an unauthenticated
    // harness that is arbitrary code execution from a browser tab.
    //
    // The list is this server's own origin, so it is non-empty — which
    // is what turns validation on — while matching no page but one this
    // very endpoint served. A request with NO `Origin` still passes, so
    // ordinary non-browser clients are unaffected.
    config.with_allowed_origins([format!("http://{bound}"), format!("https://{bound}")])
}

/// Whether the request carries the expected bearer token.
///
/// Compared over the whole value in constant time: a byte-by-byte early
/// return leaks the token's prefix to anyone who can time the response,
/// and this endpoint is reachable by every local process.
fn presented(headers: &HeaderMap, expected: &str) -> bool {
    let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(presented) = value.strip_prefix("Bearer ").map(str::trim) else {
        return false;
    };
    if presented.len() != expected.len() {
        return false;
    }

    presented
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// A bare 401.
///
/// Deliberately NOT the OAuth shape: MCP's authorization spec wants a
/// `WWW-Authenticate` pointing at RFC 9728 resource metadata, which
/// describes an OAuth resource server. This is a single shared token for
/// a server the captain runs by hand, so advertising an OAuth discovery
/// document would promise an endpoint that does not exist.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        "Unauthorized: send `Authorization: Bearer <token>`",
    )
        .into_response()
}

/// Resolves on SIGTERM or SIGHUP, mirroring the arms
/// `rpc::wait_for_shutdown` races the stdio transport against. There is
/// no `RunningService` here to await, so the listener owns the race.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term = signal(SignalKind::terminate()).ok();
        let mut hup = signal(SignalKind::hangup()).ok();
        tokio::select! {
            Some(()) = async { match term.as_mut() { Some(s) => s.recv().await, None => None } } => {
                tracing::debug!("mcp http: SIGTERM");
            }
            Some(()) = async { match hup.as_mut() { Some(s) => s.recv().await, None => None } } => {
                tracing::debug!("mcp http: SIGHUP");
            }
            else => std::future::pending::<()>().await,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bearer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());

        headers
    }

    #[test]
    fn only_the_exact_bearer_token_is_accepted() {
        assert!(presented(&bearer("Bearer s3cret"), "s3cret"));
        assert!(!presented(&bearer("Bearer s3cre"), "s3cret"));
        assert!(!presented(&bearer("Bearer s3crets"), "s3cret"));
        assert!(!presented(&bearer("Bearer wrong!"), "s3cret"));
        assert!(!presented(&bearer("s3cret"), "s3cret"), "the scheme is required");
        assert!(!presented(&HeaderMap::new(), "s3cret"), "no header is no token");
    }

    /// `allowed_origins` is what actually stops a browser page, and an
    /// EMPTY list disables the check — so the one thing this must never
    /// do is leave it empty.
    #[test]
    fn origin_validation_is_always_armed() {
        let addr: std::net::SocketAddr = "127.0.0.1:7777".parse().unwrap();
        for allow_remote in [false, true] {
            let config = server_config(addr, allow_remote);
            assert!(
                !config.allowed_origins.is_empty(),
                "an empty allow-list turns Origin validation off entirely"
            );
            assert!(config
                .allowed_origins
                .iter()
                .all(|origin| origin.contains("127.0.0.1:7777")));
        }
    }

    /// A reachable bind that refuses every request looks like a bug, so
    /// `--allow-remote` has to widen `Host` as well as the address.
    #[test]
    fn allow_remote_widens_the_host_check_too() {
        let addr: std::net::SocketAddr = "0.0.0.0:7777".parse().unwrap();
        assert!(server_config(addr, true).allowed_hosts.is_empty());
        assert!(
            !server_config(addr, false).allowed_hosts.is_empty(),
            "the default stays loopback-only"
        );
    }
}

/// End-to-end over a real socket.
///
/// Raw HTTP/1.1 rather than a client crate: the whole point is to prove
/// the bytes on the wire, and a dependency that speaks the protocol for
/// us would be proving its own correctness instead of ours.
#[cfg(test)]
mod wire_tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::mcp::server::serve_args::Transport;
    use crate::mcp::server::tools::ToolsServer;

    /// What a `2026-07-28` client attaches to every request, because
    /// statelessness leaves nowhere else to keep it. `clientCapabilities`
    /// is load-bearing beyond this test: the harness reads it to decide
    /// whether to mint a SEP-2663 task.
    const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"wire","version":"0"},"io.modelcontextprotocol/clientCapabilities":{}}"#;

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"wire","version":"0"}}}"#;

    /// Start a server on an ephemeral port and hand back its address.
    ///
    /// The task is left running; the test process owns it for the length
    /// of one test, and dropping the runtime reaps it.
    async fn serve(args: ServeArgs) -> std::net::SocketAddr {
        let listener = bind(&args).await.expect("bind");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = serve_bound(ToolsServer::new(Transport::Http), &args, "hyprpilot", listener).await;
        });

        addr
    }

    /// `version` is the negotiated revision, and it decides which of
    /// rmcp's two paths the request takes: `2026-07-28` is stateless per
    /// SEP-2567, anything older runs the legacy session lifecycle where
    /// a bare `tools/list` is refused until an `initialize` has
    /// established a session.
    async fn post(
        addr: std::net::SocketAddr,
        version: &str,
        method: &str,
        body: &str,
        token: Option<&str>,
    ) -> (u16, String) {
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let mut request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: {version}\r\n\
             Mcp-Method: {method}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(token) = token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);
        stream.write_all(request.as_bytes()).await.expect("write");

        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.expect("read");
        let status = raw
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let body = raw.split_once("\r\n\r\n").map_or(String::new(), |(_, b)| b.to_string());

        (status, body)
    }

    /// The whole stack over a socket: rmcp's transport, our handler, and
    /// the name a client routes tool calls by.
    #[tokio::test]
    async fn a_server_answers_initialize_over_http() {
        let addr = serve(ServeArgs {
            transport: Transport::Http,
            listen: Some("127.0.0.1:0".into()),
            ..ServeArgs::default()
        })
        .await;

        let (status, body) = post(addr, "2025-06-18", "initialize", INITIALIZE, None).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(
            body.contains(crate::config::mcp::DEFAULT_TOOLS_SERVER_NAME),
            "the server must report its own name: {body}"
        );
    }

    /// A stateless HTTP client that holds no subscription stream is
    /// never told when anything changes, so the result it caches must
    /// not claim a day of freshness.
    #[tokio::test]
    async fn http_results_carry_no_stale_ttl() {
        let addr = serve(ServeArgs {
            transport: Transport::Http,
            listen: Some("127.0.0.1:0".into()),
            ..ServeArgs::default()
        })
        .await;

        // Stateless, so `tools/list` stands alone — which is exactly
        // the shape whose cached result this test is about. Under
        // `2026-07-28` each request carries its own protocol metadata,
        // since there is no session left to hold it.
        let request = format!(r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{{META}}}}}"#);
        let (_, body) = post(addr, "2026-07-28", "tools/list", &request, None).await;
        assert!(body.contains("\"ttlMs\":0"), "expected ttlMs 0, got: {body}");
    }

    /// With a token configured, every request needs it — including the
    /// ones rmcp answers before any handler runs.
    #[tokio::test]
    async fn a_configured_token_gates_every_request() {
        let dir = tempfile::tempdir().unwrap();
        let token_file = dir.path().join("token");
        std::fs::write(&token_file, "hunter2").unwrap();

        let addr = serve(ServeArgs {
            transport: Transport::Http,
            listen: Some("127.0.0.1:0".into()),
            token_file: Some(token_file),
            ..ServeArgs::default()
        })
        .await;

        assert_eq!(
            post(addr, "2025-06-18", "initialize", INITIALIZE, None).await.0,
            401,
            "no token"
        );
        assert_eq!(
            post(addr, "2025-06-18", "initialize", INITIALIZE, Some("wrong"))
                .await
                .0,
            401,
            "wrong token"
        );
        let (status, body) = post(addr, "2025-06-18", "initialize", INITIALIZE, Some("hunter2")).await;
        assert_eq!(status, 200, "the right token must pass: {body}");
    }
}
