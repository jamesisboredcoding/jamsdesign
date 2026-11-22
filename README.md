<div align="center">

<img src="img/jamsdesign.svg" alt="jamsdesign logo" width="128" height="128">

# jamsdesign

**A Linux-first creative suite, written in Rust.**

Native, GPU-accelerated apps for photo and video work, with the shortcuts and workflows you already know.

![Rust](https://img.shields.io/badge/Rust-2024-B7410E?logo=rust&logoColor=white)
![iced](https://img.shields.io/badge/UI-iced%200.14-3D3846)
![wgpu](https://img.shields.io/badge/GPU-wgpu-1C71D8)
![Platform](https://img.shields.io/badge/platform-Linux-F5C211?logo=linux&logoColor=black)
![Status](https://img.shields.io/badge/status-early%20development-E01B5A)

</div>

---

## The apps

<table>
  <tr>
    <td align="center" width="50%">
      <img src="img/photosplash.svg" alt="Photosplash icon" width="96" height="96"><br>
      <h3>Photosplash</h3>
      <em>Photo editor</em>
    </td>
    <td align="center" width="50%">
      <img src="img/cinematica.svg" alt="Cinematica icon" width="96" height="96"><br>
      <h3>Cinematica</h3>
      <em>Video editor</em>
    </td>
  </tr>
  <tr>
    <td valign="top">
      Layer-based raster editing with all 24 blend modes rendered on the GPU, pressure-sensitive
      brushes for pen tablets, selections, Free Transform, adjustments and filters with live
      preview, and its own <code>.jdp</code> document format. Photoshop-compatible shortcuts
      throughout.
    </td>
    <td valign="top">
      A timeline-based video editor inspired by Premiere, built on the same core, renderer and
      UI toolkit as Photosplash. In early development.
    </td>
  </tr>
</table>

More apps will join the suite later.

## Highlights

- **Native and fast.** Pure Rust with a wgpu renderer; pixel work is tiled, copy-on-write and parallel.
- **Pen tablet support.** Pressure and tilt on Wayland, including KDE.
- **Familiar.** Tools, menus and shortcuts follow the industry-standard layout, so there is nothing new to learn.
- **Shared foundation.** Every app uses the same `jd-*` crates for colour, documents, GPU rendering and UI.

## Getting started

You need a recent stable Rust toolchain (edition 2024) and a Linux desktop with a Vulkan-capable GPU.
Cinematica also needs the system FFmpeg development libraries.

```sh
# Photosplash
cd photosplash
cargo run -p photosplash -- path/to/photo.jpg

# Cinematica
cd cinematica
cargo run -p cinematica

# Tests
cargo test --workspace
```

Each app has its own README with full details, shortcuts and status:
[Photosplash](photosplash/README.md) · [Cinematica](cinematica/README.md)

## Repository layout

```
jamsdesign/
├── photosplash/    git branch `photosplash`   (photo editor workspace)
├── cinematica/   git worktree, branch `cinematica`   (video editor workspace)
└── img/          suite and app logos (SVG)
```

The workspaces share the same crates (`jd-media` and `jd-timeline` are Cinematica only for now):

| Crate         | Purpose                                                                  |
| ------------- | ------------------------------------------------------------------------ |
| `jd-core`     | Colour maths (linear RGBA, sRGB, HSV). No UI dependencies.               |
| `jd-raster`   | Document model: layers, tiles, undo history, blend modes, brush engine.  |
| `jd-io`       | Image import and export, `.jdp` documents.                               |
| `jd-gpu`      | wgpu renderer for compositing and blending.                              |
| `jd-ui`       | iced theme tokens, icons and shared widgets.                             |
| `jd-media`    | Video and audio probing, decoding and playback via FFmpeg.               |
| `jd-timeline` | Sequences, tracks and clips with pure edit operations and undo.          |

## Design

Interface mockups, icons and logos live in Figma. The app icons follow the GNOME / Adwaita
style so they sit naturally on a Linux desktop.
