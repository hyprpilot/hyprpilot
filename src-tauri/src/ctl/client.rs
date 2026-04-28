use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::rpc::protocol::{Outcome, RequestId, Response};

/// Factory that knows how to reach the daemon. Each `ctl` handler
/// receives one of these and calls `connect()` to get a live
/// [`CtlConnection`]. The indirection is load-bearing for
/// `StatusHandler --watch`, which loops `client.connect()` with
/// back-off after transport errors so waybar keeps rendering
/// regardless of daemon state; one-shot handlers just call it once.
#[derive(Debug, Clone)]
pub struct CtlClient {
    socket: PathBuf,
}

impl CtlClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self { socket: socket.into() }
    }

    /// Open a fresh connection to the configured socket. Maps
    /// `ENOENT` / `ECONNREFUSED` to the friendly "hyprpilot daemon is
    /// not running" message; every other `io::Error` bubbles with the
    /// socket path in the context chain.
    pub fn connect(&self) -> Result<CtlConnection> {
        CtlConnection::connect(&self.socket)
    }
}

/// Synchronous client owning one unix-socket connection. Every CLI
/// round-trip goes through [`CtlConnection::call`], which writes one
/// NDJSON request line and reads one response line back. Callers that
/// want a typed result deserialise [`Outcome::Success { result }`]
/// themselves; callers that subscribe to a notification stream consume
/// the connection via [`CtlConnection::into_reader`] after the initial
/// reply lands.
///
/// Request ids are per-call UUID v4 strings. There is no monotonic
/// counter on the client side: each call embeds a fresh
/// `uuid::Uuid::new_v4().to_string()` into `RequestId::String`, which
/// the daemon echoes back verbatim. The one-connection-per-process
/// lifecycle of `ctl` makes id uniqueness trivial; UUIDs keep that
/// true even if we ever start multiplexing connections.
///
/// Connection errors for "daemon not running" (`ENOENT` /
/// `ECONNREFUSED`) surface as a friendly message; everything else
/// bubbles up as-is.
pub struct CtlConnection {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl CtlConnection {
    /// Connect to `socket`. Returns an error mapped to the friendly
    /// "hyprpilot daemon is not running" message on `ENOENT` /
    /// `ECONNREFUSED`.
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket).map_err(|err| {
            if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) {
                anyhow!("hyprpilot daemon is not running")
            } else {
                anyhow::Error::new(err).context(format!("failed to connect to {}", socket.display()))
            }
        })?;
        let writer = stream.try_clone().context("clone socket for write")?;
        Ok(Self {
            writer,
            reader: BufReader::new(stream),
        })
    }

    fn read(&mut self, ctx: &'static str) -> Result<String> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).context(ctx)?;
        if n == 0 {
            bail!("daemon closed the connection without responding");
        }
        Ok(line)
    }

    fn write(&mut self, envelope: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(envelope).context("serialize request")?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).context("write request")?;
        self.writer.flush().context("flush request")?;
        Ok(())
    }

    /// Single JSON-RPC round-trip. Writes one NDJSON request line,
    /// reads one line back, returns the raw [`Outcome`] so the caller
    /// decides how to handle `Success` vs `Error` (e.g. `status/get`
    /// falls back to the offline sentinel on error rather than
    /// propagating).
    pub fn call(&mut self, method: &str, params: Value) -> Result<Outcome> {
        let id = RequestId::String(uuid::Uuid::new_v4().to_string());
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write(&envelope)?;

        let line = self.read("read response line")?;
        let response: Response =
            serde_json::from_str(line.trim_end()).with_context(|| format!("parse response: {}", line.trim_end()))?;
        Ok(response.outcome)
    }

    /// Consume the connection and return the underlying buffered
    /// reader. Used by subscription streams (see
    /// `ctl::handlers::status::subscribe_status`) that keep draining
    /// NDJSON notifications long after the originating request has
    /// returned — they own the reader for the rest of the connection's
    /// life; the writer side drops here, which is fine because
    /// subscriptions are server-push only.
    pub fn into_reader(self) -> BufReader<UnixStream> {
        self.reader
    }
}
