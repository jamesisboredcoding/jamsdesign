//! `blobr-mcp`: an MCP server (stdio) for Blobr documents. Add it to Claude Code with
//! `claude mcp add blobr -- cargo run -q --release -p blobr-mcp` (or the built binary), then
//! optionally pass a file to open: `blobr-mcp path/to/design.jdb`.

use blobr_mcp::{FileTarget, INSTRUCTIONS, Target};
use jd_mcp::{Handler, ServerInfo, Tool, ToolResult};
use serde_json::Value;

struct Server {
    file: FileTarget,
}

impl Handler for Server {
    fn tools(&mut self) -> Vec<Tool> {
        blobr_mcp::tools()
    }

    fn call(&mut self, name: &str, args: Value) -> ToolResult {
        blobr_mcp::call(&mut self.file, name, args)
    }
}

fn main() {
    let mut file = FileTarget::default();
    if let Some(path) = std::env::args().nth(1) {
        if let Err(e) = file.open(std::path::Path::new(&path)) {
            eprintln!("blobr-mcp: {e}");
            std::process::exit(1);
        }
    }
    let info = ServerInfo { name: "blobr", version: env!("CARGO_PKG_VERSION"), instructions: INSTRUCTIONS.into() };
    if let Err(e) = jd_mcp::serve_stdio(&info, &mut Server { file }) {
        eprintln!("blobr-mcp: {e}");
        std::process::exit(1);
    }
}
