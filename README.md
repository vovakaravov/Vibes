# Vibes

Native GPU-accelerated Rust + GPUI Internet Radio & Audio-Only YouTube desktop player built for KDE Plasma (zero Electron, zero GTK).

## Highlights

- **Global Radio Directory (RadioBrowser)**: Search 50,000+ stations by name or genre, **Spin the Dial** random high-bitrate picker, local country blending, custom stream URLs, `1`–`9` stereo presets, and automatic reconnect on dropped streams.
- **Audio-Only YouTube (`mpv` + `yt-dlp`)**: Stream music mixes, podcasts, and livestreams without downloading video frames — zero ads, low RAM, `All / Music / Podcasts (20m+) / Live` filters, queue support, variable playback speed (`0.75x–2.0x`), and auto-resume timestamps for long podcasts.
- **Native KDE System Tray (`ksni` StatusNotifierItem) & Close-to-Tray**:
  - Closing the window tears down the GPUI surface while keeping audio alive in the tray.
  - Dynamic volume-modulated tray icon (3 wave bands light with volume; distinct badge icons for Paused, Buffering, Muted, and Idle).
  - Left-click: Play/Pause · Middle-click: Show/Hide Window · Scroll wheel: Volume ±5% · Right-click menu with live track info, Favorites, Sleep Timer, Copy Track Title, and Save Song.
- **MPRIS2 (`zbus`)**: Native Plasma Media Player widget, lock screen controls, hardware media keys, and KDE Connect sync.
- **Smart Save Song (`.opus` + Cover Art + Tags via `lofty`)**: Instead of recording radio streams with DJ commentary/ads, Vibes identifies the song, grabs the clean YouTube audio stream (`.opus` without lossy re-encoding by default), crops the thumbnail to square JPEG cover art via `ffmpeg`, and embeds full artist/album/date/genre/source metadata directly into `~/Music/Radio`.
- **KDE Plasma Theme Sync**: Automatically reads `~/.config/kdeglobals` colors, alongside `Kanagawa Dragon`, `KDE Breeze Dark`, and `Catppuccin Mocha` presets.
- **Live FFT Spectrum Visualizer**: 24-band radix-2 FFT spectrum powered by `parec` (automatically stopped when hidden in tray).

## Shortcuts

- `Space`: Play / Pause
- `1`–`9`: Tune favorite presets 1–9
- `Ctrl+S`: Save current song to Music
- `Ctrl+M`: Toggle Mini Deck compact mode
- `Ctrl+Q`: Quit completely
- `M`: Mute / Unmute
- `Left` / `Right`: Seek YouTube audio `-5s` / `+5s`

(Hovering over the deck's volume slider lets you hold-and-drag or scroll the mouse wheel in 5% increments.)

## Requirements

- Rust toolchain (`cargo`)
- Runtime tools: `mpv`, `yt-dlp`, `ffmpeg`, `parec` (optional `wl-copy` for tray copy)

## Build & Run

```bash
cargo build --release
./target/release/vibes
```
