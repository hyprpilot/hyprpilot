//! Captain-picked path → file-attachment payload. Pairs with the
//! [`crate::completion::source::path::PathSource`] composer source —
//! sources detect the path pattern, this resolver reads the file body
//! into the wire-side attachment shape.

/// Maximum bytes shipped to the webview as a file attachment body.
/// Same cap as the path-completion preview — if a captain attaches a
/// 1GB log file they get the head 256KB and the rest stays on disk.
const FILE_ATTACHMENT_MAX_BYTES: usize = 256 * 1024;

/// Read a file's contents for use as an `@./path` attachment payload.
/// Captain's path-completion commit in the composer pipes through here;
/// the file's text body becomes the attachment's `body`. Binary files
/// resolve to a stub describing the size + path so the agent gets a
/// pointer instead of mojibake. `~` and env-var expansion mirrors the
/// rest of the path surfaces.
pub async fn read_file_for_attachment(path: String) -> Result<serde_json::Value, String> {
    let expanded = crate::paths::resolve_user(&path).to_string_lossy().into_owned();
    let metadata = tokio::fs::metadata(&expanded)
        .await
        .map_err(|err| format!("read_file_for_attachment: stat {expanded}: {err}"))?;
    if metadata.is_dir() {
        return Err(format!("'{expanded}' is a directory; attach a file"));
    }
    let bytes = tokio::fs::read(&expanded)
        .await
        .map_err(|err| format!("read_file_for_attachment: read {expanded}: {err}"))?;
    let truncated = bytes.len() > FILE_ATTACHMENT_MAX_BYTES;
    let head = if truncated {
        &bytes[..FILE_ATTACHMENT_MAX_BYTES]
    } else {
        &bytes[..]
    };

    if head.contains(&0) {
        return Ok(serde_json::json!({
            "path": expanded,
            "body": format!("[binary file — {} bytes]", metadata.len()),
            "binary": true,
            "truncated": false,
        }));
    }
    let body = String::from_utf8_lossy(head).into_owned();
    Ok(serde_json::json!({
        "path": expanded,
        "body": body,
        "binary": false,
        "truncated": truncated,
    }))
}
