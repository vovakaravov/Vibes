use crate::albums::{
    AlbumDetails, AlbumDownloadJob, AlbumDownloadStatus, AlbumSummary, AlbumTrack,
    fetch_album_details, resolve_album_dir, run_download_album, search_albums,
};
use crate::artwork::{
    ArtworkFetcher, COVER_MAX_DIM, ICON_MAX_DIM, resolve_local_album_art, trim_allocator,
    youtube_thumb_url,
};
use crate::config::{
    AlbumFolderMode, PlayTarget, SaveAudioFormat, SaveFolderMode, SongLogEntry, VibesConfig,
    VibesState,
};
use crate::downloader::{
    DownloadJob, DownloadStatus, ParsedTrack, parse_identifiable_track, run_save_song,
};
use crate::instance::InstanceGuard;
use crate::local_library::{
    LocalAlbum, LocalLibrarySnapshot, LocalTrack, find_local_album_dir, find_local_track_in_dir,
    scan_local_library,
};
use crate::lyrics::{
    LyricsBackfillProgress, LyricsKind, LyricsOrigin, LyricsQuery, LyricsState,
    active_synced_line_index, embed_lyrics_in_file, fetch_lrclib_lyrics, lrclib_to_document,
    new_http_agent, parse_lyrics_text, read_embedded_lyrics, run_library_lyrics_backfill,
};
use crate::lyrics_overlay::{LyricsOverlayController, OverlayFrame};
use crate::mpris::{MprisAction, MprisController, MprisSnapshot};
use crate::mpv::{MpvController, MpvEvent};
use crate::radio::{
    RadioBrowserClient, RadioStation, load_cached_stations, save_cached_stations,
};
use crate::theme::{Palette, ThemePreset, apply_palette, load_palette};
use crate::tray::{TrayAction, TrayController, TrayPlaybackState, TraySnapshot};
use crate::visualizer::VisualizerController;
use crate::youtube::{YoutubeFilter, YoutubeItem, format_duration, search_youtube};
use chrono::Local;
use gpui_kit::{App, ClipboardItem, Context};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavSection {
    #[default]
    Radio,
    Youtube,
    Albums,
    Library,
    Favorites,
    SongLog,
    Queue,
    Downloads,
    Settings,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueueItem {
    Youtube(YoutubeItem),
    Local(LocalTrack),
}

impl QueueItem {
    pub fn title(&self) -> &str {
        match self {
            Self::Youtube(y) => &y.title,
            Self::Local(t) => &t.title,
        }
    }

    pub fn subtitle(&self) -> String {
        match self {
            Self::Youtube(y) => y.channel.clone(),
            Self::Local(t) => {
                if t.album.is_empty() {
                    t.artist.clone()
                } else {
                    format!("{} · {}", t.artist, t.album)
                }
            }
        }
    }

    pub fn duration_secs(&self) -> Option<f64> {
        match self {
            Self::Youtube(y) => y.duration_secs,
            Self::Local(t) => t.duration_secs,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCmd {
    Show,
    Hide,
    Toggle,
    ApplyCompact(bool),
    Quit,
}

enum CoreBgEvent {
    RadioFetched {
        gen_id: u64,
        append_custom: bool,
        res: Result<Vec<RadioStation>, String>,
    },
    RandomStationPicked(Result<RadioStation, String>),
    YoutubeSearched {
        gen_id: u64,
        query: String,
        res: Result<Vec<YoutubeItem>, String>,
    },
    AlbumsSearched {
        gen_id: u64,
        res: Result<Vec<AlbumSummary>, String>,
    },
    AlbumDetailsLoaded {
        gen_id: u64,
        auto_download: bool,
        res: Result<AlbumDetails, String>,
    },
    AlbumDownloadProgress {
        job_id: u64,
        status: AlbumDownloadStatus,
    },
    LocalLibraryScanned {
        gen_id: u64,
        snapshot: LocalLibrarySnapshot,
    },
    ArtworkLoaded {
        key: String,
        path: PathBuf,
    },
    SaveProgress {
        job_id: u64,
        status: DownloadStatus,
    },
    LyricsLoaded {
        gen_id: u64,
        key: String,
        state: LyricsState,
    },
    LyricsBackfillUpdated(LyricsBackfillProgress),
}

pub struct VibesCore {
    pub config: VibesConfig,
    pub state: VibesState,
    pub palette: Palette,
    pub section: NavSection,
    pub window_open: bool,
    pub compact_mode: bool,

    // Radio state
    pub stations: Vec<RadioStation>,
    pub selected_genre: String,
    pub radio_query: String,
    pub radio_loading: bool,
    pub radio_error: Option<String>,
    radio_fetch_gen: u64,

    // YouTube state
    pub yt_results: Vec<YoutubeItem>,
    pub yt_filter: YoutubeFilter,
    pub yt_query: String,
    pub yt_loading: bool,
    pub yt_error: Option<String>,
    yt_fetch_gen: u64,

    // Album search & download state
    pub album_query: String,
    pub album_results: Vec<AlbumSummary>,
    pub album_loading: bool,
    pub album_error: Option<String>,
    album_fetch_gen: u64,
    pub selected_album: Option<AlbumDetails>,
    pub album_detail_loading: bool,
    album_detail_gen: u64,
    pub album_downloads: Vec<AlbumDownloadJob>,

    // Local offline library state
    pub local_library: LocalLibrarySnapshot,
    pub local_library_loading: bool,
    local_scan_gen: u64,
    pub local_filter_query: String,
    pub local_tab_singles: bool,
    pub selected_local_album: Option<PathBuf>,

    // Playback state
    pub current_target: Option<PlayTarget>,
    pub playing: bool,
    pub paused: bool,
    pub buffering: bool,
    pub icy_title: Option<String>,
    pub icy_name: Option<String>,
    pub media_title: Option<String>,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub codec: Option<String>,
    pub bitrate_kbps: Option<u64>,
    pub sample_rate: Option<u32>,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub queue: Vec<QueueItem>,
    reconnect_attempts: u8,
    user_stopped: bool,
    last_resume_save_secs: f64,

    // Downloads & toasts
    pub downloads: Vec<DownloadJob>,
    next_job_id: u64,
    pub toast: Option<String>,
    toast_gen: u64,

    // Sleep timer
    pub sleep_deadline: Option<Instant>,
    last_sleep_mins_synced: Option<u64>,

    // Subsystems & caches
    pub art_cache: HashMap<String, PathBuf>,
    art_fetcher: ArtworkFetcher,
    mpv: Option<MpvController>,
    pub mpv_error: Option<String>,
    tray: TrayController,
    mpris: MprisController,
    pub visualizer: VisualizerController,
    radio_client: RadioBrowserClient,
    window_tx: async_channel::Sender<WindowCmd>,
    bg_tx: async_channel::Sender<CoreBgEvent>,
    _instance_guard: Option<InstanceGuard>,

    // Lyrics & backfill
    pub lyrics_open: bool,
    pub lyrics_state: LyricsState,
    pub lyrics_track_key: Option<String>,
    lyrics_fetch_gen: u64,
    lyrics_cache: HashMap<String, LyricsState>,
    pub lyrics_backfill: LyricsBackfillProgress,
    lyrics_backfill_cancel: Option<Arc<AtomicBool>>,
    lyrics_overlay: LyricsOverlayController,
}

impl VibesCore {
    pub fn new(
        window_tx: async_channel::Sender<WindowCmd>,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = VibesConfig::load();
        let state = VibesState::load();
        let palette = load_palette(config.theme);
        apply_palette(cx, &palette);

        let (mpv_tx, mpv_rx) = async_channel::unbounded();
        let (tray_tx, tray_rx) = async_channel::unbounded();
        let (mpris_tx, mpris_rx) = async_channel::unbounded();
        let (show_tx, show_rx) = async_channel::unbounded();
        let (bg_tx, bg_rx) = async_channel::unbounded();

        let (mpv, mpv_error) = match MpvController::spawn(config.volume, mpv_tx) {
            Ok(c) => (Some(c), None),
            Err(err) => (None, Some(format!("{err}"))),
        };

        let tray = TrayController::spawn(tray_tx);
        let mpris = MprisController::spawn(mpris_tx);
        let instance_guard = InstanceGuard::start(show_tx);
        let bg_art = bg_tx.clone();
        let art_fetcher = ArtworkFetcher::spawn(move |key, path| {
            let _ = bg_art.send_blocking(CoreBgEvent::ArtworkLoaded { key, path });
        });

        // Spawn channel listeners cleanly bound to Entity<VibesCore>
        cx.spawn(async move |weak, cx| {
            while let Ok(ev) = mpv_rx.recv().await {
                let ok = weak
                    .update(cx, |this, cx| this.handle_mpv_event(ev, cx))
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |weak, cx| {
            while let Ok(act) = tray_rx.recv().await {
                let ok = weak
                    .update(cx, |this, cx| this.handle_tray_action(act, cx))
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |weak, cx| {
            while let Ok(act) = mpris_rx.recv().await {
                let ok = weak
                    .update(cx, |this, cx| this.handle_mpris_action(act, cx))
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |weak, cx| {
            while show_rx.recv().await.is_ok() {
                let ok = weak
                    .update(cx, |this, _| {
                        let _ = this.window_tx.try_send(WindowCmd::Show);
                    })
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |weak, cx| {
            while let Ok(ev) = bg_rx.recv().await {
                let ok = weak
                    .update(cx, |this, cx| this.handle_bg_event(ev, cx))
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();

        // 1Hz background timer for sleep timer & periodic podcast resume save
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep = weak
                    .update(cx, |this, cx| this.tick_second(cx))
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();

        let cached = load_cached_stations();
        let initial_vol = config.volume;
        let current_target = state.last_played.clone();

        let mut core = Self {
            config,
            state,
            palette,
            section: NavSection::Radio,
            window_open: true,
            compact_mode: false,

            stations: cached,
            selected_genre: "Top".to_string(),
            radio_query: String::new(),
            radio_loading: false,
            radio_error: None,
            radio_fetch_gen: 0,

            yt_results: Vec::new(),
            yt_filter: YoutubeFilter::All,
            yt_query: String::new(),
            yt_loading: false,
            yt_error: None,
            yt_fetch_gen: 0,

            album_query: String::new(),
            album_results: Vec::new(),
            album_loading: false,
            album_error: None,
            album_fetch_gen: 0,
            selected_album: None,
            album_detail_loading: false,
            album_detail_gen: 0,
            album_downloads: Vec::new(),

            local_library: LocalLibrarySnapshot::default(),
            local_library_loading: false,
            local_scan_gen: 0,
            local_filter_query: String::new(),
            local_tab_singles: false,
            selected_local_album: None,

            current_target,
            playing: false,
            paused: false,
            buffering: false,
            icy_title: None,
            icy_name: None,
            media_title: None,
            position_secs: 0.0,
            duration_secs: None,
            codec: None,
            bitrate_kbps: None,
            sample_rate: None,
            volume: initial_vol,
            muted: false,
            speed: 1.0,
            queue: Vec::new(),
            reconnect_attempts: 0,
            user_stopped: true,
            last_resume_save_secs: 0.0,

            downloads: Vec::new(),
            next_job_id: 1,
            toast: None,
            toast_gen: 0,

            sleep_deadline: None,
            last_sleep_mins_synced: None,

            art_cache: HashMap::new(),
            art_fetcher,
            mpv,
            mpv_error,
            tray,
            mpris,
            visualizer: VisualizerController::new(),
            radio_client: RadioBrowserClient::new(),
            window_tx,
            bg_tx,
            _instance_guard: instance_guard,

            lyrics_open: false,
            lyrics_state: LyricsState::Empty,
            lyrics_track_key: None,
            lyrics_fetch_gen: 0,
            lyrics_cache: HashMap::new(),
            lyrics_backfill: LyricsBackfillProgress::default(),
            lyrics_backfill_cancel: None,
            lyrics_overlay: LyricsOverlayController::new(),
        };

        core.sync_integrations();
        core.refresh_visualizer_state();
        core.sync_lyrics_overlay();
        core.prefetch_startup_art();
        core.refresh_radio_stations(cx);
        core.refresh_local_library(cx);
        core
    }

    pub fn art_path_for_url(&self, url: &str) -> Option<&PathBuf> {
        let key = url.trim();
        if key.is_empty() {
            None
        } else {
            self.art_cache.get(key)
        }
    }

    pub fn art_path_for_youtube(&self, video_id: &str) -> Option<&PathBuf> {
        let url = youtube_thumb_url(video_id)?;
        self.art_cache.get(&url)
    }

    pub fn prefetch_art_url(&self, url: &str, max_dim: u32) {
        self.art_fetcher.prefetch(url, max_dim);
    }

    fn prefetch_stations_art(&self, stations: &[RadioStation]) {
        for s in stations {
            self.prefetch_art_url(&s.favicon, ICON_MAX_DIM);
        }
    }

    fn prefetch_target_art(&self, target: &PlayTarget) {
        match target {
            PlayTarget::Radio(s) => self.prefetch_art_url(&s.favicon, ICON_MAX_DIM),
            PlayTarget::Youtube(y) => {
                if let Some(u) = youtube_thumb_url(&y.id) {
                    self.prefetch_art_url(&u, COVER_MAX_DIM);
                }
            }
            PlayTarget::Local(_) => {}
        }
    }

    fn prefetch_startup_art(&self) {
        self.prefetch_stations_art(&self.stations);
        self.prefetch_stations_art(&self.state.favorites);
        if let Some(ref target) = self.current_target {
            self.prefetch_target_art(target);
        }
    }

    pub fn set_window_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.window_open = open;
        self.refresh_visualizer_state();
        self.sync_integrations();
        if open {
            cx.notify();
        } else {
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                trim_allocator();
            })
            .detach();
        }
    }

    pub fn toggle_compact_mode(&mut self, cx: &mut Context<Self>) {
        self.compact_mode = !self.compact_mode;
        let _ = self
            .window_tx
            .try_send(WindowCmd::ApplyCompact(self.compact_mode));
        cx.notify();
    }

    pub fn hide_to_tray(&mut self) {
        let _ = self.window_tx.try_send(WindowCmd::Hide);
    }

    pub fn show_toast(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        let txt = msg.into();
        self.toast = Some(txt);
        self.toast_gen = self.toast_gen.wrapping_add(1);
        let gen_id = self.toast_gen;
        cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            let _ = weak.update(cx, |this, cx| {
                if this.toast_gen == gen_id {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn persist_config(&self) {
        let _ = self.config.save();
    }

    fn persist_state(&self) {
        let _ = self.state.save();
    }

    fn refresh_visualizer_state(&mut self) {
        let want_filter = self.window_open && self.config.visualizer_enabled;
        let want_anim = want_filter && self.playing && !self.paused;
        self.visualizer
            .sync(want_filter, want_anim, self.mpv.as_ref());
    }

    pub fn current_track_text(&self) -> Option<String> {
        match &self.current_target {
            Some(PlayTarget::Radio(s)) => self
                .icy_title
                .as_deref()
                .or(self.media_title.as_deref())
                .filter(|t| {
                    let clean = t.trim();
                    !clean.is_empty() && !clean.eq_ignore_ascii_case(s.name.trim())
                })
                .map(|t| t.trim().to_string()),
            Some(PlayTarget::Youtube(y)) => Some(y.title.clone()),
            Some(PlayTarget::Local(t)) => Some(t.title.clone()),
            None => None,
        }
    }

    pub fn identifiable_current_track(&self) -> Option<(ParsedTrack, Option<String>, String)> {
        match &self.current_target {
            Some(PlayTarget::Radio(st)) => {
                let raw = self.current_track_text()?;
                let parsed = parse_identifiable_track(&raw, Some(&st.name))?;
                Some((parsed, None, st.name.clone()))
            }
            Some(PlayTarget::Youtube(yt)) => {
                let parsed = parse_identifiable_track(&yt.title, Some(&yt.channel)).unwrap_or_else(
                    || ParsedTrack {
                        artist: Some(yt.channel.clone()),
                        title: yt.title.clone(),
                        display: format!("{} - {}", yt.channel, yt.title),
                    },
                );
                Some((parsed, Some(yt.url.clone()), yt.channel.clone()))
            }
            Some(PlayTarget::Local(_)) | None => None,
        }
    }

    pub fn can_save_current_song(&self) -> bool {
        self.identifiable_current_track().is_some()
    }

    pub fn active_saving_count(&self) -> usize {
        let songs = self
            .downloads
            .iter()
            .filter(|j| {
                matches!(
                    j.status,
                    DownloadStatus::Searching | DownloadStatus::Downloading(_)
                )
            })
            .count();
        let albums = self
            .album_downloads
            .iter()
            .filter(|j| {
                matches!(
                    j.status,
                    AlbumDownloadStatus::Queued | AlbumDownloadStatus::Downloading { .. }
                )
            })
            .count();
        songs + albums
    }

    pub fn sleep_remaining_mins(&self) -> Option<u64> {
        let dl = self.sleep_deadline?;
        let now = Instant::now();
        if dl <= now {
            None
        } else {
            Some(((dl - now).as_secs() + 59) / 60)
        }
    }

    fn sync_integrations(&mut self) {
        let state = if self.buffering && self.playing && !self.paused {
            TrayPlaybackState::Buffering
        } else if self.playing && !self.paused {
            TrayPlaybackState::Playing
        } else if self.playing && self.paused {
            TrayPlaybackState::Paused
        } else {
            TrayPlaybackState::Idle
        };

        let (title, subtitle, artist) = match &self.current_target {
            Some(PlayTarget::Radio(s)) => {
                let sub = self
                    .current_track_text()
                    .unwrap_or_else(|| s.display_codec());
                let parsed_artist = self
                    .current_track_text()
                    .and_then(|t| parse_identifiable_track(&t, Some(&s.name)))
                    .and_then(|p| p.artist)
                    .unwrap_or_else(|| s.name.clone());
                (s.name.clone(), sub, parsed_artist)
            }
            Some(PlayTarget::Youtube(y)) => (y.title.clone(), y.channel.clone(), y.channel.clone()),
            Some(PlayTarget::Local(t)) => {
                let sub = if t.album.is_empty() {
                    t.artist.clone()
                } else {
                    format!("{} · {}", t.artist, t.album)
                };
                (t.title.clone(), sub, t.artist.clone())
            }
            None => ("Vibes".to_string(), "Idle · Select a station or search".to_string(), String::new()),
        };

        let favorites: Vec<(String, String)> = self
            .state
            .favorites
            .iter()
            .map(|f| (f.stationuuid.clone(), f.name.clone()))
            .collect();

        let sleep_mins = self.sleep_remaining_mins();
        self.last_sleep_mins_synced = sleep_mins;

        self.tray.sync(TraySnapshot {
            state,
            volume: self.volume,
            muted: self.muted,
            title: title.clone(),
            subtitle: subtitle.clone(),
            window_open: self.window_open,
            can_save_song: self.can_save_current_song(),
            can_copy_track: self.current_track_text().is_some(),
            favorites,
            sleep_remaining_mins: sleep_mins,
            saving_active: self.active_saving_count() > 0,
            lyrics_overlay_open: self.config.lyrics_overlay_enabled,
        });

        let mpris_status = match state {
            TrayPlaybackState::Playing | TrayPlaybackState::Buffering => "Playing",
            TrayPlaybackState::Paused => "Paused",
            TrayPlaybackState::Idle => "Stopped",
        };
        self.mpris.sync(MprisSnapshot {
            playback_status: mpris_status.to_string(),
            title: if subtitle.is_empty()
                || matches!(
                    self.current_target,
                    Some(PlayTarget::Youtube(_) | PlayTarget::Local(_))
                )
            {
                title
            } else {
                subtitle
            },
            artist,
            album: match &self.current_target {
                Some(PlayTarget::Radio(s)) => s.name.clone(),
                Some(PlayTarget::Youtube(_)) => "YouTube Audio".to_string(),
                Some(PlayTarget::Local(t)) => {
                    if t.album.is_empty() {
                        "Local Library".to_string()
                    } else {
                        t.album.clone()
                    }
                }
                None => "Vibes".to_string(),
            },
            length_us: self.duration_secs.map(|d| (d * 1_000_000.0) as i64),
            position_us: (self.position_secs * 1_000_000.0) as i64,
            volume: if self.muted { 0.0 } else { self.volume / 100.0 },
        });
    }

    // ---- Radio Browser ----

    pub fn set_genre(&mut self, genre: &str, cx: &mut Context<Self>) {
        self.selected_genre = genre.to_string();
        self.refresh_radio_stations(cx);
    }

    pub fn set_radio_query(&mut self, q: String, cx: &mut Context<Self>) {
        self.radio_query = q;
        self.refresh_radio_stations(cx);
    }

    pub fn refresh_radio_stations(&mut self, cx: &mut Context<Self>) {
        self.radio_loading = true;
        self.radio_error = None;
        self.radio_fetch_gen = self.radio_fetch_gen.wrapping_add(1);
        let gen_id = self.radio_fetch_gen;
        let query = self.radio_query.clone();
        let genre = self.selected_genre.clone();
        let cc = self.config.country_code.clone();
        let blend = self.config.local_blending;
        let client = self.radio_client.clone();
        let tx = self.bg_tx.clone();
        let append_custom = query.trim().is_empty() && genre == "Top";

        std::thread::spawn(move || {
            let res = client
                .fetch_stations(&query, &genre, &cc, blend, 70)
                .map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::RadioFetched {
                gen_id,
                append_custom,
                res,
            });
        });
        cx.notify();
    }

    pub fn spin_the_dial(&mut self, cx: &mut Context<Self>) {
        let genre = self.selected_genre.clone();
        let cc = self.config.country_code.clone();
        let exclude = match &self.current_target {
            Some(PlayTarget::Radio(s)) => Some(s.stationuuid.clone()),
            _ => None,
        };
        let client = self.radio_client.clone();
        let tx = self.bg_tx.clone();
        self.show_toast("Spinning the dial…", cx);
        std::thread::spawn(move || {
            let res = client
                .fetch_random_station(&genre, &cc, exclude.as_deref())
                .map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::RandomStationPicked(res));
        });
    }

    pub fn add_custom_station(&mut self, name: String, url: String, cx: &mut Context<Self>) {
        let u = url.trim();
        if !u.starts_with("http://") && !u.starts_with("https://") {
            self.show_toast("Stream URL must start with http:// or https://", cx);
            return;
        }
        let st = RadioStation::custom(name, u);
        if !self
            .state
            .custom_stations
            .iter()
            .any(|s| s.stream_url().eq_ignore_ascii_case(st.stream_url()))
        {
            self.state.custom_stations.insert(0, st.clone());
        }
        self.add_favorite(st.clone(), cx);
        self.play_station(st, cx);
    }

    // ---- YouTube Search & Queue ----

    pub fn search_youtube_query(&mut self, query: String, cx: &mut Context<Self>) {
        let q = query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.yt_query = q.clone();
        self.yt_loading = true;
        self.yt_error = None;
        self.yt_fetch_gen = self.yt_fetch_gen.wrapping_add(1);
        let gen_id = self.yt_fetch_gen;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            let res = search_youtube(&q, 18).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::YoutubeSearched {
                gen_id,
                query: q,
                res,
            });
        });
        cx.notify();
    }

    pub fn queue_youtube_item(&mut self, item: YoutubeItem, cx: &mut Context<Self>) {
        if let Some(u) = youtube_thumb_url(&item.id) {
            self.prefetch_art_url(&u, COVER_MAX_DIM);
        }
        self.queue.push(QueueItem::Youtube(item.clone()));
        self.show_toast(format!("Queued: {}", item.title), cx);
        if !self.playing && self.current_target.is_none() {
            self.play_next_in_queue(cx);
        }
        cx.notify();
    }

    pub fn queue_local_track(&mut self, track: LocalTrack, cx: &mut Context<Self>) {
        self.queue.push(QueueItem::Local(track.clone()));
        self.show_toast(format!("Queued offline: {}", track.title), cx);
        if !self.playing && self.current_target.is_none() {
            self.play_next_in_queue(cx);
        }
        cx.notify();
    }

    pub fn play_queue_item(&mut self, item: QueueItem, cx: &mut Context<Self>) {
        match item {
            QueueItem::Youtube(y) => self.play_youtube(y, cx),
            QueueItem::Local(t) => self.play_local_track(t, cx),
        }
    }

    pub fn remove_queue_index(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.queue.len() {
            self.queue.remove(idx);
            cx.notify();
        }
    }

    pub fn clear_queue(&mut self, cx: &mut Context<Self>) {
        self.queue.clear();
        cx.notify();
    }

    pub fn play_next_in_queue(&mut self, cx: &mut Context<Self>) {
        if !self.queue.is_empty() {
            let next = self.queue.remove(0);
            self.play_queue_item(next, cx);
        } else {
            self.cycle_favorite(1, cx);
        }
    }

    pub fn play_previous_action(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.current_target,
            Some(PlayTarget::Youtube(_) | PlayTarget::Local(_))
        ) && self.position_secs > 3.0
        {
            self.seek_absolute(0.0, cx);
        } else {
            self.cycle_favorite(-1, cx);
        }
    }

    // ---- Playback Control ----

    fn save_current_yt_resume(&mut self) {
        if let Some(PlayTarget::Youtube(yt)) = &self.current_target
            && let Some(dur) = self.duration_secs
            && dur >= 600.0
            && !yt.id.is_empty()
        {
            if self.position_secs >= dur - 25.0 || self.position_secs < 10.0 {
                self.state.resume_positions.remove(&yt.id);
            } else {
                self.state
                    .resume_positions
                    .insert(yt.id.clone(), self.position_secs);
            }
            self.persist_state();
        }
    }

    pub fn play_station(&mut self, station: RadioStation, cx: &mut Context<Self>) {
        self.save_current_yt_resume();
        let url = station.stream_url().to_string();
        if url.is_empty() {
            self.show_toast("Station has an empty stream URL", cx);
            return;
        }
        self.current_target = Some(PlayTarget::Radio(station.clone()));
        self.state.last_played = self.current_target.clone();
        self.persist_state();
        self.prefetch_art_url(&station.favicon, ICON_MAX_DIM);

        self.icy_title = None;
        self.icy_name = None;
        self.media_title = Some(station.name.clone());
        self.position_secs = 0.0;
        self.duration_secs = None;
        self.codec = (!station.codec.is_empty()).then(|| station.codec.to_uppercase());
        self.bitrate_kbps = (station.bitrate > 0).then_some(station.bitrate as u64);
        self.sample_rate = None;
        self.speed = 1.0;
        self.playing = true;
        self.paused = false;
        self.buffering = true;
        self.user_stopped = false;
        self.reconnect_attempts = 0;

        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_speed(1.0);
            if let Err(err) = mpv.load_url(&url, None) {
                self.show_toast(format!("Playback error: {err}"), cx);
            }
        }

        let client = self.radio_client.clone();
        let uuid = station.stationuuid;
        std::thread::spawn(move || client.register_click(&uuid));

        self.refresh_visualizer_state();
        if self.wants_lyrics() {
            self.refresh_lyrics_for_current(cx);
        } else {
            self.lyrics_track_key = None;
            self.lyrics_state = LyricsState::Empty;
        }
        self.sync_lyrics_overlay();
        self.sync_integrations();
        cx.notify();
    }

    pub fn play_youtube(&mut self, item: YoutubeItem, cx: &mut Context<Self>) {
        self.save_current_yt_resume();
        let resume = self.state.resume_positions.get(&item.id).copied();
        self.current_target = Some(PlayTarget::Youtube(item.clone()));
        self.state.last_played = self.current_target.clone();
        self.persist_state();
        if let Some(u) = youtube_thumb_url(&item.id) {
            self.prefetch_art_url(&u, COVER_MAX_DIM);
        }

        self.icy_title = None;
        self.icy_name = None;
        self.media_title = Some(item.title.clone());
        self.position_secs = resume.unwrap_or(0.0);
        self.duration_secs = item.duration_secs;
        self.codec = Some("OPUS".to_string());
        self.bitrate_kbps = None;
        self.sample_rate = None;
        self.playing = true;
        self.paused = false;
        self.buffering = true;
        self.user_stopped = false;
        self.reconnect_attempts = 0;
        self.last_resume_save_secs = self.position_secs;

        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_speed(self.speed);
            if let Err(err) = mpv.load_url(&item.url, resume) {
                self.show_toast(format!("YouTube error: {err}"), cx);
            }
        }
        if let Some(r) = resume {
            self.show_toast(format!("Resumed at {}", format_duration(r)), cx);
        }

        self.refresh_visualizer_state();
        if self.wants_lyrics() {
            self.refresh_lyrics_for_current(cx);
        } else {
            self.lyrics_track_key = None;
            self.lyrics_state = LyricsState::Empty;
        }
        self.sync_lyrics_overlay();
        self.sync_integrations();
        cx.notify();
    }

    pub fn play_local_track(&mut self, track: LocalTrack, cx: &mut Context<Self>) {
        if !track.path.exists() {
            self.show_toast("File missing on disk — refreshing library", cx);
            self.refresh_local_library(cx);
            return;
        }
        self.save_current_yt_resume();
        self.current_target = Some(PlayTarget::Local(track.clone()));
        self.state.last_played = self.current_target.clone();
        self.persist_state();

        self.icy_title = None;
        self.icy_name = None;
        self.media_title = Some(track.title.clone());
        self.position_secs = 0.0;
        self.duration_secs = track.duration_secs;
        self.codec = (!track.format.is_empty()).then(|| track.format.clone());
        self.bitrate_kbps = None;
        self.sample_rate = None;
        self.playing = true;
        self.paused = false;
        self.buffering = false;
        self.user_stopped = false;
        self.reconnect_attempts = 0;

        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_speed(self.speed);
            if let Err(err) = mpv.load_url(&track.path.to_string_lossy(), None) {
                self.show_toast(format!("Local playback error: {err}"), cx);
            }
        }

        self.refresh_visualizer_state();
        if self.wants_lyrics() {
            self.refresh_lyrics_for_current(cx);
        } else {
            self.lyrics_track_key = None;
            self.lyrics_state = LyricsState::Empty;
        }
        self.sync_lyrics_overlay();
        self.sync_integrations();
        cx.notify();
    }

    fn play_local_list_from(
        &mut self,
        tracks: &[LocalTrack],
        start_idx: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = tracks.get(start_idx).cloned() else {
            return;
        };
        self.queue = tracks[start_idx + 1..]
            .iter()
            .cloned()
            .map(QueueItem::Local)
            .collect();
        self.play_local_track(first, cx);
    }

    pub fn play_local_album_from(
        &mut self,
        album: &LocalAlbum,
        start_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.play_local_list_from(&album.tracks, start_idx, cx);
    }

    pub fn play_local_singles_from(
        &mut self,
        tracks: &[LocalTrack],
        start_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.play_local_list_from(tracks, start_idx, cx);
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if !self.playing {
            match self.current_target.clone() {
                Some(PlayTarget::Radio(s)) => self.play_station(s, cx),
                Some(PlayTarget::Youtube(y)) => self.play_youtube(y, cx),
                Some(PlayTarget::Local(t)) => self.play_local_track(t, cx),
                None => {
                    if let Some(fav) = self.state.favorites.first().cloned() {
                        self.play_station(fav, cx);
                    } else if let Some(first) = self.stations.first().cloned() {
                        self.play_station(first, cx);
                    } else {
                        self.spin_the_dial(cx);
                    }
                }
            }
            return;
        }
        let next_paused = !self.paused;
        self.set_paused(next_paused, cx);
    }

    fn is_live_stream(&self) -> bool {
        match &self.current_target {
            Some(PlayTarget::Radio(_)) => true,
            Some(PlayTarget::Youtube(y)) => y.is_live,
            Some(PlayTarget::Local(_)) | None => false,
        }
    }

    pub fn set_paused(&mut self, pause: bool, cx: &mut Context<Self>) {
        if !self.playing {
            if !pause {
                self.toggle_play(cx);
            }
            return;
        }
        if self.is_live_stream() {
            if pause {
                // Stop live stream so no bandwidth is wasted, while keeping stream paused-in-place.
                self.paused = true;
                self.buffering = false;
                self.user_stopped = true;
                if let Some(ref mpv) = self.mpv {
                    let _ = mpv.stop();
                }
            } else {
                // Reload the live stream fresh to rejoin at the latest live edge.
                let url = match &self.current_target {
                    Some(PlayTarget::Radio(s)) => s.stream_url().to_string(),
                    Some(PlayTarget::Youtube(y)) => y.url.clone(),
                    Some(PlayTarget::Local(_)) | None => return,
                };
                self.paused = false;
                self.buffering = true;
                self.user_stopped = false;
                self.reconnect_attempts = 0;
                if let Some(ref mpv) = self.mpv {
                    let _ = mpv.load_url(&url, None);
                }
            }
        } else {
            self.paused = pause;
            if let Some(ref mpv) = self.mpv {
                let _ = mpv.set_pause(pause);
            }
            if pause {
                self.save_current_yt_resume();
            }
        }
        self.refresh_visualizer_state();
        self.sync_lyrics_overlay();
        self.sync_integrations();
        cx.notify();
    }

    pub fn stop_playback(&mut self, cx: &mut Context<Self>) {
        self.save_current_yt_resume();
        self.user_stopped = true;
        self.playing = false;
        self.paused = false;
        self.buffering = false;
        self.position_secs = 0.0;
        self.lyrics_fetch_gen += 1;
        self.lyrics_track_key = None;
        self.lyrics_state = LyricsState::Empty;
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.stop();
        }
        self.refresh_visualizer_state();
        self.sync_lyrics_overlay();
        self.sync_integrations();
        cx.notify();
    }

    pub fn reconnect_stream(&mut self, cx: &mut Context<Self>) {
        match self.current_target.clone() {
            Some(PlayTarget::Radio(s)) => self.play_station(s, cx),
            Some(PlayTarget::Youtube(y)) => self.play_youtube(y, cx),
            Some(PlayTarget::Local(t)) => self.play_local_track(t, cx),
            None => {}
        }
    }

    pub fn seek_absolute(&mut self, secs: f64, cx: &mut Context<Self>) {
        if !matches!(
            self.current_target,
            Some(PlayTarget::Youtube(_) | PlayTarget::Local(_))
        ) {
            return;
        }
        let target = if let Some(dur) = self.duration_secs {
            secs.clamp(0.0, dur)
        } else {
            secs.max(0.0)
        };
        self.position_secs = target;
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.seek_absolute(target);
        }
        self.sync_lyrics_overlay();
        cx.notify();
    }

    pub fn seek_relative(&mut self, delta: f64, cx: &mut Context<Self>) {
        if !matches!(
            self.current_target,
            Some(PlayTarget::Youtube(_) | PlayTarget::Local(_))
        ) {
            return;
        }
        self.position_secs = (self.position_secs + delta).max(0.0);
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.seek_relative(delta);
        }
        self.sync_lyrics_overlay();
        cx.notify();
    }

    pub fn set_volume_live(&mut self, vol: f64, persist: bool, cx: &mut Context<Self>) {
        let clamped = vol.clamp(0.0, 100.0);
        let prev_bucket = (self.volume / 5.0).round() as i32;
        let next_bucket = (clamped / 5.0).round() as i32;
        let mut mute_changed = false;

        if clamped > 0.5 && self.muted {
            self.muted = false;
            mute_changed = true;
            if let Some(ref mpv) = self.mpv {
                let _ = mpv.set_mute(false);
            }
        }

        self.volume = clamped;
        self.config.volume = clamped;
        if persist {
            self.persist_config();
        }
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_volume(clamped);
        }
        if persist || mute_changed || prev_bucket != next_bucket {
            self.sync_integrations();
        }
        cx.notify();
    }

    pub fn set_volume(&mut self, vol: f64, cx: &mut Context<Self>) {
        self.set_volume_live(vol, true, cx);
    }

    pub fn step_volume(&mut self, up: bool, cx: &mut Context<Self>) {
        let next = step_volume_5(self.volume, up);
        self.set_volume(next, cx);
    }

    pub fn persist_config_now(&self) {
        self.persist_config();
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_mute(self.muted);
        }
        self.sync_integrations();
        cx.notify();
    }

    pub fn cycle_speed(&mut self, cx: &mut Context<Self>) {
        const SPEEDS: &[f64] = &[1.0, 1.25, 1.5, 1.75, 2.0, 0.75];
        let idx = SPEEDS
            .iter()
            .position(|&s| (s - self.speed).abs() < 0.05)
            .unwrap_or(0);
        self.speed = SPEEDS[(idx + 1) % SPEEDS.len()];
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_speed(self.speed);
        }
        cx.notify();
    }

    // ---- Favorites & Presets ----

    pub fn is_favorite(&self, station: &RadioStation) -> bool {
        self.state.favorites.iter().any(|f| {
            (!f.stationuuid.is_empty() && f.stationuuid == station.stationuuid)
                || f.stream_url().eq_ignore_ascii_case(station.stream_url())
        })
    }

    pub fn toggle_favorite_station(&mut self, station: RadioStation, cx: &mut Context<Self>) {
        if self.is_favorite(&station) {
            self.state.favorites.retain(|f| {
                !((!f.stationuuid.is_empty() && f.stationuuid == station.stationuuid)
                    || f.stream_url().eq_ignore_ascii_case(station.stream_url()))
            });
            self.show_toast(format!("Removed {} from favorites", station.name), cx);
        } else {
            self.add_favorite(station.clone(), cx);
            return;
        }
        self.persist_state();
        self.sync_integrations();
        cx.notify();
    }

    pub fn add_favorite(&mut self, station: RadioStation, cx: &mut Context<Self>) {
        if !self.is_favorite(&station) {
            self.prefetch_art_url(&station.favicon, ICON_MAX_DIM);
            self.state.favorites.push(station.clone());
            self.persist_state();
            self.sync_integrations();
            self.show_toast(format!("Starred {}", station.name), cx);
            cx.notify();
        }
    }

    pub fn move_favorite(&mut self, idx: usize, delta: isize, cx: &mut Context<Self>) {
        let new_idx = idx as isize + delta;
        if idx < self.state.favorites.len()
            && new_idx >= 0
            && (new_idx as usize) < self.state.favorites.len()
        {
            self.state.favorites.swap(idx, new_idx as usize);
            self.persist_state();
            self.sync_integrations();
            cx.notify();
        }
    }

    pub fn play_favorite_preset(&mut self, one_based_slot: usize, cx: &mut Context<Self>) {
        if let Some(st) = self.state.favorites.get(one_based_slot.saturating_sub(1)).cloned() {
            self.play_station(st, cx);
        }
    }

    pub fn cycle_favorite(&mut self, dir: isize, cx: &mut Context<Self>) {
        if self.state.favorites.is_empty() {
            return;
        }
        let len = self.state.favorites.len() as isize;
        let cur_idx = match &self.current_target {
            Some(PlayTarget::Radio(s)) => self
                .state
                .favorites
                .iter()
                .position(|f| f.stream_url().eq_ignore_ascii_case(s.stream_url()))
                .map(|i| i as isize)
                .unwrap_or(0),
            _ => 0,
        };
        let next = ((cur_idx + dir).rem_euclid(len)) as usize;
        let st = self.state.favorites[next].clone();
        self.play_station(st, cx);
    }

    // ---- Song Log & Save Song ----

    fn record_song_log(&mut self, raw_title: &str) {
        let Some(PlayTarget::Radio(ref station)) = self.current_target else {
            return;
        };
        let Some(parsed) = parse_identifiable_track(raw_title, Some(&station.name)) else {
            return;
        };
        if let Some(last) = self.state.song_log.first()
            && last.track.eq_ignore_ascii_case(&parsed.display)
            && last.station == station.name
        {
            return;
        }
        self.state.song_log.insert(
            0,
            SongLogEntry {
                timestamp: Local::now().format("%Y-%m-%d %H:%M").to_string(),
                track: parsed.display,
                station: station.name.clone(),
                source_url: station.stream_url().to_string(),
            },
        );
        self.state.song_log.truncate(500);
        self.persist_state();
    }

    pub fn clear_song_log(&mut self, cx: &mut Context<Self>) {
        self.state.song_log.clear();
        self.persist_state();
        cx.notify();
    }

    pub fn save_current_song(&mut self, cx: &mut Context<Self>) {
        let Some((parsed, direct_url, station)) = self.identifiable_current_track() else {
            self.show_toast("No identifiable song title yet on this stream", cx);
            return;
        };
        self.enqueue_save_job(parsed, direct_url, station, cx);
    }

    pub fn save_song_from_text(&mut self, text: &str, station: &str, cx: &mut Context<Self>) {
        let Some(parsed) = parse_identifiable_track(text, Some(station)) else {
            self.show_toast("Track title looks like stream/station metadata", cx);
            return;
        };
        self.enqueue_save_job(parsed, None, station.to_string(), cx);
    }

    pub fn save_youtube_item(&mut self, item: &YoutubeItem, cx: &mut Context<Self>) {
        let parsed = parse_identifiable_track(&item.title, Some(&item.channel)).unwrap_or_else(
            || ParsedTrack {
                artist: Some(item.channel.clone()),
                title: item.title.clone(),
                display: format!("{} - {}", item.channel, item.title),
            },
        );
        self.enqueue_save_job(parsed, Some(item.url.clone()), item.channel.clone(), cx);
    }

    pub fn retry_download(&mut self, job_id: u64, cx: &mut Context<Self>) {
        let Some(job) = self.downloads.iter().find(|j| j.id == job_id).cloned() else {
            return;
        };
        self.enqueue_save_job(job.parsed, job.direct_url, job.source_station, cx);
    }

    fn enqueue_save_job(
        &mut self,
        parsed: ParsedTrack,
        direct_url: Option<String>,
        source_station: String,
        cx: &mut Context<Self>,
    ) {
        if self.downloads.iter().any(|j| {
            j.parsed.display.eq_ignore_ascii_case(&parsed.display)
                && matches!(
                    j.status,
                    DownloadStatus::Searching | DownloadStatus::Downloading(_)
                )
        }) {
            self.show_toast(format!("Already saving {}", parsed.display), cx);
            return;
        }

        let job_id = self.next_job_id;
        self.next_job_id += 1;
        let label = parsed.display.clone();
        self.downloads.insert(
            0,
            DownloadJob {
                id: job_id,
                label: label.clone(),
                source_station: source_station.clone(),
                direct_url: direct_url.clone(),
                parsed: parsed.clone(),
                status: if direct_url.is_some() {
                    DownloadStatus::Downloading(label.clone())
                } else {
                    DownloadStatus::Searching
                },
                created_at: Local::now().format("%H:%M:%S").to_string(),
            },
        );
        self.show_toast(format!("Saving {label} to Music…"), cx);
        self.sync_integrations();

        let music_dir = self.config.music_dir.clone();
        let folder_mode = self.config.save_folder_mode;
        let format = self.config.save_audio_format;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            run_save_song(
                &music_dir,
                folder_mode,
                format,
                &parsed,
                &source_station,
                direct_url.as_deref(),
                |status| {
                    let _ = tx.send_blocking(CoreBgEvent::SaveProgress { job_id, status });
                },
            );
        });
        cx.notify();
    }

    pub fn copy_current_track(&mut self, cx: &mut Context<Self>) {
        let Some(txt) = self.current_track_text() else {
            return;
        };
        if self.window_open {
            cx.write_to_clipboard(ClipboardItem::new_string(txt.clone()));
        } else if let Ok(mut child) = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
        {
            let _ = stdin.write_all(txt.as_bytes());
        }
        self.show_toast(format!("Copied: {txt}"), cx);
    }

    // ---- Sleep Timer & Settings ----

    pub fn set_sleep_timer(&mut self, mins: Option<u64>, cx: &mut Context<Self>) {
        self.sleep_deadline = mins.map(|m| Instant::now() + Duration::from_secs(m * 60));
        match mins {
            Some(m) => self.show_toast(format!("Sleep timer set for {m}m"), cx),
            None => self.show_toast("Sleep timer cleared", cx),
        }
        self.sync_integrations();
        cx.notify();
    }

    pub fn cycle_sleep_timer(&mut self, cx: &mut Context<Self>) {
        let next = match self.sleep_remaining_mins() {
            None => Some(15),
            Some(m) if m <= 15 => Some(30),
            Some(m) if m <= 30 => Some(60),
            Some(_) => None,
        };
        self.set_sleep_timer(next, cx);
    }

    pub fn set_theme_preset(&mut self, preset: ThemePreset, cx: &mut Context<Self>) {
        self.config.theme = preset;
        self.palette = load_palette(preset);
        apply_palette(cx, &self.palette);
        self.persist_config();
        self.sync_lyrics_overlay();
        cx.notify();
    }

    pub fn set_country_code(&mut self, cc: String, cx: &mut Context<Self>) {
        self.config.country_code = cc.trim().to_uppercase();
        self.persist_config();
        self.refresh_radio_stations(cx);
    }

    pub fn toggle_local_blending(&mut self, cx: &mut Context<Self>) {
        self.config.local_blending = !self.config.local_blending;
        self.persist_config();
        self.refresh_radio_stations(cx);
    }

    pub fn toggle_visualizer(&mut self, cx: &mut Context<Self>) {
        self.config.visualizer_enabled = !self.config.visualizer_enabled;
        self.persist_config();
        self.refresh_visualizer_state();
        cx.notify();
    }

    fn wants_lyrics(&self) -> bool {
        self.lyrics_open || self.config.lyrics_overlay_enabled
    }

    fn overlay_header_for_current(&self) -> String {
        match &self.current_target {
            Some(PlayTarget::Local(t)) => {
                if t.artist.trim().is_empty() || t.artist.eq_ignore_ascii_case("Unknown Artist") {
                    format!("♪ {}", t.title)
                } else {
                    format!("♪ {} — {}", t.artist, t.title)
                }
            }
            Some(PlayTarget::Youtube(y)) => {
                let ch = y.channel.strip_suffix(" - Topic").unwrap_or(&y.channel).trim();
                if ch.is_empty() {
                    format!("♪ {}", y.title)
                } else {
                    format!("♪ {} — {}", ch, y.title)
                }
            }
            Some(PlayTarget::Radio(s)) => match self.current_track_text() {
                Some(tr) => format!("♪ {tr}"),
                None => format!("♪ {}", s.name),
            },
            None => "♪ Vibes".to_string(),
        }
    }

    fn sync_lyrics_overlay(&self) {
        if !self.config.lyrics_overlay_enabled {
            self.lyrics_overlay.set_disabled();
            return;
        }
        if !self.playing || self.paused {
            self.lyrics_overlay.hide();
            return;
        }
        let (primary, secondary) = match &self.lyrics_state {
            LyricsState::Loading => ("Searching lyrics…".to_string(), None),
            LyricsState::Ready(doc) => match &doc.kind {
                LyricsKind::Synced(lines) => {
                    match active_synced_line_index(lines, self.position_secs) {
                        Some(idx) => {
                            let pri = lines[idx].text.clone();
                            let sec = lines.get(idx + 1).map(|l| l.text.clone());
                            (pri, sec)
                        }
                        None => (
                            "♪ …".to_string(),
                            lines.first().map(|l| l.text.clone()),
                        ),
                    }
                }
                LyricsKind::Plain(_) => (
                    "Unsynced lyrics available".to_string(),
                    Some("Open Vibes (L) to read".to_string()),
                ),
                LyricsKind::Instrumental => {
                    self.lyrics_overlay.hide();
                    return;
                }
            },
            LyricsState::Empty | LyricsState::NotFound => {
                self.lyrics_overlay.hide();
                return;
            }
        };

        self.lyrics_overlay.show(OverlayFrame {
            header: self.overlay_header_for_current(),
            primary,
            secondary,
            bg_rgb: self.palette.bg_deep,
            border_rgb: self.palette.border,
            accent_rgb: self.palette.accent,
            fg_rgb: self.palette.fg,
            muted_rgb: self.palette.fg_muted,
        });
    }

    pub fn toggle_lyrics_overlay(&mut self, cx: &mut Context<Self>) {
        self.config.lyrics_overlay_enabled = !self.config.lyrics_overlay_enabled;
        self.persist_config();
        if self.config.lyrics_overlay_enabled {
            self.refresh_lyrics_for_current(cx);
            self.sync_lyrics_overlay();
            self.show_toast("Lyrics overlay enabled (click-through)", cx);
        } else {
            self.lyrics_overlay.set_disabled();
            self.show_toast("Lyrics overlay disabled", cx);
        }
        self.sync_integrations();
        cx.notify();
    }

    pub fn toggle_lyrics_panel(&mut self, cx: &mut Context<Self>) {
        self.lyrics_open = !self.lyrics_open;
        if self.lyrics_open {
            self.refresh_lyrics_for_current(cx);
        }
        cx.notify();
    }

    fn resolve_current_lyrics_target(&self) -> Option<(String, Option<PathBuf>, LyricsQuery)> {
        match &self.current_target {
            Some(PlayTarget::Local(t)) => {
                let artist = if t.artist.eq_ignore_ascii_case("Unknown Artist") {
                    String::new()
                } else {
                    t.artist.clone()
                };
                let query = LyricsQuery {
                    artist,
                    title: t.title.clone(),
                    album: (!t.album.trim().is_empty()).then_some(t.album.clone()),
                    duration_secs: t.duration_secs.or(self.duration_secs),
                };
                let key = format!("local|{}", t.path.display());
                Some((key, Some(t.path.clone()), query))
            }
            Some(PlayTarget::Youtube(y)) => {
                let ch = y
                    .channel
                    .strip_suffix(" - Topic")
                    .unwrap_or(&y.channel)
                    .trim()
                    .to_string();
                let (artist, title) = match parse_identifiable_track(&y.title, None) {
                    Some(p) => (p.artist.unwrap_or(ch), p.title),
                    None => (ch, y.title.clone()),
                };
                let query = LyricsQuery {
                    artist,
                    title,
                    album: None,
                    duration_secs: y.duration_secs.or(self.duration_secs),
                };
                let key = format!("yt|{}|{}", y.id, query.cache_key());
                Some((key, None, query))
            }
            Some(PlayTarget::Radio(s)) => {
                let text = self.current_track_text()?;
                let parsed = parse_identifiable_track(&text, Some(&s.name))?;
                let query = LyricsQuery {
                    artist: parsed.artist.unwrap_or_default(),
                    title: parsed.title,
                    album: None,
                    duration_secs: None,
                };
                let key = format!("radio|{}", query.cache_key());
                Some((key, None, query))
            }
            None => None,
        }
    }

    fn cache_lyrics_state(&mut self, key: &str, state: LyricsState) {
        if self.lyrics_cache.len() >= 64 && !self.lyrics_cache.contains_key(key) {
            self.lyrics_cache.clear();
        }
        self.lyrics_cache.insert(key.to_string(), state);
    }

    pub fn refresh_lyrics_for_current(&mut self, cx: &mut Context<Self>) {
        self.load_lyrics_for_current(false, cx);
    }

    pub fn force_reload_lyrics(&mut self, cx: &mut Context<Self>) {
        self.load_lyrics_for_current(true, cx);
    }

    fn load_lyrics_for_current(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some((key, local_path, query)) = self.resolve_current_lyrics_target() else {
            self.lyrics_fetch_gen += 1;
            self.lyrics_track_key = None;
            self.lyrics_state = LyricsState::Empty;
            cx.notify();
            return;
        };

        if !force
            && self.lyrics_track_key.as_deref() == Some(key.as_str())
            && self.lyrics_state != LyricsState::Empty
        {
            return;
        }

        self.lyrics_track_key = Some(key.clone());
        if !force && let Some(cached) = self.lyrics_cache.get(&key) {
            self.lyrics_state = cached.clone();
            cx.notify();
            return;
        }

        self.lyrics_fetch_gen += 1;
        let gen_id = self.lyrics_fetch_gen;
        self.lyrics_state = LyricsState::Loading;
        cx.notify();

        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let state = if let Some(ref path) = local_path
                && let Some(raw) = read_embedded_lyrics(path)
                && let Some(doc) = parse_lyrics_text(&raw, LyricsOrigin::Embedded)
            {
                LyricsState::Ready(doc)
            } else if !query.title.trim().is_empty() {
                let res = fetch_lrclib_lyrics(&new_http_agent(), &query);
                match lrclib_to_document(res) {
                    Some(doc) => LyricsState::Ready(doc),
                    None => LyricsState::NotFound,
                }
            } else {
                LyricsState::NotFound
            };
            let _ = tx.send_blocking(CoreBgEvent::LyricsLoaded { gen_id, key, state });
        });
    }

    pub fn embed_current_lyrics_to_file(&mut self, cx: &mut Context<Self>) {
        let (path, name, raw_lyrics) = match (&self.current_target, &self.lyrics_state) {
            (Some(PlayTarget::Local(t)), LyricsState::Ready(doc))
                if doc.origin == LyricsOrigin::Lrclib && doc.kind != LyricsKind::Instrumental =>
            {
                let name = t
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&t.title)
                    .to_string();
                (t.path.clone(), name, doc.raw.clone())
            }
            _ => return,
        };

        match embed_lyrics_in_file(&path, &raw_lyrics) {
            Ok(()) => {
                if let LyricsState::Ready(ref mut doc) = self.lyrics_state {
                    doc.origin = LyricsOrigin::Embedded;
                }
                if let Some(key) = self.lyrics_track_key.clone() {
                    let st = self.lyrics_state.clone();
                    self.cache_lyrics_state(&key, st);
                }
                self.show_toast(format!("Embedded lyrics into {name}"), cx);
                cx.notify();
            }
            Err(err) => {
                self.show_toast(format!("Failed to embed lyrics: {err}"), cx);
            }
        }
    }

    pub fn start_lyrics_backfill(&mut self, cx: &mut Context<Self>) {
        if self.lyrics_backfill.active {
            return;
        }
        let mut roots = vec![self.config.albums_dir.clone()];
        if self.config.music_dir != self.config.albums_dir {
            roots.push(self.config.music_dir.clone());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.lyrics_backfill_cancel = Some(Arc::clone(&cancel));
        self.lyrics_backfill = LyricsBackfillProgress {
            active: true,
            ..Default::default()
        };
        cx.notify();

        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            run_library_lyrics_backfill(roots, cancel, |prog| {
                let _ = tx.send_blocking(CoreBgEvent::LyricsBackfillUpdated(prog));
            });
        });
    }

    pub fn cancel_lyrics_backfill(&mut self, cx: &mut Context<Self>) {
        if let Some(ref cancel) = self.lyrics_backfill_cancel {
            cancel.store(true, Ordering::Release);
            self.show_toast("Cancelling lyrics backfill…", cx);
        }
    }

    pub fn toggle_close_to_tray(&mut self, cx: &mut Context<Self>) {
        self.config.close_to_tray = !self.config.close_to_tray;
        self.persist_config();
        cx.notify();
    }

    pub fn set_music_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.config.music_dir = dir;
        self.persist_config();
        self.refresh_local_library(cx);
    }

    pub fn set_save_folder_mode(&mut self, mode: SaveFolderMode, cx: &mut Context<Self>) {
        self.config.save_folder_mode = mode;
        self.persist_config();
        cx.notify();
    }

    pub fn set_save_audio_format(&mut self, fmt: SaveAudioFormat, cx: &mut Context<Self>) {
        self.config.save_audio_format = fmt;
        self.persist_config();
        cx.notify();
    }

    pub fn open_music_folder(&self) {
        self.open_folder(&self.config.music_dir);
    }

    pub fn open_folder(&self, path: &std::path::Path) {
        let _ = std::fs::create_dir_all(path);
        let _ = Command::new("xdg-open").arg(path).spawn();
    }

    pub fn set_albums_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.config.albums_dir = dir;
        self.persist_config();
        self.refresh_local_library(cx);
    }

    pub fn set_album_folder_mode(&mut self, mode: AlbumFolderMode, cx: &mut Context<Self>) {
        self.config.album_folder_mode = mode;
        self.persist_config();
        cx.notify();
    }

    // ---- Local Offline Library ----

    pub fn refresh_local_library(&mut self, cx: &mut Context<Self>) {
        self.local_scan_gen = self.local_scan_gen.wrapping_add(1);
        let gen_id = self.local_scan_gen;
        self.local_library_loading = true;
        let albums_dir = self.config.albums_dir.clone();
        let music_dir = self.config.music_dir.clone();
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            let snapshot = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scan_local_library(&albums_dir, &music_dir)
            }))
            .unwrap_or_default();
            let _ = tx.send_blocking(CoreBgEvent::LocalLibraryScanned { gen_id, snapshot });
            trim_allocator();
        });
        cx.notify();
    }

    pub fn set_local_filter_query(&mut self, q: String, cx: &mut Context<Self>) {
        if self.local_filter_query != q {
            self.local_filter_query = q;
            cx.notify();
        }
    }

    pub fn set_local_tab_singles(&mut self, singles: bool, cx: &mut Context<Self>) {
        if self.local_tab_singles != singles {
            self.local_tab_singles = singles;
            cx.notify();
        }
    }

    pub fn select_local_album(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.selected_local_album = folder;
        cx.notify();
    }

    // ---- Official Album Search, Playback & Full Download ----

    pub fn search_albums_query(&mut self, query: String, cx: &mut Context<Self>) {
        let q = query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.album_query = q.clone();
        self.selected_album = None;
        self.album_loading = true;
        self.album_error = None;
        self.album_fetch_gen = self.album_fetch_gen.wrapping_add(1);
        let gen_id = self.album_fetch_gen;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            let res = search_albums(&q).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::AlbumsSearched { gen_id, res });
        });
        cx.notify();
    }

    fn load_album_details(&mut self, summary: AlbumSummary, auto_download: bool, cx: &mut Context<Self>) {
        if !auto_download {
            self.album_detail_loading = true;
            self.album_error = None;
        } else {
            self.show_toast(format!("Fetching tracklist for {}…", summary.title), cx);
        }
        self.album_detail_gen = self.album_detail_gen.wrapping_add(1);
        let gen_id = self.album_detail_gen;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            let res = fetch_album_details(&summary).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::AlbumDetailsLoaded {
                gen_id,
                auto_download,
                res,
            });
        });
        cx.notify();
    }

    pub fn select_album(&mut self, summary: AlbumSummary, cx: &mut Context<Self>) {
        self.load_album_details(summary, false, cx);
    }

    pub fn quick_download_album(&mut self, summary: AlbumSummary, cx: &mut Context<Self>) {
        self.load_album_details(summary, true, cx);
    }

    pub fn clear_selected_album(&mut self, cx: &mut Context<Self>) {
        self.selected_album = None;
        self.album_detail_loading = false;
        cx.notify();
    }

    fn track_to_yt_item(track: &AlbumTrack) -> YoutubeItem {
        YoutubeItem {
            id: track.video_id.clone(),
            title: track.title.clone(),
            channel: track.artist.clone(),
            duration_secs: track.duration_secs,
            url: track.url.clone(),
            is_live: false,
            view_count: None,
        }
    }

    pub fn play_album_from_track(
        &mut self,
        details: &AlbumDetails,
        start_idx: usize,
        cx: &mut Context<Self>,
    ) {
        if start_idx >= details.tracks.len() {
            return;
        }

        let local_dir = find_local_album_dir(
            &self.config.albums_dir,
            &details.summary.artist,
            &details.summary.title,
        );
        let album_cover = local_dir
            .as_deref()
            .and_then(|d| resolve_local_album_art(d, &[]))
            .or_else(|| {
                details
                    .cover_url
                    .as_deref()
                    .or(details.summary.cover_url.as_deref())
                    .and_then(|u| self.art_path_for_url(u).cloned())
            });

        let resolve_item = |tr: &AlbumTrack| -> QueueItem {
            if let Some(path) = local_dir
                .as_deref()
                .and_then(|d| find_local_track_in_dir(d, tr.track_number, &tr.title))
            {
                let ext_upper = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default()
                    .to_uppercase();
                let track_cover = album_cover.clone().or_else(|| {
                    path.parent()
                        .and_then(|d| resolve_local_album_art(d, &[path.clone()]))
                });
                return QueueItem::Local(LocalTrack {
                    path,
                    title: tr.title.clone(),
                    artist: tr.artist.clone(),
                    album: details.summary.title.clone(),
                    track_number: Some(tr.track_number),
                    duration_secs: tr.duration_secs,
                    format: ext_upper,
                    cover_path: track_cover,
                });
            }
            if let Some(u) = youtube_thumb_url(&tr.video_id) {
                self.prefetch_art_url(&u, COVER_MAX_DIM);
            }
            QueueItem::Youtube(Self::track_to_yt_item(tr))
        };

        let mut items: Vec<QueueItem> = details.tracks[start_idx..]
            .iter()
            .map(resolve_item)
            .collect();
        if items.is_empty() {
            return;
        }
        let all_local = items.iter().all(QueueItem::is_local);
        let first = items.remove(0);
        let first_local = first.is_local();
        self.queue = items;
        self.play_queue_item(first, cx);
        if first_local {
            self.show_toast(
                if all_local {
                    "Playing locally from disk (offline)"
                } else {
                    "Playing track locally from disk"
                },
                cx,
            );
        }
    }

    pub fn enqueue_album_download(&mut self, details: AlbumDetails, cx: &mut Context<Self>) {
        if details.tracks.is_empty() {
            self.show_toast("Album has no downloadable tracks", cx);
            return;
        }
        if self.album_downloads.iter().any(|j| {
            j.title.eq_ignore_ascii_case(&details.summary.title)
                && j.artist.eq_ignore_ascii_case(&details.summary.artist)
                && matches!(
                    j.status,
                    AlbumDownloadStatus::Queued | AlbumDownloadStatus::Downloading { .. }
                )
        }) {
            self.show_toast(format!("Already downloading {}", details.summary.title), cx);
            return;
        }

        let job_id = self.next_job_id;
        self.next_job_id += 1;
        let folder = resolve_album_dir(
            &self.config.albums_dir,
            self.config.album_folder_mode,
            &details.summary.artist,
            &details.summary.title,
        );
        let total = details.tracks.len();
        let title = details.summary.title.clone();
        self.album_downloads.insert(
            0,
            AlbumDownloadJob {
                id: job_id,
                title: title.clone(),
                artist: details.summary.artist.clone(),
                year: details.summary.year,
                folder: folder.clone(),
                status: AlbumDownloadStatus::Queued,
                created_at: Local::now().format("%H:%M:%S").to_string(),
                details: details.clone(),
            },
        );
        self.show_toast(format!("Downloading album: {title} ({total} tracks)…"), cx);
        self.sync_integrations();

        let albums_dir = self.config.albums_dir.clone();
        let mode = self.config.album_folder_mode;
        let format = self.config.save_audio_format;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            run_download_album(&albums_dir, mode, format, &details, |status| {
                let _ = tx.send_blocking(CoreBgEvent::AlbumDownloadProgress { job_id, status });
            });
        });
        cx.notify();
    }

    pub fn retry_album_download(&mut self, job_id: u64, cx: &mut Context<Self>) {
        let Some(job) = self.album_downloads.iter().find(|j| j.id == job_id).cloned() else {
            return;
        };
        self.enqueue_album_download(job.details, cx);
    }

    // ---- Internal Event Handlers ----

    fn tick_second(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(deadline) = self.sleep_deadline {
            if Instant::now() >= deadline {
                self.sleep_deadline = None;
                self.set_paused(true, cx);
                self.show_toast("Sleep timer finished · Paused", cx);
            } else if self.sleep_remaining_mins() != self.last_sleep_mins_synced {
                self.sync_integrations();
                if self.window_open {
                    cx.notify();
                }
            }
        }

        if self.playing
            && !self.paused
            && (self.position_secs - self.last_resume_save_secs).abs() >= 15.0
        {
            self.last_resume_save_secs = self.position_secs;
            self.save_current_yt_resume();
        }
        true
    }

    fn handle_mpv_event(&mut self, ev: MpvEvent, cx: &mut Context<Self>) {
        let mut sync_dbus = false;
        let mut dirty_ui = false;

        match ev {
            MpvEvent::PauseChanged(p) => {
                // Live radio uses mpv stop/reload for pause; ignore mpv's synthetic pause resets.
                if !self.is_live_stream() && !self.user_stopped && self.playing {
                    self.paused = p;
                    self.refresh_visualizer_state();
                    self.sync_lyrics_overlay();
                    sync_dbus = true;
                    dirty_ui = true;
                }
            }
            MpvEvent::BufferingChanged(b) => {
                if !self.paused {
                    self.buffering = b;
                    sync_dbus = true;
                    dirty_ui = true;
                }
            }
            MpvEvent::FileLoaded | MpvEvent::PlaybackRestart => {
                self.buffering = false;
                self.reconnect_attempts = 0;
                if !self.user_stopped && !self.paused {
                    self.playing = true;
                }
                self.refresh_visualizer_state();
                self.sync_lyrics_overlay();
                sync_dbus = true;
                dirty_ui = true;
            }
            MpvEvent::MediaTitleChanged(title) => {
                let changed = self.media_title.as_deref() != Some(title.as_str());
                self.media_title = Some(title.clone());
                if changed {
                    self.record_song_log(&title);
                    if self.wants_lyrics()
                        && matches!(self.current_target, Some(PlayTarget::Radio(_)))
                    {
                        self.refresh_lyrics_for_current(cx);
                    }
                    self.sync_lyrics_overlay();
                    sync_dbus = true;
                    dirty_ui = true;
                }
            }
            MpvEvent::IcyMetadata { title, name } => {
                if let Some(t) = title {
                    let changed = self.icy_title.as_deref() != Some(t.as_str());
                    self.icy_title = Some(t.clone());
                    if changed {
                        self.record_song_log(&t);
                        if self.wants_lyrics() {
                            self.refresh_lyrics_for_current(cx);
                        }
                        self.sync_lyrics_overlay();
                        sync_dbus = true;
                        dirty_ui = true;
                    }
                }
                if let Some(n) = name {
                    self.icy_name = Some(n);
                    dirty_ui = true;
                }
            }
            MpvEvent::TimePosChanged(pos) => {
                let prev_pos = self.position_secs;
                let prev_whole = prev_pos as u64;
                self.position_secs = pos;
                let line_changed = if let LyricsState::Ready(ref doc) = self.lyrics_state
                    && let LyricsKind::Synced(ref lines) = doc.kind
                {
                    active_synced_line_index(lines, prev_pos)
                        != active_synced_line_index(lines, pos)
                } else {
                    false
                };
                if (pos as u64) != prev_whole || (self.lyrics_open && line_changed) {
                    dirty_ui = true;
                }
                if self.config.lyrics_overlay_enabled && line_changed {
                    self.sync_lyrics_overlay();
                }
            }
            MpvEvent::DurationChanged(d) => {
                self.duration_secs = d;
                sync_dbus = true;
                dirty_ui = true;
            }
            MpvEvent::AudioCodecChanged(c) => {
                self.codec = Some(c);
                dirty_ui = true;
            }
            MpvEvent::AudioBitrateChanged(br) => {
                self.bitrate_kbps = Some(br);
                dirty_ui = true;
            }
            MpvEvent::SampleRateChanged(sr) => {
                self.sample_rate = Some(sr);
                dirty_ui = true;
            }
            MpvEvent::VolumeChanged(v) => {
                let prev_bucket = (self.volume / 5.0).round() as i32;
                let next_bucket = (v / 5.0).round() as i32;
                self.volume = v;
                if prev_bucket != next_bucket {
                    sync_dbus = true;
                }
                dirty_ui = true;
            }
            MpvEvent::MuteChanged(m) => {
                self.muted = m;
                sync_dbus = true;
                dirty_ui = true;
            }
            MpvEvent::SpeedChanged(s) => {
                self.speed = s;
                dirty_ui = true;
            }
            MpvEvent::Error(msg) => {
                self.show_toast(format!("Stream error: {msg}"), cx);
            }
            MpvEvent::EndFile { reason } => {
                // `loadfile ... replace` and playlist redirects emit `stop`/`redirect` for the old entry
                // right before starting the new stream; never treat those as playback ending.
                if reason != "stop"
                    && reason != "redirect"
                    && !self.user_stopped
                    && self.playing
                    && !self.paused
                {
                    match self.current_target.clone() {
                        Some(PlayTarget::Youtube(_)) | Some(PlayTarget::Local(_))
                            if reason == "eof" =>
                        {
                            if let Some(PlayTarget::Youtube(ref yt)) = self.current_target
                                && !yt.id.is_empty()
                            {
                                self.state.resume_positions.remove(&yt.id);
                                self.persist_state();
                            }
                            if !self.queue.is_empty() {
                                self.play_next_in_queue(cx);
                                return;
                            }
                            self.playing = false;
                            self.paused = false;
                            self.buffering = false;
                            self.refresh_visualizer_state();
                            self.sync_lyrics_overlay();
                            sync_dbus = true;
                            dirty_ui = true;
                        }
                        Some(PlayTarget::Radio(st)) if reason != "stop" => {
                            if self.reconnect_attempts < 3 {
                                self.reconnect_attempts += 1;
                                let wait = Duration::from_secs(self.reconnect_attempts as u64 * 2);
                                self.buffering = true;
                                sync_dbus = true;
                                dirty_ui = true;
                                let url = st.stream_url().to_string();
                                cx.spawn(async move |weak, cx| {
                                    cx.background_executor().timer(wait).await;
                                    let _ = weak.update(cx, |this, cx| {
                                        if !this.user_stopped
                                            && matches!(this.current_target, Some(PlayTarget::Radio(_)))
                                            && let Some(ref mpv) = this.mpv
                                        {
                                            let _ = mpv.load_url(&url, None);
                                            cx.notify();
                                        }
                                    });
                                })
                                .detach();
                            } else {
                                self.playing = false;
                                self.buffering = false;
                                self.sync_lyrics_overlay();
                                self.show_toast("Stream dropped after 3 reconnect attempts", cx);
                                sync_dbus = true;
                                dirty_ui = true;
                            }
                        }
                        _ => {
                            self.playing = false;
                            self.buffering = false;
                            self.refresh_visualizer_state();
                            sync_dbus = true;
                            dirty_ui = true;
                        }
                    }
                }
            }
        }

        if sync_dbus {
            self.sync_integrations();
        }
        if dirty_ui && self.window_open {
            cx.notify();
        }
    }

    fn handle_tray_action(&mut self, act: TrayAction, cx: &mut Context<Self>) {
        match act {
            TrayAction::TogglePlay => self.toggle_play(cx),
            TrayAction::ToggleWindow => {
                let _ = self.window_tx.try_send(WindowCmd::Toggle);
            }
            TrayAction::ToggleLyricsOverlay => self.toggle_lyrics_overlay(cx),
            TrayAction::PlayFavorite(uuid) => {
                if let Some(st) = self
                    .state
                    .favorites
                    .iter()
                    .find(|f| f.stationuuid == uuid)
                    .cloned()
                {
                    self.play_station(st, cx);
                }
            }
            TrayAction::SaveCurrentSong => self.save_current_song(cx),
            TrayAction::CopyTrackName => self.copy_current_track(cx),
            TrayAction::AdjustVolume(delta) => self.set_volume(self.volume + delta, cx),
            TrayAction::SetSleepTimer(mins) => self.set_sleep_timer(mins, cx),
            TrayAction::Quit => {
                let _ = self.window_tx.try_send(WindowCmd::Quit);
            }
        }
    }

    fn handle_mpris_action(&mut self, act: MprisAction, cx: &mut Context<Self>) {
        match act {
            MprisAction::Play => self.set_paused(false, cx),
            MprisAction::Pause => self.set_paused(true, cx),
            MprisAction::PlayPause => self.toggle_play(cx),
            MprisAction::Stop => self.stop_playback(cx),
            MprisAction::Next => self.play_next_in_queue(cx),
            MprisAction::Previous => self.play_previous_action(cx),
            MprisAction::SeekMicros(us) => self.seek_relative((us as f64) / 1_000_000.0, cx),
            MprisAction::Raise => {
                let _ = self.window_tx.try_send(WindowCmd::Show);
            }
            MprisAction::Quit => {
                let _ = self.window_tx.try_send(WindowCmd::Quit);
            }
        }
    }

    fn handle_bg_event(&mut self, ev: CoreBgEvent, cx: &mut Context<Self>) {
        match ev {
            CoreBgEvent::RadioFetched {
                gen_id,
                append_custom,
                res,
            } => {
                if gen_id != self.radio_fetch_gen {
                    return;
                }
                self.radio_loading = false;
                match res {
                    Ok(mut fetched) => {
                        self.radio_error = None;
                        save_cached_stations(&fetched);
                        if append_custom && !self.state.custom_stations.is_empty() {
                            let mut combined = self.state.custom_stations.clone();
                            combined.append(&mut fetched);
                            self.stations = combined;
                        } else {
                            self.stations = fetched;
                        }
                        self.prefetch_stations_art(&self.stations);
                    }
                    Err(err) => {
                        self.radio_error = Some(err);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::RandomStationPicked(res) => match res {
                Ok(st) => self.play_station(st, cx),
                Err(err) => self.show_toast(format!("Dial error: {err}"), cx),
            },
            CoreBgEvent::YoutubeSearched { gen_id, query, res } => {
                if gen_id != self.yt_fetch_gen {
                    return;
                }
                self.yt_loading = false;
                match res {
                    Ok(items) => {
                        self.yt_error = None;
                        for it in &items {
                            if let Some(u) = youtube_thumb_url(&it.id) {
                                self.prefetch_art_url(&u, COVER_MAX_DIM);
                            }
                        }
                        self.yt_results = items;
                        self.state.recent_yt_searches.retain(|q| !q.eq_ignore_ascii_case(&query));
                        self.state.recent_yt_searches.insert(0, query);
                        self.state.recent_yt_searches.truncate(10);
                        self.persist_state();
                    }
                    Err(err) => {
                        self.yt_error = Some(err);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::AlbumsSearched { gen_id, res } => {
                if gen_id != self.album_fetch_gen {
                    return;
                }
                self.album_loading = false;
                match res {
                    Ok(list) => {
                        self.album_error = None;
                        for summary in &list {
                            if let Some(ref url) = summary.cover_url {
                                self.prefetch_art_url(url, COVER_MAX_DIM);
                            }
                        }
                        self.album_results = list;
                    }
                    Err(err) => {
                        self.album_error = Some(err);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::AlbumDetailsLoaded {
                gen_id,
                auto_download,
                res,
            } => {
                if !auto_download && gen_id != self.album_detail_gen {
                    return;
                }
                self.album_detail_loading = false;
                match res {
                    Ok(details) => {
                        self.album_error = None;
                        if let Some(ref url) = details.cover_url {
                            self.prefetch_art_url(url, COVER_MAX_DIM);
                        }
                        if let Some(ref url) = details.summary.cover_url {
                            self.prefetch_art_url(url, COVER_MAX_DIM);
                        }
                        if auto_download {
                            self.enqueue_album_download(details, cx);
                        } else {
                            self.selected_album = Some(details);
                        }
                    }
                    Err(err) => {
                        self.album_error = Some(err.clone());
                        self.show_toast(format!("Album error: {err}"), cx);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::AlbumDownloadProgress { job_id, status } => {
                let title = self
                    .album_downloads
                    .iter()
                    .find(|j| j.id == job_id)
                    .map(|j| j.title.clone())
                    .unwrap_or_default();
                if let Some(job) = self.album_downloads.iter_mut().find(|j| j.id == job_id) {
                    job.status = status.clone();
                }
                match status {
                    AlbumDownloadStatus::Completed {
                        saved,
                        skipped,
                        failed,
                        ..
                    } => {
                        self.show_toast(
                            format!("Album '{title}' ready: {saved} saved, {skipped} skipped, {failed} failed"),
                            cx,
                        );
                        self.refresh_local_library(cx);
                    }
                    AlbumDownloadStatus::Failed(ref err) => {
                        self.show_toast(format!("Album '{title}' failed: {err}"), cx);
                    }
                    AlbumDownloadStatus::Queued | AlbumDownloadStatus::Downloading { .. } => {}
                }
                self.sync_integrations();
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::LocalLibraryScanned { gen_id, snapshot } => {
                if gen_id != self.local_scan_gen {
                    return;
                }
                self.local_library_loading = false;
                if let Some(ref sel) = self.selected_local_album
                    && !snapshot.albums.iter().any(|a| &a.folder == sel)
                {
                    self.selected_local_album = None;
                }
                if let Some(PlayTarget::Local(ref mut cur)) = self.current_target
                    && cur.cover_path.is_none()
                {
                    cur.cover_path = snapshot
                        .albums
                        .iter()
                        .flat_map(|a| a.tracks.iter())
                        .chain(snapshot.singles.iter())
                        .find(|t| t.path == cur.path)
                        .and_then(|t| t.cover_path.clone());
                }
                self.local_library = snapshot;
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::ArtworkLoaded { key, path } => {
                self.art_cache.insert(key, path);
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::SaveProgress { job_id, status } => {
                let label = self
                    .downloads
                    .iter()
                    .find(|j| j.id == job_id)
                    .map(|j| j.label.clone())
                    .unwrap_or_default();
                if let Some(job) = self.downloads.iter_mut().find(|j| j.id == job_id) {
                    job.status = status.clone();
                }
                match status {
                    DownloadStatus::Saved(ref p) => {
                        let name = p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&label);
                        self.show_toast(format!("Saved: {name}"), cx);
                        self.refresh_local_library(cx);
                    }
                    DownloadStatus::AlreadyExists(ref p) => {
                        let name = p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&label);
                        self.show_toast(format!("Already in library: {name}"), cx);
                    }
                    DownloadStatus::Failed(ref err) => {
                        self.show_toast(format!("Save failed: {err}"), cx);
                    }
                    DownloadStatus::Searching | DownloadStatus::Downloading(_) => {}
                }
                self.sync_integrations();
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::LyricsLoaded { gen_id, key, state } => {
                self.cache_lyrics_state(&key, state.clone());
                if gen_id == self.lyrics_fetch_gen
                    && self.lyrics_track_key.as_deref() == Some(key.as_str())
                {
                    self.lyrics_state = state;
                    self.sync_lyrics_overlay();
                    if self.window_open {
                        cx.notify();
                    }
                }
            }
            CoreBgEvent::LyricsBackfillUpdated(prog) => {
                let finished = self.lyrics_backfill.active && !prog.active;
                self.lyrics_backfill = prog.clone();
                if finished {
                    self.lyrics_backfill_cancel = None;
                    self.show_toast(
                        format!(
                            "Lyrics backfill: {} embedded, {} already tagged, {} not found",
                            prog.embedded, prog.skipped_existing, prog.not_found
                        ),
                        cx,
                    );
                    if prog.embedded > 0
                        && self.wants_lyrics()
                        && matches!(self.current_target, Some(PlayTarget::Local(_)))
                    {
                        self.force_reload_lyrics(cx);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
        }
    }

    pub fn shutdown(&mut self, cx: &mut App) {
        if let Some(ref cancel) = self.lyrics_backfill_cancel {
            cancel.store(true, Ordering::Release);
        }
        self.lyrics_overlay.shutdown();
        self.save_current_yt_resume();
        self.persist_config();
        self.persist_state();
        if let Some(ref mpv) = self.mpv {
            mpv.shutdown();
        }
        self.visualizer.stop();
        self.tray.shutdown();
        self._instance_guard = None;
        cx.quit();
    }
}

/// Steps volume onto crisp 5% increments in `0.0..=100.0`.
pub fn step_volume_5(current: f64, up: bool) -> f64 {
    let clamped = current.clamp(0.0, 100.0);
    let nearest = (clamped / 5.0).round() * 5.0;
    let next = if up {
        if clamped < nearest - 0.1 {
            nearest
        } else {
            nearest + 5.0
        }
    } else if clamped > nearest + 0.1 {
        nearest
    } else {
        nearest - 5.0
    };
    next.clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_volume_in_clean_five_percent_increments() {
        assert_eq!(step_volume_5(80.0, true), 85.0);
        assert_eq!(step_volume_5(80.0, false), 75.0);
        assert_eq!(step_volume_5(73.0, true), 75.0);
        assert_eq!(step_volume_5(73.0, false), 70.0);
        assert_eq!(step_volume_5(72.0, true), 75.0);
        assert_eq!(step_volume_5(72.0, false), 70.0);
        assert_eq!(step_volume_5(98.0, true), 100.0);
        assert_eq!(step_volume_5(100.0, true), 100.0);
        assert_eq!(step_volume_5(2.0, false), 0.0);
        assert_eq!(step_volume_5(0.0, false), 0.0);
    }
}
