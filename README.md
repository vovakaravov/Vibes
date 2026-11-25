<p align="center">
  <img src="assets/vibes.svg" alt="Vibes" width="88" height="88" />
</p>

<h1 align="center">Vibes</h1>

<p align="center">
  Native GPU-accelerated Rust + <a href="https://gpui.rs">GPUI</a> Internet Radio, Audio-Only YouTube, Full Albums &amp; Offline Library player built for KDE Plasma (zero Electron, zero GTK).
</p>

---

## Highlights

- **Global Radio Directory (RadioBrowser):** Search 50,000+ stations by name or genre, **Spin the Dial** random high-bitrate picker, local country blending, custom stream URLs, `1`–`9` favorite presets, cached station favicons, and automatic reconnect on dropped streams.
- **Audio-Only YouTube (`mpv` + `yt-dlp`):** Stream music mixes, podcasts, and livestreams without decoding video frames — zero ads, low RAM (bounded `16 MiB` forward / `8 MiB` back `mpv` demuxer cache and stripped ThinLTO release builds), `All` / `Music` / `Podcasts (20m+)` / `Live` filters, Up Next queue, variable playback speed (`0.75x–2.0x`), thumbnails, and auto-resume timestamps for long podcasts.
- **Full Albums & Offline Local Library:**
  - Browse and search full YouTube Music albums with tracklists and square cover art, and download complete tagged albums into `~/Music` (`Artist / Album`, `Album - Name`, or `Artist - Album` folder layouts).
  - Built-in **Local Library** scans `~/Music` and `~/Music/Radio` for offline album & single playback with embedded artwork and metadata backed by a virtualized album grid and frame-safe 64-entry LRU image cache.
- **Smart Save Song (`.opus` / `.m4a` / `.mp3` / `.ogg` / `.flac` + Cover Art + Tags via `lofty`):** Identifies radio tracks, downloads the clean YouTube audio stream (`.opus` by default, configurable format and `Flat` / `By Artist` folder layout in `~/Music/Radio`), crops thumbnails to square JPEG cover art via `ffmpeg`, and embeds full artist/album/date/genre/source tags plus lyrics.
- **Embedded & Live Synced Lyrics (`lofty` + LRCLIB):**
  - Automatically fetches synced LRC (or plain) lyrics from LRCLIB and embeds them natively during Save Song and Full Album downloads (`USLT` via `ItemKey::UnsyncLyrics` for ID3v2/MP3; `LYRICS` / `©lyr` via `ItemKey::Lyrics` for Opus, Ogg, FLAC, and M4A).
  - Right-side **Synced Lyrics** drawer (`L` or player deck **Lyrics** button) reads embedded tags offline-first with LRCLIB fallback, auto-scrolls the active line, supports click-to-seek, and includes one-click inline **Embed** for local files.
  - One-click **Embed Lyrics** library backfill (in Local Library and Settings) with live progress and cancel support.
- **Click-Through Wayland Desktop Lyrics Overlay (`zwlr_layer_shell_v1`):**
  - Native bottom-right floating lyrics card (`Layer::Top`, empty `wl_region` input shape for true click-through, `KeyboardInteractivity::None`, HiDPI `PreferredBufferScale`, `cosmic-text` shaping, premultiplied BGRA `wl_shm` surface) that sits cleanly above the KDE system tray without stealing focus or mouse clicks.
  - Toggled from the system tray right-click menu (**Lyrics Overlay**), persists across launches, works while minimized to the tray, and consumes zero extra RAM when disabled.
- **Native KDE System Tray (`ksni` StatusNotifierItem) & Close-to-Tray:**
  - Closing the window tears down the GPUI surface while keeping audio (and the lyrics overlay, if enabled) alive in the tray.
  - **Circular Volume Disc** tray icon (`32×32` vinyl disc with clockwise crimson volume progress ring, plus distinct visual states for Idle, Paused, Buffering, and Muted).
  - Left-click: Play/Pause · Middle-click: Show/Hide Window · Scroll wheel: Volume ±5% · Right-click menu with live track info, Play/Pause, Show/Hide Window, **Lyrics Overlay** toggle, Save Current Song, Copy Track Name, Favorites submenu, Sleep Timer (`Off`, `15m`, `30m`, `60m`), and Quit.
- **MPRIS2 (`zbus`):** Native Plasma Media Player integration, lock screen controls, hardware media keys, and KDE Connect sync.
- **In-Process `mpv` `lavfi` Spectrum Visualizer:** 16-band Linkwitz-Riley crossover (`acrossover` → `join` → `astats` → `ametadata` over a private FIFO) interpolated to a 24-bar spectrum — runs completely inside `mpv` with zero PipeWire/PulseAudio monitor capture streams (never triggers KDE's microphone tray indicator on Play/Pause), and automatically pauses when hidden in the tray.
- **Kanagawa Dragon & Crimson Accents:** Defaults to **Vibes Crimson** (Kanagawa Dragon charcoal with warm crimson accents matching the app icon), alongside **KDE Plasma** (`~/.config/kdeglobals`), `Kanagawa Dragon`, `KDE Breeze Dark`, and `Catppuccin Mocha` presets.

---

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Space` | Play / Pause |
| `L` | Toggle **Synced Lyrics** drawer |
| `M` | Mute / Unmute |
| `1`–`9` | Tune favorite preset 1–9 |
| `Left` / `Right` | Seek `-5s` / `+5s` |
| `Ctrl+S` | Save current song to Music |
| `Ctrl+M` | Toggle Mini Deck compact mode |
| `Ctrl+Q` | Quit completely |

*(Hovering over the deck's volume slider lets you hold-and-drag or scroll the mouse wheel in 5% increments.)*

---

## Requirements

- **Rust toolchain** (`rustc` + `cargo`, Edition 2024) and Linux GPUI dependencies (Vulkan, Wayland/X11, Fontconfig)
- **Runtime tools:** `mpv`, `yt-dlp`, `ffmpeg`, `mkfifo` (`coreutils`)
- **Optional:**
  - `wl-copy` (`wl-clipboard`) for **Copy Track Name** in the tray menu
  - `xdg-open` for opening music folders
  - Wayland compositor implementing `zwlr_layer_shell_v1` (e.g. KDE Plasma KWin, Hyprland, Sway) for the click-through **Lyrics Overlay**

---

## Build & Install

```bash
git clone https://github.com/vovakaravov/Vibes.git
cd Vibes
cargo build --release

# Put `vibes` on your PATH
mkdir -p ~/.local/bin
ln -sf "$(pwd)/target/release/vibes" ~/.local/bin/vibes
```

### Optional: Linux `.desktop` Entry (KDE / Wayland)

Vibes sets its Wayland `app_id` to `vibes` so desktop environments match the taskbar icon cleanly:

```bash
mkdir -p ~/.local/share/icons/hicolor/scalable/apps ~/.local/share/applications
cp assets/vibes.svg ~/.local/share/icons/hicolor/scalable/apps/vibes.svg

cat > ~/.local/share/applications/vibes.desktop <<EOF
[Desktop Entry]
Type=Application
Name=Vibes
GenericName=Internet Radio & Audio Player
Comment=Native GPUI Internet Radio & Audio-Only YouTube Deck
Exec=$HOME/.local/bin/vibes
Icon=vibes
Terminal=false
Categories=AudioVideo;Audio;Player;
StartupWMClass=vibes
Keywords=radio;music;youtube;audio;mpv;lyrics;
EOF
```

---

## License

[MIT](LICENSE)
