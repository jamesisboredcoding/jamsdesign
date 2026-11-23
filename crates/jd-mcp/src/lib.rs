//! A Model Context Protocol server over stdio: newline-delimited JSON-RPC 2.0, the
//! `initialize` handshake, `tools/list` and `tools/call`. Apps describe their tools with
//! [`Tool`] and answer calls through [`Handler`]; everything else (framing, errors, the
//! protocol version) lives here, so Photosplash and Cinematica can reuse it later.
//!
//! Synchronous on purpose: one client, one call at a time, no async runtime.

use std::io::{BufRead, Write};

use base64::Engine;
use serde_json::{Value, json};

/// The newest protocol revision this server speaks; older clients get theirs echoed back.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const KNOWN_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// A tool as clients see it.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: &'static str,
    pub description: String,
    /// JSON Schema for the arguments (an object schema).
    pub input_schema: Value,
    /// Doesn't change anything (clients may run it without asking).
    pub read_only: bool,
}

/// One piece of a tool's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(String),
    /// PNG bytes.
    Png(Vec<u8>),
}

impl Content {
    fn to_json(&self) -> Value {
        match self {
            Content::Text(t) => json!({ "type": "text", "text": t }),
            Content::Png(b) => json!({
                "type": "image",
                "data": base64::engine::general_purpose::STANDARD.encode(b),
                "mimeType": "image/png",
            }),
        }
    }
}

/// What a tool call gives back. An `Err` is shown to the model as a failed call (so it can
/// correct itself), not as a protocol error.
pub type ToolResult = Result<Vec<Content>, String>;

pub trait Handler {
    fn tools(&mut self) -> Vec<Tool>;
    fn call(&mut self, name: &str, args: Value) -> ToolResult;
}

/// Who we are, for `initialize`.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub name: &'static str,
    pub version: &'static str,
    /// Shown to the model once per session: how to use the tools well.
    pub instructions: String,
}

/// Answers one JSON-RPC message. `None` for notifications (and responses), which get no
/// reply.
pub fn handle(info: &ServerInfo, handler: &mut dyn Handler, message: &str) -> Option<Value> {
    let msg: Value = match serde_json::from_str(message) {
        Ok(v) => v,
        Err(e) => return Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    if let Value::Array(batch) = msg {
        // Batches were dropped from the protocol in 2025-06-18, but older clients may send them.
        let replies: Vec<Value> = batch.iter().filter_map(|m| handle(info, handler, &m.to_string())).collect();
        return (!replies.is_empty()).then_some(Value::Array(replies));
    }
    let id = msg.get("id").cloned();
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        // A response to something we never send, or junk.
        return id.map(|id| error(id, -32600, "invalid request"));
    };
    let id = id?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
            let version = if KNOWN_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": info.name, "version": info.version },
                "instructions": info.instructions,
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            let tools: Vec<Value> = handler
                .tools()
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                        "annotations": { "readOnlyHint": t.read_only },
                    })
                })
                .collect();
            json!({ "tools": tools })
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "tools/call needs a name"));
            };
            if !handler.tools().iter().any(|t| t.name == name) {
                return Some(error(id, -32602, &format!("unknown tool {name:?}")));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match handler.call(name, args) {
                Ok(content) => json!({ "content": content.iter().map(Content::to_json).collect::<Vec<_>>(), "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            }
        }
        // We offer no resources or prompts, but answering keeps picky clients happy.
        "resources/list" => json!({ "resources": [] }),
        "prompts/list" => json!({ "prompts": [] }),
        _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Serves stdin/stdout until the client closes stdin. Logs go to stderr.
pub fn serve_stdio(info: &ServerInfo, handler: &mut dyn Handler) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(info, handler, &line) {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// Reads an argument, with a clear message when it's missing or the wrong type.
pub mod args {
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    pub fn get<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
        let v = args.get(key).ok_or_else(|| format!("missing argument {key:?}"))?;
        serde_json::from_value(v.clone()).map_err(|e| format!("argument {key:?}: {e}"))
    }

    pub fn opt<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, String> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("argument {key:?}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl Handler for Echo {
        fn tools(&mut self) -> Vec<Tool> {
            vec![Tool { name: "echo", description: "Says it back".into(), input_schema: json!({"type":"object"}), read_only: true }]
        }
        fn call(&mut self, _: &str, args: Value) -> ToolResult {
            match args.get("say").and_then(Value::as_str) {
                Some(s) => Ok(vec![Content::Text(s.into())]),
                None => Err("say what?".into()),
            }
        }
    }

    fn info() -> ServerInfo {
        ServerInfo { name: "test", version: "0", instructions: "hi".into() }
    }

    #[test]
    fn handshake_list_and_call() {
        let i = info();
        let r = handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["instructions"], "hi");
        assert!(handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let r = handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert_eq!(r["result"]["tools"][0]["name"], "echo");
        assert_eq!(r["result"]["tools"][0]["annotations"]["readOnlyHint"], true);
        let r = handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"echo","arguments":{"say":"blob"}}}"#).unwrap();
        assert_eq!((r["id"].clone(), r["result"]["content"][0]["text"].clone(), r["result"]["isError"].clone()), (json!("a"), json!("blob"), json!(false)));
        let r = handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{}}}"#).unwrap();
        assert_eq!(r["result"]["isError"], true, "tool failures are results the model can read");
        let r = handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}"#).unwrap();
        assert_eq!(r["error"]["code"], -32602);
        let r = handle(&i, &mut Echo, r#"{"jsonrpc":"2.0","id":5,"method":"what"}"#).unwrap();
        assert_eq!(r["error"]["code"], -32601);
        assert_eq!(handle(&i, &mut Echo, "{oops").unwrap()["error"]["code"], -32700);
    }

    #[test]
    fn images_are_base64() {
        assert_eq!(Content::Png(vec![1, 2, 3]).to_json()["data"], "AQID");
    }
}
