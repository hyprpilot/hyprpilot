//! `hyprpilot mcp passthrough` — the HTTP passthrough MCP server.
//!
//! Serves exactly the tools the captain declared under
//! `[[mcp.passthrough.tools]]`. A call POSTs the tool's static `body`
//! with the call arguments laid over it to the tool's `url`, and the
//! response body comes back verbatim as the tool's text. A non-2xx
//! status or an unreachable upstream is `isError`, carrying the url,
//! the status and whatever the upstream said.
//!
//! Knows nothing about any upstream. What a tool means lives in its
//! description and schema, which are listed as written; the server's
//! only job is getting the arguments there and the answer back.
//!
//! The result is text alone, unlike the other servers' structured
//! pairs: the contract is the upstream's bytes, and parsing them into
//! structured content would reshape a response the server does not
//! understand.
//!
//! Stateless like `mcp serve` — the tool list is fixed at startup, so
//! nothing reloads and nothing notifies.

use std::sync::Arc;
use std::time::Duration;

use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;

use crate::config::mcp::{PassthroughTool, DEFAULT_PASSTHROUGH_SERVER_NAME, DEFAULT_PASSTHROUGH_TIMEOUT_SECONDS};

use super::rpc::{tool_error, wait_for_shutdown, RESULT_CACHE_SCOPE};
use super::serve_args::{ServeArgs, Transport};

/// Args for `hyprpilot mcp passthrough`.
///
/// The tools ride argv for the reason `--skill-dir` does: the `mcp`
/// branch never loads config, and only the launcher knows which
/// profile's `[mcp.passthrough]` block was picked.
#[derive(Debug, Args, Clone)]
pub struct PassthroughArgs {
    #[command(flatten)]
    pub serve: ServeArgs,

    /// JSON-encoded tool. Repeatable, one per tool.
    ///
    /// Shape: `{"name":"…","description":"…","inputSchema":{…},"url":"https://…","body":{…}}`
    /// — the `[[mcp.passthrough.tools]]` entry, validated the same way.
    #[arg(long = "tool", value_name = "JSON", value_parser = parse_tool_arg)]
    pub tools: Vec<PassthroughTool>,

    /// Per-request timeout, connect through the last byte.
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_PASSTHROUGH_TIMEOUT_SECONDS,
          value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout_seconds: u64,
}

pub(crate) fn parse_tool_arg(raw: &str) -> Result<PassthroughTool, String> {
    let tool: PassthroughTool = serde_json::from_str(raw).map_err(|e| {
        format!("--tool must be a JSON object `{{\"name\":\"...\",\"inputSchema\":{{...}},\"url\":\"...\"}}`: {e}")
    })?;
    garde::Validate::validate(&tool).map_err(|e| format!("--tool {}: {e}", tool.name))?;

    Ok(tool)
}

/// The passthrough server.
///
/// `Clone` for the HTTP transport's per-request handler; the tool list
/// is shared and `reqwest::Client` is a handle onto one pool.
#[derive(Clone)]
pub struct PassthroughServer {
    tools: Arc<[PassthroughTool]>,
    client: reqwest::Client,
    transport: Transport,
}

impl PassthroughServer {
    fn new(args: &PassthroughArgs) -> anyhow::Result<Self> {
        let mut seen = std::collections::HashSet::new();
        if let Some(dup) = args.tools.iter().find(|t| !seen.insert(t.name.as_str())) {
            anyhow::bail!("mcp: --tool '{}' is declared twice", dup.name);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(args.timeout_seconds))
            .build()?;

        Ok(Self {
            tools: args.tools.clone().into(),
            client,
            transport: args.serve.transport,
        })
    }

    async fn forward(
        &self,
        tool: &PassthroughTool,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> CallToolResponse {
        let mut payload = tool.body.clone().unwrap_or_default();
        payload.extend(arguments);
        let url = &tool.url;

        let response = match self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::Value::Object(payload).to_string())
            .send()
            .await
        {
            Ok(response) => response,
            // `{:#}` walks the source chain; reqwest's own Display stops
            // at "error sending request", which names no cause.
            Err(err) => return tool_error(format!("{url} unreachable: {:#}", anyhow::Error::from(err))),
        };
        let status = response.status();
        let text = match response.text().await {
            Ok(text) => text,
            Err(err) => {
                return tool_error(format!(
                    "{url} returned {} with an unreadable body: {:#}",
                    status.as_u16(),
                    anyhow::Error::from(err)
                ))
            }
        };
        if !status.is_success() {
            return tool_error(format!("{url} returned {}: {text}", status.as_u16()));
        }

        CallToolResult::success(vec![ContentBlock::text(text)]).into()
    }
}

impl ServerHandler for PassthroughServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        super::rpc::supported_protocol_versions()
    }

    /// Record the negotiated protocol version as the peer's, per
    /// `rpc::initialize_negotiated`.
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, rmcp::ErrorData> {
        Ok(super::rpc::initialize_negotiated(self, request, &context))
    }

    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        // Fixed for the life of the process.
        let mut tools = rmcp::model::ToolsCapability::default();
        tools.list_changed = Some(false);
        caps.tools = Some(tools);

        ServerInfo::new(caps)
            .with_server_info(Implementation::new(
                DEFAULT_PASSTHROUGH_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(
                "Hyprpilot passthrough MCP server. Each tool forwards its arguments as a JSON POST \
                 to an HTTP endpoint the captain configured and returns the response body verbatim. \
                 A non-2xx status or an unreachable endpoint comes back as an error naming the url. \
                 What a tool does is in its own description.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let tools = self
            .tools
            .iter()
            .map(|t| {
                Tool::new_with_raw(
                    t.name.clone(),
                    t.description.clone().map(Into::into),
                    Arc::new(t.input_schema.clone()),
                )
            })
            .collect();

        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(self.transport.result_ttl_ms())
            .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let Some(tool) = self.tools.iter().find(|t| t.name == request.name.as_ref()) else {
            return Err(rmcp::ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {}", request.name),
                None,
            ));
        };

        Ok(self.forward(tool, request.arguments.unwrap_or_default()).await)
    }
}

/// Run the passthrough server.
pub async fn run_passthrough(args: PassthroughArgs, _config: super::ConfigSource) -> anyhow::Result<()> {
    tracing::info!(tools = args.tools.len(), "mcp: starting the passthrough server");

    let handler = PassthroughServer::new(&args)?;
    if args.serve.transport == Transport::Http {
        return super::http::serve_http(handler, &args.serve, DEFAULT_PASSTHROUGH_SERVER_NAME).await;
    }

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = super::rpc::serve_from_first_byte(handler, (stdin, stdout));

    wait_for_shutdown(running).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// A one-shot upstream answering `status` with `reply`, handing back
    /// the request body it received.
    async fn stub_upstream(status: &'static str, reply: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/decide", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            let body = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break body.to_string();
                    }
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            body
        });

        (url, handle)
    }

    fn server(url: &str) -> PassthroughServer {
        PassthroughServer::new(&PassthroughArgs {
            serve: ServeArgs::default(),
            tools: vec![tool(url)],
            timeout_seconds: 5,
        })
        .unwrap()
    }

    fn tool(url: &str) -> PassthroughTool {
        parse_tool_arg(
            &serde_json::json!({
                "name": "decide",
                "inputSchema": { "type": "object" },
                "url": url,
                "body": { "model": "m", "stream": false },
            })
            .to_string(),
        )
        .unwrap()
    }

    fn outcome(response: CallToolResponse) -> (bool, String) {
        let CallToolResponse::Complete(result) = response else {
            panic!("a passthrough call always completes");
        };
        let value = serde_json::to_value(result).unwrap();
        (
            value["isError"].as_bool().unwrap_or(false),
            value["content"][0]["text"].as_str().unwrap_or_default().to_string(),
        )
    }

    fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().unwrap().clone()
    }

    #[tokio::test]
    async fn a_call_posts_the_body_under_its_arguments_and_returns_the_reply_verbatim() {
        let (url, upstream) = stub_upstream("200 OK", "{\"decision\": \"yes\"}\n").await;
        let passthrough = server(&url);

        let (is_error, text) = outcome(
            passthrough
                .forward(
                    &passthrough.tools[0],
                    args(serde_json::json!({ "question": "q", "model": "override" })),
                )
                .await,
        );

        assert!(!is_error);
        assert_eq!(text, "{\"decision\": \"yes\"}\n", "the upstream's bytes, untouched");
        let sent: serde_json::Value = serde_json::from_str(&upstream.await.unwrap()).unwrap();
        assert_eq!(
            sent,
            serde_json::json!({ "model": "override", "stream": false, "question": "q" }),
            "an argument wins over the static body; the rest of the body survives"
        );
    }

    #[tokio::test]
    async fn an_upstream_error_status_is_an_error_carrying_what_it_said() {
        let (url, _upstream) = stub_upstream("503 Service Unavailable", "model loading").await;
        let passthrough = server(&url);

        let (is_error, text) = outcome(passthrough.forward(&passthrough.tools[0], Default::default()).await);

        assert!(is_error);
        assert_eq!(text, format!("{url} returned 503: model loading"));
    }

    #[tokio::test]
    async fn an_unreachable_upstream_is_an_error_naming_the_url() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/decide", listener.local_addr().unwrap());
        drop(listener);
        let passthrough = server(&url);

        let (is_error, text) = outcome(passthrough.forward(&passthrough.tools[0], Default::default()).await);

        assert!(is_error);
        assert!(text.starts_with(&format!("{url} unreachable: ")), "got: {text}");
    }

    #[test]
    fn a_tool_arg_is_validated_like_the_config_entry() {
        let err = parse_tool_arg(r#"{"name":"t","inputSchema":{},"url":"ftp://x/"}"#).expect_err("ftp is refused");
        assert!(err.contains("must be http or https"), "got: {err}");
        parse_tool_arg("{not json").expect_err("garbage must not decay into no tool");
    }

    /// Dispatch is by name, so the second would be listed and unreachable.
    #[test]
    fn a_tool_passed_twice_fails_startup() {
        let err = PassthroughServer::new(&PassthroughArgs {
            serve: ServeArgs::default(),
            tools: vec![tool("http://127.0.0.1/a"), tool("http://127.0.0.1/b")],
            timeout_seconds: 5,
        })
        .err()
        .expect("duplicate tool names");
        assert!(err.to_string().contains("declared twice"), "got: {err}");
    }

    /// The whole contract over the wire: the listing carries the declared
    /// tool with the cache stamps `2026-07-28` requires, and a call
    /// reaches the upstream and comes back.
    #[tokio::test]
    async fn the_declared_tool_is_listed_and_callable_over_stdio() {
        let (url, _upstream) = stub_upstream("200 OK", "forwarded").await;
        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
        let running = super::super::rpc::serve_from_first_byte(server(&url), (server_rx, server_tx));

        for line in [
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"decide","arguments":{"question":"q"}}}"#,
        ] {
            client_tx.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        }
        client_tx.flush().await.unwrap();

        let mut replies = std::collections::HashMap::new();
        let mut lines = BufReader::new(client_rx).lines();
        while replies.len() < 3 {
            let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                .await
                .expect("the server answers")
                .unwrap()
                .expect("the stream stays open");
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            if let Some(id) = value.get("id").and_then(serde_json::Value::as_i64) {
                replies.insert(id, value);
            }
        }
        drop(client_tx);
        running.cancel().await.ok();

        assert_eq!(
            replies[&0]["result"]["serverInfo"]["name"],
            DEFAULT_PASSTHROUGH_SERVER_NAME
        );
        let listing = &replies[&1]["result"];
        assert_eq!(listing["tools"][0]["name"], "decide");
        assert!(listing["ttlMs"].is_number() && listing["cacheScope"].is_string());
        assert_eq!(replies[&2]["result"]["content"][0]["text"], "forwarded");
    }
}
