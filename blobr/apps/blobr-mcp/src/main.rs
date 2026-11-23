//! `blobr-mcp`: an MCP server (stdio) for Blobr documents.
//!
//! Build it once (`cargo build --release -p blobr-mcp`), then add it to Claude Code with
//! `claude mcp add blobr -- /path/to/jamsdesign/target/release/blobr-mcp`. An optional
//! argument opens a file headless at start: `blobr-mcp design.jdb`.
//!
//! While the Blobr app runs, every call goes to it (live edits, undoable there); otherwise
//! calls work on a headless document here. It never switches while the headless document
//! has unsaved changes, so an agent's work can't get stranded.

use blobr_mcp::live::Client;
use blobr_mcp::{FileTarget, INSTRUCTIONS, Target};
use jd_mcp::{Content, Handler, ServerInfo, Tool, ToolResult};
use serde_json::Value;

struct Server {
    file: FileTarget,
    app: Option<Client>,
    /// Told the agent the app is running but its headless work is unsaved.
    warned: bool,
}

impl Handler for Server {
    fn tools(&mut self) -> Vec<Tool> {
        blobr_mcp::tools()
    }

    fn call(&mut self, name: &str, args: Value) -> ToolResult {
        if self.app.is_none() {
            if let Some(client) = Client::connect() {
                if self.file.dirty() {
                    if !self.warned {
                        self.warned = true;
                        let mut out = blobr_mcp::call(&mut self.file, name, args)?;
                        out.push(Content::Text(
                            "Note: the Blobr app is running, but this headless document has unsaved changes, so the tools stay on it. \
                             save_document it, and the next call works live in the app."
                                .into(),
                        ));
                        return Ok(out);
                    }
                } else {
                    self.app = Some(client);
                    self.warned = false;
                }
            }
        }
        if let Some(client) = &mut self.app {
            match client.call(name, &args) {
                Ok(result) => return result,
                Err(e) => {
                    self.app = None;
                    return Err(format!(
                        "Lost the connection to the Blobr app ({e}); if it closed, the next call works headless. \
                         Unsaved changes in the app stay there."
                    ));
                }
            }
        }
        blobr_mcp::call(&mut self.file, name, args)
    }
}

fn main() {
    let mut file = FileTarget::default();
    if let Some(path) = std::env::args().nth(1)
        && let Err(e) = file.open(std::path::Path::new(&path))
    {
        eprintln!("blobr-mcp: {e}");
        std::process::exit(1);
    }
    let info = ServerInfo { name: "blobr", version: env!("CARGO_PKG_VERSION"), instructions: INSTRUCTIONS.into() };
    if let Err(e) = jd_mcp::serve_stdio(&info, &mut Server { file, app: None, warned: false }) {
        eprintln!("blobr-mcp: {e}");
        std::process::exit(1);
    }
}
