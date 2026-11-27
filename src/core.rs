use crate::albums::{
    AlbumDetails, AlbumDownloadJob, AlbumDownloadStatus, AlbumSummary, AlbumTrack,
    fetch_album_details, resolve_album_dir, run_download_album, search_albums,
};
use crate::artwork::{
    ArtworkFetcher, COVER_MAX_DIM, ICON_MAX_DIM, resolve_local_album_art, trim_allocator,
    youtube_thumb_url,
};
pub use crate::config::QueueItem;
use crate::config::{
    AlbumFolderMode, FailedDownloadItem, FailedDownloadKind, FollowedArtist, PlayTarget,
    RepeatMode, SaveAudioFormat, SaveFolderMode, SongLogEntry, UserPlaylist, VibesConfig,
    VibesState,
};
use crate::discography::{
    ArtistDiscography, ArtistSummary, ArtistSyncStatus, fetch_artist_discography,
    normalize_artist_key, run_artist_sync, search_artists,
};
use crate::downloader::{
    DownloadJob, DownloadStatus, ParsedTrack, parse_identifiable_track, run_retry_failed_item,
    run_save_song,
};
use crate::instance::InstanceGuard;
use crate::local_library::{
    LocalAlbum, LocalArtist, LocalLibrarySnapshot, LocalTrack, ReorgReport, find_local_album_dir,
    find_local_track_in_dir, reorganize_music_library, scan_local_library,
};
use crate::lyrics::{
    LyricsBackfillProgress, LyricsKind, LyricsOrigin, LyricsQuery, LyricsState,
    active_synced_line_index, embed_lyrics_in_file, fetch_lrclib_lyrics, lrclib_to_document,
    new_http_agent, parse_lyrics_text, read_embedded_lyrics, run_library_lyrics_backfill,
};
use crate::lyrics_overlay::{LyricsOverlayController, OverlayFrame};
use crate::mpris::{MprisAction, MprisController, MprisSnapshot};
use crate::mpv::{MpvController, MpvEvent, resolve_youtube_audio_stream_url};
use crate::radio::{
    RadioBrowserClient, RadioStation, load_cached_stations, save_cached_stations,
};
use crate::theme::{Palette, ThemePreset, apply_palette, load_palette};
use crate::tray::{TrayAction, TrayController, TrayPlaybackState, TraySnapshot};
use crate::visualizer::VisualizerController;
use crate::youtube::{YoutubeFilter, YoutubeItem, format_duration, search_youtube};
use chrono::Local;
use gpui_kit::{App, ClipboardItem, Context};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavSection {
    #[default]
    Artists,
    Albums,
    Playlists,
    Discover,
    Radio,
    Queue,
    Downloads,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverTab {
    #[default]
    Artists,
    Albums,
    Tracks,
}

impl DiscoverTab {
    pub const ALL: [Self; 3] = [Self::Artists, Self::Albums, Self::Tracks];

    pub fn label(self) -> &'static str {
        match self {
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Tracks => "Tracks & Streams",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioTab {
    #[default]
    Directory,
    Favorites,
    SongLog,
}

impl RadioTab {
    pub const ALL: [Self; 3] = [Self::Directory, Self::Favorites, Self::SongLog];

    pub fn label(self) -> &'static str {
        match self {
            Self::Directory => "Directory",
            Self::Favorites => "Starred Presets",
            Self::SongLog => "Heard on Radio",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlbumSortMode {
    #[default]
    Artist,
    Title,
    YearDesc,
    TrackCount,
}

impl AlbumSortMode {
    pub const ALL: [Self; 4] = [Self::Artist, Self::Title, Self::YearDesc, Self::TrackCount];

    pub fn label(self) -> &'static str {
        match self {
            Self::Artist => "Artist",
            Self::Title => "Title",
            Self::YearDesc => "Newest",
            Self::TrackCount => "Tracks",
        }
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
    AlbumDownloadFinished {
        job_id: u64,
        status: AlbumDownloadStatus,
        failures: Vec<FailedDownloadItem>,
    },
    ArtistsSearched {
        gen_id: u64,
        res: Result<Vec<ArtistSummary>, String>,
    },
    ArtistDiscographyLoaded {
        gen_id: u64,
        res: Result<ArtistDiscography, String>,
    },
    ArtistSyncProgress(ArtistSyncStatus),
    ArtistSyncFinished {
        browse_id: String,
        status: ArtistSyncStatus,
        failures: Vec<FailedDownloadItem>,
    },
    FollowArtistResolved {
        name: String,
        res: Result<Option<ArtistSummary>, String>,
    },
    FailedItemRetried {
        id: String,
        res: Result<PathBuf, String>,
    },
    ReorgCompleted(Result<ReorgReport, String>),
    StreamUrlPrefetched {
        video_id: String,
        res: Result<String, String>,
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
    pub discover_tab: DiscoverTab,
    pub radio_tab: RadioTab,
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

    // Online Artist Discovery & Follow Sync state
    pub artist_query: String,
    pub artist_results: Vec<ArtistSummary>,
    pub artist_loading: bool,
    pub artist_error: Option<String>,
    artist_fetch_gen: u64,
    pub selected_online_artist: Option<ArtistDiscography>,
    pub online_artist_loading: bool,
    online_artist_gen: u64,
    pub artist_sync: ArtistSyncStatus,
    artist_sync_cancel: Option<Arc<AtomicBool>>,
    pub artist_sync_queue: VecDeque<FollowedArtist>,

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

    // Local offline library & Playlists state
    pub local_library: LocalLibrarySnapshot,
    pub local_library_loading: bool,
    local_scan_gen: u64,
    pub local_filter_query: String,
    pub album_sort: AlbumSortMode,
    pub selected_local_artist: Option<String>,
    pub selected_local_album: Option<PathBuf>,
    pub selected_playlist_id: Option<String>,
    pub reorg_running: bool,

    // Stream prefetch cache (video_id -> (cdn_url, fetched_at))
    stream_url_cache: HashMap<String, (String, Instant)>,
    prefetching_video_ids: HashSet<String>,

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
    pub queue_history: Vec<QueueItem>,
    last_prev_click_at: Option<Instant>,
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
        let initial_queue = state.saved_queue.clone();

        let mut core = Self {
            config,
            state,
            palette,
            section: NavSection::Artists,
            discover_tab: DiscoverTab::Artists,
            radio_tab: RadioTab::Directory,
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

            artist_query: String::new(),
            artist_results: Vec::new(),
            artist_loading: false,
            artist_error: None,
            artist_fetch_gen: 0,
            selected_online_artist: None,
            online_artist_loading: false,
            online_artist_gen: 0,
            artist_sync: ArtistSyncStatus::Idle,
            artist_sync_cancel: None,
            artist_sync_queue: VecDeque::new(),

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
            album_sort: AlbumSortMode::Artist,
            selected_local_artist: None,
            selected_local_album: None,
            selected_playlist_id: None,
            reorg_running: false,

            stream_url_cache: HashMap::new(),
            prefetching_video_ids: HashSet::new(),

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
            queue: initial_queue,
            queue_history: Vec::new(),
            last_prev_click_at: None,
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

    fn persist_queue(&mut self) {
        self.state.saved_queue = self.queue.clone();
        self.persist_state();
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
        let sync = usize::from(self.artist_sync.is_active());
        songs + albums + sync
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
        self.persist_queue();
        self.show_toast(format!("Queued: {}", item.title), cx);
        if !self.playing && self.current_target.is_none() {
            self.play_next_in_queue(cx);
        } else {
            self.prefetch_next_youtube_stream(cx);
        }
        cx.notify();
    }

    pub fn queue_local_track(&mut self, track: LocalTrack, cx: &mut Context<Self>) {
        self.queue.push(QueueItem::Local(track.clone()));
        self.persist_queue();
        self.show_toast(format!("Queued offline: {}", track.title), cx);
        if !self.playing && self.current_target.is_none() {
            self.play_next_in_queue(cx);
        }
        cx.notify();
    }

    pub fn queue_play_next(&mut self, item: QueueItem, cx: &mut Context<Self>) {
        if let QueueItem::Youtube(ref y) = item
            && let Some(u) = youtube_thumb_url(&y.id)
        {
            self.prefetch_art_url(&u, COVER_MAX_DIM);
        }
        let title = item.title().to_string();
        self.queue.insert(0, item);
        self.persist_queue();
        self.show_toast(format!("Up next: {title}"), cx);
        if !self.playing && self.current_target.is_none() {
            self.play_next_in_queue(cx);
        } else {
            self.prefetch_next_youtube_stream(cx);
        }
        cx.notify();
    }

    pub fn move_queue_item(&mut self, idx: usize, delta: isize, cx: &mut Context<Self>) {
        let new_idx = idx as isize + delta;
        if idx < self.queue.len() && new_idx >= 0 && (new_idx as usize) < self.queue.len() {
            self.queue.swap(idx, new_idx as usize);
            self.persist_queue();
            self.prefetch_next_youtube_stream(cx);
            cx.notify();
        }
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.state.shuffle_enabled = !self.state.shuffle_enabled;
        self.persist_state();
        self.show_toast(
            if self.state.shuffle_enabled {
                "Shuffle: On"
            } else {
                "Shuffle: Off"
            },
            cx,
        );
        cx.notify();
    }

    pub fn cycle_repeat_mode(&mut self, cx: &mut Context<Self>) {
        self.state.repeat_mode = self.state.repeat_mode.cycle();
        self.persist_state();
        self.show_toast(format!("Repeat: {}", self.state.repeat_mode.label()), cx);
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
            self.persist_queue();
            cx.notify();
        }
    }

    pub fn clear_queue(&mut self, cx: &mut Context<Self>) {
        self.queue.clear();
        self.persist_queue();
        cx.notify();
    }

    pub fn play_queue_index(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.queue.len() {
            return;
        }
        if let Some(cur) = target_as_queue_item(&self.current_target) {
            push_queue_history(&mut self.queue_history, cur);
        }
        let item = self.queue.remove(idx);
        self.persist_queue();
        self.play_queue_item(item, cx);
    }

    pub fn play_youtube_standalone(&mut self, item: YoutubeItem, cx: &mut Context<Self>) {
        match &self.current_target {
            Some(PlayTarget::Local(t)) => {
                push_queue_history(&mut self.queue_history, QueueItem::Local(t.clone()));
            }
            Some(PlayTarget::Youtube(cur)) if cur.id != item.id => {
                push_queue_history(&mut self.queue_history, QueueItem::Youtube(cur.clone()));
            }
            _ => {}
        }
        self.play_youtube(item, cx);
    }

    pub fn play_next_in_queue(&mut self, cx: &mut Context<Self>) {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        if let Some(next) = step_queue_advance(
            &self.current_target,
            &mut self.queue_history,
            &mut self.queue,
            self.state.shuffle_enabled,
            self.state.repeat_mode,
            false,
            seed,
        ) {
            self.persist_queue();
            self.play_queue_item(next, cx);
        } else if matches!(self.current_target, Some(PlayTarget::Radio(_)) | None) {
            self.cycle_favorite(1, cx);
        } else {
            self.show_toast("End of queue", cx);
        }
    }

    pub fn play_previous_action(&mut self, cx: &mut Context<Self>) {
        let rapid_second_click = self
            .last_prev_click_at
            .is_some_and(|t| t.elapsed() <= PREV_DOUBLE_CLICK_WINDOW);
        match step_queue_previous(
            &self.current_target,
            self.position_secs,
            rapid_second_click,
            &mut self.queue_history,
            &mut self.queue,
        ) {
            PreviousStep::SeekToStart => {
                self.last_prev_click_at = Some(Instant::now());
                self.seek_absolute(0.0, cx);
            }
            PreviousStep::PlayTrack(prev) => {
                self.last_prev_click_at = None;
                self.persist_queue();
                self.play_queue_item(prev, cx);
            }
            PreviousStep::CycleFavorite => {
                self.last_prev_click_at = None;
                self.cycle_favorite(-1, cx);
            }
        }
    }

    fn prefetch_next_youtube_stream(&mut self, cx: &Context<Self>) {
        let next_vid = self.queue.iter().find_map(|q| match q {
            QueueItem::Youtube(y) if !y.is_live && !y.id.is_empty() => Some((y.id.clone(), y.url.clone())),
            _ => None,
        });
        let Some((vid, url)) = next_vid else {
            return;
        };
        if let Some((_, fetched_at)) = self.stream_url_cache.get(&vid)
            && fetched_at.elapsed() < Duration::from_secs(3 * 3600)
        {
            return;
        }
        if !self.prefetching_video_ids.insert(vid.clone()) {
            return;
        }
        let tx = self.bg_tx.clone();
        let _ = cx;
        std::thread::spawn(move || {
            let res = resolve_youtube_audio_stream_url(&url).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::StreamUrlPrefetched { video_id: vid, res });
        });
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
        self.queue_history.clear();
        self.last_prev_click_at = None;
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
        self.last_prev_click_at = None;
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

        let stream_target = if !item.is_live
            && !item.id.is_empty()
            && let Some((cached_url, fetched_at)) = self.stream_url_cache.get(&item.id)
            && fetched_at.elapsed() < Duration::from_secs(3 * 3600)
        {
            cached_url.clone()
        } else {
            item.url.clone()
        };

        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_speed(self.speed);
            if let Err(err) = mpv.load_url(&stream_target, resume) {
                self.show_toast(format!("YouTube error: {err}"), cx);
            }
        }
        if let Some(r) = resume {
            self.show_toast(format!("Resumed at {}", format_duration(r)), cx);
        }

        self.prefetch_next_youtube_stream(cx);

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
        self.last_prev_click_at = None;
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
        self.queue_history = tracks[..start_idx]
            .iter()
            .cloned()
            .map(QueueItem::Local)
            .collect();
        self.queue = tracks[start_idx + 1..]
            .iter()
            .cloned()
            .map(QueueItem::Local)
            .collect();
        self.persist_queue();
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

    pub fn play_tracks_shuffled(&mut self, mut tracks: Vec<LocalTrack>, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        shuffle_slice(&mut tracks);
        self.state.shuffle_enabled = true;
        self.persist_state();
        self.play_local_list_from(&tracks, 0, cx);
    }

    pub fn play_artist_all(&mut self, artist: &LocalArtist, shuffled: bool, cx: &mut Context<Self>) {
        let all: Vec<LocalTrack> = artist
            .albums
            .iter()
            .flat_map(|a| a.tracks.iter().cloned())
            .chain(artist.singles.iter().cloned())
            .collect();
        if all.is_empty() {
            return;
        }
        if shuffled {
            self.play_tracks_shuffled(all, cx);
        } else {
            self.play_local_list_from(&all, 0, cx);
        }
    }

    pub fn jump_to_artist_by_name(&mut self, artist_name: &str, cx: &mut Context<Self>) {
        let want = normalize_artist_key(artist_name);
        if want.is_empty() {
            return;
        }
        self.section = NavSection::Artists;
        self.selected_local_album = None;
        if let Some(hit) = self
            .local_library
            .artists
            .iter()
            .find(|a| a.key == want || normalize_artist_key(&a.name) == want)
        {
            self.selected_local_artist = Some(hit.key.clone());
        } else {
            self.selected_local_artist = Some(want);
        }
        cx.notify();
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

        let albums_dir = self.config.albums_dir.clone();
        let format = self.config.save_audio_format;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            run_save_song(
                &albums_dir,
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

    pub fn retry_failed_download(&mut self, failed_id: String, cx: &mut Context<Self>) {
        let Some(item) = self
            .state
            .failed_downloads
            .iter()
            .find(|f| f.id == failed_id)
            .cloned()
        else {
            return;
        };
        if item.url.is_none() {
            let parsed = ParsedTrack {
                artist: (!item.artist.trim().is_empty()).then(|| item.artist.clone()),
                title: item.title.clone(),
                display: format!("{} - {}", item.artist, item.title),
            };
            self.state.failed_downloads.retain(|f| f.id != failed_id);
            self.persist_state();
            self.enqueue_save_job(parsed, None, String::new(), cx);
            return;
        }
        self.show_toast(format!("Retrying {}…", item.title), cx);
        let albums_dir = self.config.albums_dir.clone();
        let format = self.config.save_audio_format;
        let tx = self.bg_tx.clone();
        let fid = item.id.clone();
        std::thread::spawn(move || {
            let res = run_retry_failed_item(&item, &albums_dir, format).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::FailedItemRetried { id: fid, res });
        });
        cx.notify();
    }

    pub fn retry_all_failed_downloads(&mut self, cx: &mut Context<Self>) {
        let items = self.state.failed_downloads.clone();
        if items.is_empty() {
            return;
        }
        self.show_toast(format!("Retrying {} failed downloads…", items.len()), cx);
        let albums_dir = self.config.albums_dir.clone();
        let format = self.config.save_audio_format;
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            for item in items {
                let fid = item.id.clone();
                let res =
                    run_retry_failed_item(&item, &albums_dir, format).map_err(|e| format!("{e}"));
                let _ = tx.send_blocking(CoreBgEvent::FailedItemRetried { id: fid, res });
            }
        });
        cx.notify();
    }

    pub fn dismiss_failed_download(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.failed_downloads.retain(|f| f.id != id);
        self.persist_state();
        self.sync_integrations();
        cx.notify();
    }

    pub fn clear_failed_downloads(&mut self, cx: &mut Context<Self>) {
        self.state.failed_downloads.clear();
        self.persist_state();
        self.sync_integrations();
        cx.notify();
    }

    fn record_failed_downloads(&mut self, items: Vec<FailedDownloadItem>) {
        if items.is_empty() {
            return;
        }
        for item in items {
            self.state
                .failed_downloads
                .retain(|f| !(f.artist.eq_ignore_ascii_case(&item.artist) && f.title.eq_ignore_ascii_case(&item.title)));
            self.state.failed_downloads.insert(0, item);
        }
        self.state.failed_downloads.truncate(150);
        self.persist_state();
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

    pub fn toggle_clean_editions(&mut self, cx: &mut Context<Self>) {
        self.config.clean_editions = !self.config.clean_editions;
        self.persist_config();
        self.show_toast(
            if self.config.clean_editions {
                "Clean Editions: On (skips live/demo/box-set bloat)"
            } else {
                "Clean Editions: Off (shows all editions)"
            },
            cx,
        );
        cx.notify();
    }

    pub fn reorganize_local_library(&mut self, cx: &mut Context<Self>) {
        if self.reorg_running {
            return;
        }
        self.reorg_running = true;
        self.show_toast("Reorganizing ~/Music & backing up duplicates…", cx);
        let albums_dir = self.config.albums_dir.clone();
        let radio_dir = self.config.music_dir.clone();
        let backup_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("vibes-reorg-backup");
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let res = reorganize_music_library(&albums_dir, &radio_dir, &backup_dir)
                .map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::ReorgCompleted(res));
        });
        cx.notify();
    }

    // ---- Liked Tracks & User Playlists ----

    pub fn is_track_liked(&self, path: &Path) -> bool {
        self.state.liked_tracks.iter().any(|p| p == path)
    }

    pub fn toggle_like_track(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(pos) = self.state.liked_tracks.iter().position(|p| p == &path) {
            self.state.liked_tracks.remove(pos);
            self.show_toast("Removed from Liked Songs", cx);
        } else {
            self.state.liked_tracks.insert(0, path);
            self.show_toast("Added to Liked Songs", cx);
        }
        self.persist_state();
        cx.notify();
    }

    pub fn create_playlist(&mut self, raw_name: String, cx: &mut Context<Self>) {
        let name = raw_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let id = format!(
            "pl-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        self.state.playlists.push(UserPlaylist {
            id: id.clone(),
            name: name.clone(),
            track_paths: Vec::new(),
        });
        self.selected_playlist_id = Some(id);
        self.persist_state();
        self.show_toast(format!("Created playlist '{name}'"), cx);
        cx.notify();
    }

    pub fn delete_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.playlists.retain(|p| p.id != id);
        if self.selected_playlist_id.as_deref() == Some(id) {
            self.selected_playlist_id = None;
        }
        self.persist_state();
        cx.notify();
    }

    pub fn add_track_to_playlist(&mut self, playlist_id: &str, path: PathBuf, cx: &mut Context<Self>) {
        let Some(pl) = self.state.playlists.iter_mut().find(|p| p.id == playlist_id) else {
            return;
        };
        if !pl.track_paths.contains(&path) {
            pl.track_paths.push(path);
            let name = pl.name.clone();
            self.persist_state();
            self.show_toast(format!("Added to '{name}'"), cx);
        } else {
            self.show_toast("Track is already in playlist", cx);
        }
        cx.notify();
    }

    pub fn remove_track_from_playlist(&mut self, playlist_id: &str, idx: usize, cx: &mut Context<Self>) {
        if let Some(pl) = self.state.playlists.iter_mut().find(|p| p.id == playlist_id)
            && idx < pl.track_paths.len()
        {
            pl.track_paths.remove(idx);
            self.persist_state();
            cx.notify();
        }
    }

    pub fn save_queue_as_playlist(&mut self, raw_name: String, cx: &mut Context<Self>) {
        let mut paths = Vec::new();
        if let Some(PlayTarget::Local(t)) = &self.current_target {
            paths.push(t.path.clone());
        }
        for q in &self.queue {
            if let QueueItem::Local(t) = q
                && !paths.contains(&t.path)
            {
                paths.push(t.path.clone());
            }
        }
        if paths.is_empty() {
            self.show_toast("No offline tracks in current queue to save", cx);
            return;
        }
        let name = if raw_name.trim().is_empty() {
            format!("Mix {}", Local::now().format("%b %d %H:%M"))
        } else {
            raw_name.trim().to_string()
        };
        let id = format!(
            "pl-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        self.state.playlists.push(UserPlaylist {
            id: id.clone(),
            name: name.clone(),
            track_paths: paths,
        });
        self.selected_playlist_id = Some(id);
        self.persist_state();
        self.show_toast(format!("Saved queue as '{name}'"), cx);
        cx.notify();
    }

    pub fn resolve_tracks_by_paths(&self, paths: &[PathBuf]) -> Vec<LocalTrack> {
        let mut by_path: HashMap<&Path, &LocalTrack> = HashMap::new();
        for alb in &self.local_library.albums {
            for t in &alb.tracks {
                by_path.insert(t.path.as_path(), t);
            }
        }
        for s in &self.local_library.singles {
            by_path.insert(s.path.as_path(), s);
        }
        paths
            .iter()
            .filter_map(|p| by_path.get(p.as_path()).map(|t| (*t).clone()))
            .collect()
    }

    pub fn play_liked_songs(&mut self, start_idx: usize, shuffled: bool, cx: &mut Context<Self>) {
        let tracks = self.resolve_tracks_by_paths(&self.state.liked_tracks);
        if shuffled {
            self.play_tracks_shuffled(tracks, cx);
        } else {
            self.play_local_list_from(&tracks, start_idx, cx);
        }
    }

    pub fn play_playlist(
        &mut self,
        playlist_id: &str,
        start_idx: usize,
        shuffled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(paths) = self
            .state
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
            .map(|p| p.track_paths.clone())
        else {
            return;
        };
        let tracks = self.resolve_tracks_by_paths(&paths);
        if shuffled {
            self.play_tracks_shuffled(tracks, cx);
        } else {
            self.play_local_list_from(&tracks, start_idx, cx);
        }
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

    pub fn set_album_sort(&mut self, sort: AlbumSortMode, cx: &mut Context<Self>) {
        if self.album_sort != sort {
            self.album_sort = sort;
            cx.notify();
        }
    }

    pub fn select_local_artist(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        self.selected_local_artist = key;
        self.selected_local_album = None;
        cx.notify();
    }

    pub fn select_local_album(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.selected_local_album = folder;
        cx.notify();
    }

    // ---- Artist Discovery, Follow & Auto-Sync ----

    pub fn followed_entry_for_name(&self, name: &str) -> Option<&FollowedArtist> {
        let want_key = normalize_artist_key(name);
        self.state
            .followed_artists
            .iter()
            .find(|f| !want_key.is_empty() && normalize_artist_key(&f.name) == want_key)
    }

    pub fn search_artists_query(&mut self, query: String, cx: &mut Context<Self>) {
        let q = query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.artist_query = q.clone();
        self.selected_online_artist = None;
        self.artist_loading = true;
        self.artist_error = None;
        self.artist_fetch_gen = self.artist_fetch_gen.wrapping_add(1);
        let gen_id = self.artist_fetch_gen;
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let res = search_artists(&q).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::ArtistsSearched { gen_id, res });
        });
        cx.notify();
    }

    pub fn select_online_artist(&mut self, summary: ArtistSummary, cx: &mut Context<Self>) {
        self.online_artist_loading = true;
        self.artist_error = None;
        self.online_artist_gen = self.online_artist_gen.wrapping_add(1);
        let gen_id = self.online_artist_gen;
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let res = fetch_artist_discography(&summary).map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::ArtistDiscographyLoaded { gen_id, res });
        });
        cx.notify();
    }

    pub fn clear_selected_online_artist(&mut self, cx: &mut Context<Self>) {
        self.selected_online_artist = None;
        self.online_artist_loading = false;
        cx.notify();
    }

    pub fn follow_artist_summary(&mut self, summary: ArtistSummary, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .state
            .followed_artists
            .iter()
            .find(|f| f.browse_id == summary.browse_id)
            .cloned()
        {
            self.enqueue_artist_sync(existing, cx);
            return;
        }
        let followed = FollowedArtist {
            browse_id: summary.browse_id.clone(),
            name: summary.name.clone(),
            avatar_url: summary.avatar_url.clone(),
            include_singles: true,
            clean_editions: self.config.clean_editions,
            last_synced_at: None,
        };
        if let Some(ref url) = followed.avatar_url {
            self.prefetch_art_url(url, COVER_MAX_DIM);
        }
        self.state.followed_artists.push(followed.clone());
        self.persist_state();
        self.show_toast(format!("Following {} · Starting auto-sync…", followed.name), cx);
        self.enqueue_artist_sync(followed, cx);
        cx.notify();
    }

    pub fn follow_local_artist(&mut self, artist_name: String, cx: &mut Context<Self>) {
        if let Some(existing) = self.followed_entry_for_name(&artist_name).cloned() {
            self.enqueue_artist_sync(existing, cx);
            return;
        }
        self.show_toast(format!("Finding {artist_name} on YouTube Music…"), cx);
        let name = artist_name.clone();
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let res = search_artists(&name)
                .map(|list| list.into_iter().next())
                .map_err(|e| format!("{e}"));
            let _ = tx.send_blocking(CoreBgEvent::FollowArtistResolved { name, res });
        });
    }

    pub fn unfollow_artist(&mut self, browse_id_or_name: &str, cx: &mut Context<Self>) {
        let want_key = normalize_artist_key(browse_id_or_name);
        self.state.followed_artists.retain(|f| {
            f.browse_id != browse_id_or_name && normalize_artist_key(&f.name) != want_key
        });
        self.persist_state();
        self.show_toast("Unfollowed artist", cx);
        cx.notify();
    }

    pub fn sync_all_followed_artists(&mut self, cx: &mut Context<Self>) {
        if self.state.followed_artists.is_empty() {
            self.show_toast("No followed artists yet · Follow an artist first", cx);
            return;
        }
        let list = self.state.followed_artists.clone();
        for f in list {
            if !self
                .artist_sync_queue
                .iter()
                .any(|q| q.browse_id == f.browse_id)
            {
                self.artist_sync_queue.push_back(f);
            }
        }
        self.start_next_artist_sync(cx);
    }

    pub fn enqueue_artist_sync(&mut self, artist: FollowedArtist, cx: &mut Context<Self>) {
        if !self
            .artist_sync_queue
            .iter()
            .any(|q| q.browse_id == artist.browse_id)
        {
            self.artist_sync_queue.push_back(artist);
        }
        self.start_next_artist_sync(cx);
    }

    fn start_next_artist_sync(&mut self, cx: &mut Context<Self>) {
        if self.artist_sync.is_active() {
            return;
        }
        let Some(mut next) = self.artist_sync_queue.pop_front() else {
            cx.notify();
            return;
        };
        next.clean_editions = self.config.clean_editions;
        let cancel = Arc::new(AtomicBool::new(false));
        self.artist_sync_cancel = Some(Arc::clone(&cancel));
        self.artist_sync = ArtistSyncStatus::FetchingDiscography {
            artist: next.name.clone(),
        };
        let albums_dir = self.config.albums_dir.clone();
        let format = self.config.save_audio_format;
        let tx = self.bg_tx.clone();
        let browse_id = next.browse_id.clone();

        std::thread::spawn(move || {
            let (status, failures) = run_artist_sync(&albums_dir, format, &next, cancel, |st| {
                let _ = tx.send_blocking(CoreBgEvent::ArtistSyncProgress(st));
            });
            let _ = tx.send_blocking(CoreBgEvent::ArtistSyncFinished {
                browse_id,
                status,
                failures,
            });
        });
        self.sync_integrations();
        cx.notify();
    }

    pub fn cancel_artist_sync(&mut self, cx: &mut Context<Self>) {
        self.artist_sync_queue.clear();
        if let Some(ref cancel) = self.artist_sync_cancel {
            cancel.store(true, Ordering::Release);
            self.show_toast("Cancelling artist sync…", cx);
        }
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

        let resolve_item = |idx: usize, tr: &AlbumTrack| -> QueueItem {
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
            if idx >= start_idx
                && let Some(u) = youtube_thumb_url(&tr.video_id)
            {
                self.prefetch_art_url(&u, COVER_MAX_DIM);
            }
            QueueItem::Youtube(Self::track_to_yt_item(tr))
        };

        let mut all_items: Vec<QueueItem> = details
            .tracks
            .iter()
            .enumerate()
            .map(|(i, tr)| resolve_item(i, tr))
            .collect();
        let mut tail = all_items.split_off(start_idx);
        if tail.is_empty() {
            return;
        }
        let first = tail.remove(0);
        let first_local = first.is_local();
        let all_local = first_local && tail.iter().all(QueueItem::is_local);
        self.queue_history = all_items;
        self.queue = tail;
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
        let clean_editions = self.config.clean_editions;
        let tx = self.bg_tx.clone();

        std::thread::spawn(move || {
            let (status, failures) =
                run_download_album(&albums_dir, mode, format, &details, clean_editions, |st| {
                    let _ = tx.send_blocking(CoreBgEvent::AlbumDownloadProgress {
                        job_id,
                        status: st,
                    });
                });
            let _ = tx.send_blocking(CoreBgEvent::AlbumDownloadFinished {
                job_id,
                status,
                failures,
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
                            let seed = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.subsec_nanos() as usize)
                                .unwrap_or(0);
                            if let Some(next) = step_queue_advance(
                                &self.current_target,
                                &mut self.queue_history,
                                &mut self.queue,
                                self.state.shuffle_enabled,
                                self.state.repeat_mode,
                                true,
                                seed,
                            ) {
                                self.persist_queue();
                                self.play_queue_item(next, cx);
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
                if let Some(job) = self.album_downloads.iter_mut().find(|j| j.id == job_id) {
                    job.status = status;
                }
                self.sync_integrations();
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::AlbumDownloadFinished {
                job_id,
                status,
                failures,
            } => {
                let title = self
                    .album_downloads
                    .iter()
                    .find(|j| j.id == job_id)
                    .map(|j| j.title.clone())
                    .unwrap_or_default();
                if let Some(job) = self.album_downloads.iter_mut().find(|j| j.id == job_id) {
                    job.status = status.clone();
                }
                if !failures.is_empty() {
                    self.record_failed_downloads(failures);
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
            CoreBgEvent::ArtistsSearched { gen_id, res } => {
                if gen_id != self.artist_fetch_gen {
                    return;
                }
                self.artist_loading = false;
                match res {
                    Ok(list) => {
                        self.artist_error = None;
                        for a in &list {
                            if let Some(ref url) = a.avatar_url {
                                self.prefetch_art_url(url, COVER_MAX_DIM);
                            }
                        }
                        self.artist_results = list;
                    }
                    Err(err) => {
                        self.artist_error = Some(err);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::ArtistDiscographyLoaded { gen_id, res } => {
                if gen_id != self.online_artist_gen {
                    return;
                }
                self.online_artist_loading = false;
                match res {
                    Ok(disco) => {
                        self.artist_error = None;
                        if let Some(ref url) = disco.artist.avatar_url {
                            self.prefetch_art_url(url, COVER_MAX_DIM);
                        }
                        for r in disco.albums.iter().chain(disco.singles_and_eps.iter()) {
                            if let Some(ref url) = r.cover_url {
                                self.prefetch_art_url(url, COVER_MAX_DIM);
                            }
                        }
                        self.selected_online_artist = Some(disco);
                    }
                    Err(err) => {
                        self.artist_error = Some(err);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::ArtistSyncProgress(st) => {
                self.artist_sync = st;
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::ArtistSyncFinished {
                browse_id,
                status,
                failures,
            } => {
                self.artist_sync_cancel = None;
                self.artist_sync = status.clone();
                if let Some(f) = self
                    .state
                    .followed_artists
                    .iter_mut()
                    .find(|x| x.browse_id == browse_id)
                {
                    f.last_synced_at = Some(Local::now().format("%Y-%m-%d %H:%M").to_string());
                }
                if !failures.is_empty() {
                    self.record_failed_downloads(failures);
                } else {
                    self.persist_state();
                }
                self.show_toast(status.summary_line(), cx);
                self.refresh_local_library(cx);
                self.sync_integrations();
                if !self.artist_sync_queue.is_empty()
                    && !matches!(status, ArtistSyncStatus::Cancelled { .. })
                {
                    self.start_next_artist_sync(cx);
                } else if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::FollowArtistResolved { name, res } => match res {
                Ok(Some(summary)) => {
                    self.follow_artist_summary(summary, cx);
                }
                Ok(None) => {
                    self.show_toast(format!("Could not find artist '{name}' on YouTube Music"), cx);
                }
                Err(err) => {
                    self.show_toast(format!("Artist search failed: {err}"), cx);
                }
            },
            CoreBgEvent::FailedItemRetried { id, res } => {
                match res {
                    Ok(path) => {
                        self.state.failed_downloads.retain(|f| f.id != id);
                        self.persist_state();
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("Track");
                        self.show_toast(format!("Recovered: {name}"), cx);
                        self.refresh_local_library(cx);
                    }
                    Err(err) => {
                        if let Some(item) =
                            self.state.failed_downloads.iter_mut().find(|f| f.id == id)
                        {
                            item.error = err.clone();
                            item.failed_at = Local::now().format("%Y-%m-%d %H:%M").to_string();
                        }
                        self.persist_state();
                        self.show_toast(format!("Retry failed: {err}"), cx);
                    }
                }
                self.sync_integrations();
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::ReorgCompleted(res) => {
                self.reorg_running = false;
                match res {
                    Ok(report) => {
                        self.show_toast(report.summary(), cx);
                        self.refresh_local_library(cx);
                    }
                    Err(err) => {
                        self.show_toast(format!("Library organize error: {err}"), cx);
                    }
                }
                if self.window_open {
                    cx.notify();
                }
            }
            CoreBgEvent::StreamUrlPrefetched { video_id, res } => {
                self.prefetching_video_ids.remove(&video_id);
                if let Ok(cdn_url) = res {
                    if self.stream_url_cache.len() >= 32 {
                        self.stream_url_cache.clear();
                    }
                    self.stream_url_cache
                        .insert(video_id, (cdn_url, Instant::now()));
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
                if let Some(PlayTarget::Local(ref mut cur)) = self.current_target {
                    if cur.cover_path.is_none() {
                        cur.cover_path = snapshot
                            .albums
                            .iter()
                            .flat_map(|a| a.tracks.iter())
                            .chain(snapshot.singles.iter())
                            .find(|t| t.path == cur.path)
                            .and_then(|t| t.cover_path.clone());
                    }
                    if gen_id == 1 && self.queue.is_empty() && self.queue_history.is_empty() {
                        for album in &snapshot.albums {
                            if let Some(idx) = album.tracks.iter().position(|t| t.path == cur.path)
                            {
                                self.queue_history = album.tracks[..idx]
                                    .iter()
                                    .cloned()
                                    .map(QueueItem::Local)
                                    .collect();
                                self.queue = album.tracks[idx + 1..]
                                    .iter()
                                    .cloned()
                                    .map(QueueItem::Local)
                                    .collect();
                                break;
                            }
                        }
                    }
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
                        if let Some(job) = self.downloads.iter().find(|j| j.id == job_id).cloned() {
                            let dest_dir = self.config.albums_dir.clone();
                            self.record_failed_downloads(vec![FailedDownloadItem {
                                id: format!("sng-{job_id}"),
                                kind: FailedDownloadKind::SingleTrack,
                                artist: job.parsed.artist.clone().unwrap_or_else(|| "Unknown Artist".to_string()),
                                album: None,
                                title: job.parsed.title.clone(),
                                track_number: None,
                                url: job.direct_url.clone(),
                                dest_dir,
                                error: err.clone(),
                                failed_at: Local::now().format("%Y-%m-%d %H:%M").to_string(),
                            }]);
                        }
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

const MAX_QUEUE_HISTORY: usize = 200;
const PREV_RESTART_THRESHOLD_SECS: f64 = 3.0;
const PREV_DOUBLE_CLICK_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
enum PreviousStep {
    SeekToStart,
    PlayTrack(QueueItem),
    CycleFavorite,
}

fn target_as_queue_item(target: &Option<PlayTarget>) -> Option<QueueItem> {
    match target {
        Some(PlayTarget::Youtube(y)) => Some(QueueItem::Youtube(y.clone())),
        Some(PlayTarget::Local(t)) => Some(QueueItem::Local(t.clone())),
        _ => None,
    }
}

fn push_queue_history(history: &mut Vec<QueueItem>, item: QueueItem) {
    history.push(item);
    if history.len() > MAX_QUEUE_HISTORY {
        let excess = history.len() - MAX_QUEUE_HISTORY;
        history.drain(..excess);
    }
}

fn shuffle_slice<T>(items: &mut [T]) {
    if items.len() <= 1 {
        return;
    }
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    if state == 0 {
        state = 0x517c_c1b7_2722_0a95;
    }
    for i in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state as usize) % (i + 1);
        items.swap(i, j);
    }
}

fn step_queue_advance(
    current: &Option<PlayTarget>,
    history: &mut Vec<QueueItem>,
    queue: &mut Vec<QueueItem>,
    shuffle_enabled: bool,
    repeat_mode: RepeatMode,
    is_eof: bool,
    seed: usize,
) -> Option<QueueItem> {
    if is_eof
        && repeat_mode == RepeatMode::One
        && let Some(cur) = target_as_queue_item(current)
    {
        return Some(cur);
    }

    if !queue.is_empty() {
        if let Some(cur) = target_as_queue_item(current) {
            push_queue_history(history, cur);
        }
        let pick_idx = if shuffle_enabled {
            seed % queue.len()
        } else {
            0
        };
        return Some(queue.remove(pick_idx));
    }

    if repeat_mode == RepeatMode::All {
        let mut pool = Vec::new();
        pool.append(history);
        if let Some(cur) = target_as_queue_item(current) {
            pool.push(cur);
        }
        if !pool.is_empty() {
            if shuffle_enabled {
                shuffle_slice(&mut pool);
            }
            let first = pool.remove(0);
            *queue = pool;
            return Some(first);
        }
    }

    None
}

#[cfg(test)]
fn step_queue_next(
    current: &Option<PlayTarget>,
    history: &mut Vec<QueueItem>,
    queue: &mut Vec<QueueItem>,
) -> Option<QueueItem> {
    step_queue_advance(current, history, queue, false, RepeatMode::Off, false, 0)
}

fn step_queue_previous(
    current: &Option<PlayTarget>,
    position_secs: f64,
    rapid_second_click: bool,
    history: &mut Vec<QueueItem>,
    queue: &mut Vec<QueueItem>,
) -> PreviousStep {
    if matches!(
        current,
        Some(PlayTarget::Youtube(_) | PlayTarget::Local(_))
    ) {
        if position_secs > PREV_RESTART_THRESHOLD_SECS && !rapid_second_click {
            return PreviousStep::SeekToStart;
        }
        if let Some(prev) = history.pop() {
            if let Some(cur) = target_as_queue_item(current) {
                queue.insert(0, cur);
            }
            return PreviousStep::PlayTrack(prev);
        }
        return PreviousStep::SeekToStart;
    }
    PreviousStep::CycleFavorite
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

    fn dummy_track(n: u32) -> LocalTrack {
        LocalTrack {
            path: PathBuf::from(format!("/music/Album/{n:02} Track {n}.opus")),
            title: format!("Track {n}"),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            track_number: Some(n),
            duration_secs: Some(180.0),
            format: "OPUS".to_string(),
            cover_path: None,
        }
    }

    fn as_target(item: &QueueItem) -> Option<PlayTarget> {
        match item {
            QueueItem::Youtube(y) => Some(PlayTarget::Youtube(y.clone())),
            QueueItem::Local(t) => Some(PlayTarget::Local(t.clone())),
        }
    }

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

    #[test]
    fn album_forward_and_back_preserves_track_order() {
        let t1 = QueueItem::Local(dummy_track(1));
        let t2 = QueueItem::Local(dummy_track(2));
        let t3 = QueueItem::Local(dummy_track(3));
        let mut current = as_target(&t1);
        let mut history = Vec::new();
        let mut queue = vec![t2.clone(), t3.clone()];

        // Forward 1 -> 2 -> 3
        let next2 = step_queue_next(&current, &mut history, &mut queue).unwrap();
        assert_eq!(next2, t2);
        current = as_target(&next2);

        let next3 = step_queue_next(&current, &mut history, &mut queue).unwrap();
        assert_eq!(next3, t3);
        current = as_target(&next3);
        assert!(queue.is_empty());
        assert_eq!(history, vec![t1.clone(), t2.clone()]);

        // Back 3 -> 2 -> 1
        let prev2 = match step_queue_previous(&current, 1.0, false, &mut history, &mut queue) {
            PreviousStep::PlayTrack(it) => it,
            other => panic!("expected PlayTrack(t2), got {other:?}"),
        };
        assert_eq!(prev2, t2);
        current = as_target(&prev2);
        assert_eq!(queue, vec![t3.clone()]);

        let prev1 = match step_queue_previous(&current, 0.5, false, &mut history, &mut queue) {
            PreviousStep::PlayTrack(it) => it,
            other => panic!("expected PlayTrack(t1), got {other:?}"),
        };
        assert_eq!(prev1, t1);
        current = as_target(&prev1);
        assert!(history.is_empty());
        assert_eq!(queue, vec![t2.clone(), t3.clone()]);

        // Forward again 1 -> 2 -> 3
        let again2 = step_queue_next(&current, &mut history, &mut queue).unwrap();
        assert_eq!(again2, t2);
        current = as_target(&again2);
        let again3 = step_queue_next(&current, &mut history, &mut queue).unwrap();
        assert_eq!(again3, t3);
        assert!(queue.is_empty());
        assert_eq!(history, vec![t1, t2]);
    }

    #[test]
    fn mid_album_back_steps_to_start_and_never_cycles_favorites() {
        let t1 = QueueItem::Local(dummy_track(1));
        let t2 = QueueItem::Local(dummy_track(2));
        let t3 = QueueItem::Local(dummy_track(3));
        let t4 = QueueItem::Local(dummy_track(4));

        let mut current = as_target(&t3);
        let mut history = vec![t1.clone(), t2.clone()];
        let mut queue = vec![t4.clone()];

        let back2 = match step_queue_previous(&current, 0.0, false, &mut history, &mut queue) {
            PreviousStep::PlayTrack(it) => it,
            other => panic!("expected t2, got {other:?}"),
        };
        assert_eq!(back2, t2);
        current = as_target(&back2);

        let back1 = match step_queue_previous(&current, 0.0, false, &mut history, &mut queue) {
            PreviousStep::PlayTrack(it) => it,
            other => panic!("expected t1, got {other:?}"),
        };
        assert_eq!(back1, t1);
        current = as_target(&back1);
        assert!(history.is_empty());
        assert_eq!(queue, vec![t2, t3, t4]);

        // At track 1 with empty history, pressing Back restarts track 1 rather than jumping to radio.
        assert_eq!(
            step_queue_previous(&current, 1.0, false, &mut history, &mut queue),
            PreviousStep::SeekToStart
        );
        assert_eq!(
            step_queue_previous(&current, 20.0, true, &mut history, &mut queue),
            PreviousStep::SeekToStart
        );
    }

    #[test]
    fn previous_threshold_and_rapid_double_click_behave_cleanly() {
        let t1 = QueueItem::Local(dummy_track(1));
        let t2 = QueueItem::Local(dummy_track(2));
        let current = as_target(&t2);
        let mut history = vec![t1.clone()];
        let mut queue = Vec::new();

        // Single click past 3s restarts current track without disturbing history or queue.
        assert_eq!(
            step_queue_previous(&current, 25.0, false, &mut history, &mut queue),
            PreviousStep::SeekToStart
        );
        assert_eq!(history, vec![t1.clone()]);
        assert!(queue.is_empty());

        // Rapid second click (even if mpv IPC still reports 25.0s) steps back to t1 and re-queues t2.
        assert_eq!(
            step_queue_previous(&current, 25.0, true, &mut history, &mut queue),
            PreviousStep::PlayTrack(t1)
        );
        assert!(history.is_empty());
        assert_eq!(queue, vec![t2]);
    }

    #[test]
    fn radio_and_empty_target_cycle_favorites_on_previous() {
        let radio = RadioStation {
            stationuuid: "u1".to_string(),
            name: "Station".to_string(),
            url: "https://example.com/stream".to_string(),
            url_resolved: String::new(),
            favicon: String::new(),
            tags: String::new(),
            country: String::new(),
            countrycode: String::new(),
            codec: String::new(),
            bitrate: 128,
            votes: 0,
            clickcount: 0,
            hls: 0,
        };
        let mut history = Vec::new();
        let mut queue = Vec::new();
        assert_eq!(
            step_queue_previous(
                &Some(PlayTarget::Radio(radio)),
                10.0,
                false,
                &mut history,
                &mut queue
            ),
            PreviousStep::CycleFavorite
        );
        assert_eq!(
            step_queue_previous(&None, 0.0, false, &mut history, &mut queue),
            PreviousStep::CycleFavorite
        );
    }

    #[test]
    fn shuffle_and_repeat_modes_behave_correctly() {
        let t1 = QueueItem::Local(dummy_track(1));
        let t2 = QueueItem::Local(dummy_track(2));
        let t3 = QueueItem::Local(dummy_track(3));
        let current = as_target(&t1);
        let mut history = Vec::new();
        let mut queue = vec![t2.clone(), t3.clone()];

        // Repeat One replays current track on EOF without draining queue
        let rep1 = step_queue_advance(
            &current,
            &mut history,
            &mut queue,
            false,
            RepeatMode::One,
            true,
            0,
        );
        assert_eq!(rep1, Some(t1.clone()));
        assert_eq!(queue.len(), 2);

        // Shuffle picks within queue using seed
        let shuf = step_queue_advance(
            &current,
            &mut history,
            &mut queue,
            true,
            RepeatMode::Off,
            true,
            1,
        );
        assert_eq!(shuf, Some(t3.clone()));
        assert_eq!(queue, vec![t2.clone()]);

        // Repeat All recycles history + current when queue empties
        history.push(t2.clone());
        let mut empty_q = Vec::new();
        let wrapped = step_queue_advance(
            &as_target(&t3),
            &mut history,
            &mut empty_q,
            false,
            RepeatMode::All,
            true,
            0,
        );
        assert_eq!(wrapped, Some(t1));
        assert_eq!(empty_q.len(), 2);
    }

    #[test]
    fn push_queue_history_caps_at_max_entries() {
        let mut history = Vec::new();
        for n in 1..=(MAX_QUEUE_HISTORY as u32 + 5) {
            push_queue_history(&mut history, QueueItem::Local(dummy_track(n)));
        }
        assert_eq!(history.len(), MAX_QUEUE_HISTORY);
        assert_eq!(history.first(), Some(&QueueItem::Local(dummy_track(6))));
        assert_eq!(
            history.last(),
            Some(&QueueItem::Local(dummy_track(MAX_QUEUE_HISTORY as u32 + 5)))
        );
    }
}
