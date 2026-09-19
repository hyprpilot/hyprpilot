//! `hyprpilot mcp serve` — the general-tools MCP server.
//!
//! The surface that is neither a skills read nor an agent launch.
//! `open` lives here today; anything later that doesn't earn a
//! dedicated server lands here too.
//!
//! Kept off the skills server for the same reason the harness is: one
//! process per surface means one catalog entry, one approval policy,
//! and one blast radius. A captain who wants skills but not OS-level
//! side effects sets `mcp.serve.enabled = false` and still gets them.
//!
//! Holds no state — every tool is a straight call through to the OS —
//! so unlike the other two servers there is nothing to reload or reap.

use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;

use crate::config::mcp::{DEFAULT_HARNESS_SERVER_NAME, DEFAULT_SKILLS_SERVER_NAME, DEFAULT_TOOLS_SERVER_NAME};

use super::rpc::{
    object_schema, require_string, structured_with_text, tool_error, wait_for_shutdown, RESULT_CACHE_SCOPE,
};
use super::serve_args::{ServeArgs, Transport};

/// Args for `hyprpilot mcp serve`. Nothing of its own — the server is
/// stateless and takes no catalog — beyond how it is served.
#[derive(Debug, Args, Clone)]
pub struct ToolsArgs {
    #[command(flatten)]
    pub serve: ServeArgs,
}

/// The general-tools server.
///
/// `Clone` because the HTTP transport builds one handler and clones it
/// per request; there is no state behind it, so the clone is free.
#[derive(Clone, Copy)]
pub struct ToolsServer {
    transport: Transport,
}

impl ToolsServer {
    pub(super) fn new(transport: Transport) -> Self {
        Self { transport }
    }
}

impl ServerHandler for ToolsServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        super::rpc::supported_protocol_versions()
    }

    /// Record the negotiated protocol version as the peer's, per
    /// `rpc::initialize_negotiated`.
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
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
                DEFAULT_TOOLS_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(self.instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![Tool::new_with_raw(
            "open",
            Some(
                "Open a URL, file path, or directory in the OS default handler. \
                 Uses `xdg-open` on Linux, `open` on macOS, `start` on Windows. \
                 The MCP sidecar is a plain stdio process — this is a native OS \
                 call."
                    .into(),
            ),
            open_object_schema(),
        )])
        .with_ttl_ms(self.transport.result_ttl_ms())
        .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "open" => {
                let path = require_string(&args, "path")?;
                match open::that_detached(path) {
                    Ok(()) => Ok(structured_with_text(
                        format!("Opened {path}"),
                        serde_json::json!({ "opened": path }),
                    )),
                    Err(err) => Ok(tool_error(format!("open failed: {err}"))),
                }
            }
            other => Err(rmcp::ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}

impl ToolsServer {
    fn instructions(&self) -> String {
        format!(
            "Hyprpilot general-tools MCP server. Use `open` to open a URL, \
             file, or directory in the OS default handler. Skills live on the \
             separate `{DEFAULT_SKILLS_SERVER_NAME}` server (`list_skills` / \
             `read_skill`); agent sessions on `{DEFAULT_HARNESS_SERVER_NAME}` \
             (`spawn` / `session_*`)."
        )
    }
}

fn open_object_schema() -> std::sync::Arc<serde_json::Map<String, serde_json::Value>> {
    object_schema(
        serde_json::json!({
            "path": {
                "type": "string",
                "description": "URL, file path, or directory path to open in the OS default \
                                handler. Accepts `https://`, `file://`, absolute paths, and \
                                relative paths — the same shapes `xdg-open` / `open` / `start` \
                                accept natively.",
            },
        }),
        &["path"],
    )
}

/// Run the general-tools server.
pub async fn run_tools(args: ToolsArgs, _config: super::ConfigSource) -> anyhow::Result<()> {
    tracing::info!("mcp: starting the general-tools server");

    let handler = ToolsServer::new(args.serve.transport);
    if args.serve.transport == Transport::Http {
        // `open` hands a URL or path to the OS default handler, so over
        // a shared port this is a way to make the host open whatever a
        // caller names. Same class as the harness, smaller blast radius.
        return super::http::serve_http(handler, &args.serve, DEFAULT_TOOLS_SERVER_NAME).await;
    }

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = super::rpc::serve_from_first_byte(handler, (stdin, stdout));

    wait_for_shutdown(running).await;

    Ok(())
}
