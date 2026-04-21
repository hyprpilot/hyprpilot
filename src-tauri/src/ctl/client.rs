use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::rpc::protocol::{Call, JsonRpcVersion, Outcome, Request, RequestId, Response};

/// Synchronous one-shot client. Connects to the daemon socket, writes a
/// single NDJSON request, reads a single reply, returns the `Outcome`.
/// Connection errors for "daemon not running" (`ENOENT` /
/// `ECONNREFUSED`) surface a friendly message; everything else bubbles up
/// as-is.
pub fn call(socket: &Path, call: Call) -> Result<Outcome> {
    let stream = UnixStream::connect(socket).map_err(|err| {
        if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) {
            anyhow!("hyprpilot daemon is not running")
        } else {
            anyhow::Error::new(err).context(format!("failed to connect to {}", socket.display()))
        }
    })?;

    let request = Request {
        jsonrpc: JsonRpcVersion,
        id: RequestId::Number(1),
        call,
    };

    let mut bytes = serde_json::to_vec(&request).context("serialize request")?;
    bytes.push(b'\n');

    let mut writer = stream.try_clone().context("clone socket for write")?;
    writer.write_all(&bytes).context("write request")?;
    writer.flush().context("flush request")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).context("read response line")?;
    if n == 0 {
        bail!("daemon closed the connection without responding");
    }

    let response: Response =
        serde_json::from_str(line.trim_end()).with_context(|| format!("parse response: {}", line.trim_end()))?;
    Ok(response.outcome)
}
