# Blobr plugins

A plugin is a program in Blobr's plugins folder, written in any language. The Plugins menu
lists every executable file there; choosing one runs it, and it edits the open document with
the same tools agents use over MCP. Every change it makes is an undo step.

## Where plugins go

`~/.local/share/jamsdesign/blobr/plugins/`, or for the Flatpak,
`~/.var/app/io.github.jamesisboredcoding.jamsdesign.Blobr/data/jamsdesign/blobr/plugins/`.

**Plugins > Open Plugins Folder...** creates the folder and opens it. The file has to be executable
(`chmod +x`). The menu shows its name without the extension, with `-` and `_` as spaces, and
picks up new files within a couple of seconds.

## How a plugin talks to Blobr

Blobr sets `BLOBR_SOCKET` to a Unix socket. Send one JSON object per line and read one JSON
line back:

```json
{"name": "get_selection", "args": {}}
```

The reply is `{"ok": [{"text": "..."}, {"png": "<base64>"}]}` or `{"err": "message"}`.

The tools and their arguments are the ones `blobr-mcp` offers. `apply_ops` is the one that
edits, and the `reference` tool returns the full node and op format. It's also in
[`apps/blobr-mcp/src/reference.md`](apps/blobr-mcp/src/reference.md).

When the plugin exits with an error, Blobr shows the last line it printed to stderr in the
status bar. Anything it prints to stdout is ignored.

## Example

[`plugins/lorem-ipsum.py`](plugins/lorem-ipsum.py) fills the selected text layers with placeholder
text. Copy it into the plugins folder to try it.

## Notes

- Plugins run as you, with your permissions. Only install ones you trust.
- In the Flatpak a plugin runs inside Blobr's sandbox. It can use what the runtime has (Python
  3, a shell), but not programs installed only on the host.
- Only the first Blobr window you open has the socket, so run plugins from that one.
