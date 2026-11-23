# blobr-mcp

An MCP server that lets AI agents (Claude Code and others) read and edit Blobr documents.

## Set up

Build it once, then register the binary (building inside the MCP startup would time out):

    cargo build --release -p blobr-mcp
    claude mcp add blobr -- "$PWD/target/release/blobr-mcp"

Restart Claude Code, and the `blobr` tools appear. Rebuild after pulling changes.

## Modes

- **Live**: while the Blobr app runs, the tools work on the document open in it. Edits
  appear at once, each is an undo step in the app, and the status bar shows
  "Agent connected". The agent can read the selection, select layers, and move the view.
- **Headless**: otherwise the tools work on a document of their own (`open_document`,
  `new_document`, `save_document`). Start with a file: `blobr-mcp design.jdb`.

The server never switches to the app while its headless document has unsaved changes.

## For agents

The `reference` tool holds the node and op formats (`src/reference.md`). Build in batches
with `apply_ops`, then check with `render`.
