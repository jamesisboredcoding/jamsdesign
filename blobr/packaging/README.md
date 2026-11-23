# Packaging Blobr

Blobr is packaged as a Flatpak with the id `dev.jamsdesign.Blobr`. The Flatpak includes both
the app and `blobr-mcp`.

| File | What it is |
| --- | --- |
| `dev.jamsdesign.Blobr.yml` | The Flatpak manifest (freedesktop 26.08 runtime, Rust from the SDK extension) |
| `cargo-sources.json` | Every crate in `Cargo.lock`, so the build runs offline. Generated |
| `cargo-sources.py` | Regenerates `cargo-sources.json`. Run it whenever `Cargo.lock` changes |
| `dev.jamsdesign.Blobr.desktop` | The launcher. It opens `.jdb` files |
| `dev.jamsdesign.Blobr.metainfo.xml` | AppStream data for software centres |
| `dev.jamsdesign.Blobr.mime.xml` | The `.jdb` file type (`application/vnd.jamsdesign.blobr`) |

The icon is the suite's `img/blobr.svg`.

## Build and install

You need `flatpak-builder` (the `org.flatpak.Builder` Flatpak), the freedesktop 26.08 SDK
and its Rust extension. Install them once:

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.flatpak.Builder org.freedesktop.Sdk//26.08 org.freedesktop.Sdk.Extension.rust-stable//26.08
```

Then, from the repository root:

```sh
python3 blobr/packaging/cargo-sources.py      # only if Cargo.lock changed
flatpak run org.flatpak.Builder --user --install --force-clean \
    --state-dir ~/.cache/blobr-flatpak build-dir blobr/packaging/dev.jamsdesign.Blobr.yml
flatpak run dev.jamsdesign.Blobr
```

The manifest builds the working tree as it is. For a release, swap the `dir` source for a
`git` source pinned to a tag.

## Agents (MCP)

To use the Flatpak's `blobr-mcp` in every Claude Code session:

```sh
claude mcp add --scope user blobr -- flatpak run --command=blobr-mcp dev.jamsdesign.Blobr
```

A running Blobr listens on `$XDG_RUNTIME_DIR/app/dev.jamsdesign.Blobr/blobr.sock`. Flatpak
shares that folder with the host, so a `blobr-mcp` inside or outside the sandbox reaches the
app either way.

## Sandbox permissions

- **Display:** `wayland`, `fallback-x11`, `ipc` and `dri` (GPU).
- **`--filesystem=home`:** Blobr remembers libraries and recent files by path, and re-reads
  linked libraries when they change. The file picker's one-off access isn't enough for that.
- **`xdg-data/fonts:ro`:** fonts installed for your user. System fonts come from the host
  through `/run/host/fonts`.
- **No network.**

Preferences and recent files live in `~/.var/app/dev.jamsdesign.Blobr/config/jamsdesign/`.

## Before publishing on Flathub

- Add a licence. The repository doesn't have one yet; once it does, add `<project_license>`
  to the metainfo.
- Add screenshots to the metainfo.
- Point the manifest at a tagged git release instead of the working tree.
