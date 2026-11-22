# Cinematica

The jamsdesign video editor: Premiere Pro's layout, shortcuts and editing behaviour, in Rust
with iced 0.14. It builds in the suite's root workspace and shares `crates/` with Gallerion.

Design reference: the Cinematica page of the Figma file "jamsdesign Photo Editor".

## Run

From the repository root:

```sh
cargo run -p cinematica                # optionally: -- clip.mp4 song.wav ...
cargo test -p cinematica -p jd-media -p jd-timeline -p jd-playback
cargo build --release -p cinematica && cinematica/packaging/install-desktop.sh
```

Needs the system FFmpeg (8.x) development files and clang (for ffmpeg-next's bindgen):
`dnf install ffmpeg-devel clang-devel` on Fedora.

## Layout

```
crates/                (shared, at the repository root)
  jd-media     FFmpeg: probe, frame-accurate video decode, audio decode and resampling,
               waveforms, the audio output (cpal), the Source monitor's player thread
  jd-timeline  sequences, tracks and clips in flicks; insert/overwrite/lift/extract,
               ripple, trim, slip, razor, snapping; undo history. No dependencies
  jd-playback  sequence playback: audio mixer, picture following the played audio
  jd-ui        theme, icons, widgets, and chrome.rs (the custom window title bar)
cinematica/
  apps/cinematica/   the editor (iced)
  packaging/         desktop entry and installer for the taskbar icon
```

The video crates never depend on iced; only the app does.
