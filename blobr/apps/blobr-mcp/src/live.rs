//! The bridge to a running Blobr: the app listens on a Unix socket under `$XDG_RUNTIME_DIR`;
//! `blobr-mcp` sends it each tool call as one JSON line (`{"name", "args"}`) and gets one
//! line back (`{"ok": [content]}` or `{"err": "..."}`, images as base64 PNG). The app runs
//! the same [`crate::call`] on its own document.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use jd_mcp::{Content, ToolResult};
use serde_json::{Value, json};

/// Blobr's app id (also its Flatpak id).
pub const APP_ID: &str = "dev.jamsdesign.Blobr";

/// Where a running Blobr listens: `$XDG_RUNTIME_DIR/app/dev.jamsdesign.Blobr/blobr.sock`.
/// That folder is the one a Flatpak'd Blobr shares with the host, so `blobr-mcp` finds the
/// app whether either of them is sandboxed or not. Only under `$XDG_RUNTIME_DIR` (private to
/// the user); with no runtime dir there's no live mode.
pub fn socket_path() -> Option<PathBuf> {
    Some(runtime_dir()?.join("app").join(APP_ID).join("blobr.sock"))
}

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
}

pub fn request(name: &str, args: &Value) -> String {
    json!({ "name": name, "args": args }).to_string()
}

pub fn parse_request(line: &str) -> Result<(String, Value), String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("bad request: {e}"))?;
    let name = v.get("name").and_then(Value::as_str).ok_or("request has no name")?.to_owned();
    Ok((name, v.get("args").cloned().unwrap_or_else(|| json!({}))))
}

pub fn encode(result: &ToolResult) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    match result {
        Ok(content) => {
            let parts: Vec<Value> = content
                .iter()
                .map(|c| match c {
                    Content::Text(t) => json!({ "text": t }),
                    Content::Png(b) => json!({ "png": b64.encode(b) }),
                })
                .collect();
            json!({ "ok": parts }).to_string()
        }
        Err(e) => json!({ "err": e }).to_string(),
    }
}

pub fn decode(line: &str) -> ToolResult {
    let b64 = base64::engine::general_purpose::STANDARD;
    let v: Value = serde_json::from_str(line).map_err(|e| format!("bad reply from Blobr: {e}"))?;
    if let Some(e) = v.get("err").and_then(Value::as_str) {
        return Err(e.to_owned());
    }
    let parts = v.get("ok").and_then(Value::as_array).ok_or("bad reply from Blobr")?;
    parts
        .iter()
        .map(|p| {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                Ok(Content::Text(t.to_owned()))
            } else if let Some(d) = p.get("png").and_then(Value::as_str) {
                b64.decode(d).map(Content::Png).map_err(|e| format!("bad image from Blobr: {e}"))
            } else {
                Err("bad reply from Blobr".into())
            }
        })
        .collect()
}

/// `blobr-mcp`'s end: one connection to the app.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    /// Connects if Blobr is running.
    pub fn connect() -> Option<Self> {
        let stream = UnixStream::connect(socket_path()?).ok()?;
        // Renders of big pages, or the app being busy, can take a while.
        stream.set_read_timeout(Some(Duration::from_secs(120))).ok()?;
        let writer = stream.try_clone().ok()?;
        Some(Self { reader: BufReader::new(stream), writer })
    }

    /// One call. An `Err` from the outer result means the connection broke.
    pub fn call(&mut self, name: &str, args: &Value) -> std::io::Result<ToolResult> {
        writeln!(self.writer, "{}", request(name, args))?;
        self.writer.flush()?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Blobr closed the connection"));
        }
        Ok(decode(&line))
    }
}

/// The app's end: binds the socket unless another Blobr already serves it (a stale socket
/// from a crashed app is replaced).
pub fn bind() -> Option<UnixListener> {
    let path = socket_path()?;
    let dir = path.parent()?;
    if !dir.exists() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).ok()?;
        // `recursive` leaves existing parents alone; the app folder itself stays private.
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return None;
        }
        let _ = std::fs::remove_file(&path);
    }
    UnixListener::bind(&path).ok()
}

/// Serves one connection: reads requests, answers each with `answer`, until the client
/// goes away.
pub fn serve(stream: UnixStream, mut answer: impl FnMut(String, Value) -> ToolResult) {
    let Ok(mut writer) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let result = match parse_request(&line) {
            Ok((name, args)) => answer(name, args),
            Err(e) => Err(e),
        };
        if writeln!(writer, "{}", encode(&result)).and_then(|_| writer.flush()).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileTarget;

    #[test]
    fn wire_round_trip() {
        let r: ToolResult = Ok(vec![Content::Text("hi".into()), Content::Png(vec![0, 1, 2, 255])]);
        assert_eq!(decode(&encode(&r)), r);
        let e: ToolResult = Err("no".into());
        assert_eq!(decode(&encode(&e)), e);
        assert_eq!(parse_request(&request("get_tree", &json!({ "depth": 2 }))).unwrap(), ("get_tree".into(), json!({ "depth": 2 })));
    }

    #[test]
    fn a_client_talks_to_a_served_target() {
        let (a, b) = UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || {
            let mut t = FileTarget::default();
            serve(b, |name, args| crate::call(&mut t, &name, args));
        });
        let writer = a.try_clone().unwrap();
        let mut c = Client { reader: BufReader::new(a), writer };
        let out = c.call("apply_ops", &json!({ "ops": [{ "op": "create", "parent": 1, "node": { "type": "rectangle", "name": "R" } }] })).unwrap();
        assert!(matches!(&out.unwrap()[0], Content::Text(t) if t.starts_with("Applied.")));
        assert!(matches!(c.call("render", &json!({ "id": 1 })).unwrap().unwrap()[0], Content::Png(_)));
        assert!(c.call("render", &json!({ "id": 99 })).unwrap().is_err());
        drop(c);
        server.join().unwrap();
    }
}
