use crate::albums::{
    AlbumSummary, AlbumTrack, best_thumbnail_url, download_album_tracks_with_failures,
    extract_subtitle_parts, fetch_album_details, fetch_square_cover_jpg, join_runs, post_ytm_json,
    walk_json, ytm_context,
};
use crate::config::{
    AlbumFolderMode, FailedDownloadItem, FailedDownloadKind, FollowedArtist, SaveAudioFormat,
};
use crate::downloader::{
    TrackTagSpec, extract_ytdlp_audio_with_retry, sanitize_filename, write_tags_and_cover,
};
use crate::local_library::{artist_has_track, collect_artist_track_keys, normalize_match_key};
use anyhow::{Context, Result};
use chrono::Local;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const YTM_SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
const YTM_BROWSE_URL: &str = "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false";
const YTM_ARTIST_SEARCH_PARAMS: &str = "EgWKAQIgAWoKEAMQBBAJEAoQBQ==";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistSummary {
    pub browse_id: String,
    pub name: String,
    pub subtitle: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtistDiscography {
    pub artist: ArtistSummary,
    pub albums: Vec<AlbumSummary>,
    pub singles_and_eps: Vec<AlbumSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ArtistSyncStatus {
    #[default]
    Idle,
    FetchingDiscography {
        artist: String,
    },
    Syncing {
        artist: String,
        release_idx: usize,
        release_total: usize,
        current_release: String,
        current_track: String,
        saved: usize,
        skipped: usize,
        failed: usize,
    },
    Completed {
        artist: String,
        saved: usize,
        skipped: usize,
        failed: usize,
    },
    Cancelled {
        artist: String,
        saved: usize,
        failed: usize,
    },
    Failed {
        artist: String,
        error: String,
    },
}

impl ArtistSyncStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::FetchingDiscography { .. } | Self::Syncing { .. }
        )
    }

    pub fn summary_line(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::FetchingDiscography { artist } => {
                format!("Fetching discography for {artist}…")
            }
            Self::Syncing {
                artist,
                release_idx,
                release_total,
                current_release,
                current_track,
                saved,
                skipped,
                failed,
            } => format!(
                "Syncing {artist} ({release_idx}/{release_total}): {current_release} — {current_track} · {saved} saved, {skipped} skipped, {failed} failed"
            ),
            Self::Completed {
                artist,
                saved,
                skipped,
                failed,
            } => format!(
                "Synced {artist}: {saved} saved, {skipped} skipped, {failed} failed"
            ),
            Self::Cancelled {
                artist,
                saved,
                failed,
            } => format!("Cancelled {artist} sync · {saved} saved, {failed} failed"),
            Self::Failed { artist, error } => format!("Sync failed for {artist}: {error}"),
        }
    }
}

/// Normalizes an artist name for folder deduplication, ignoring case, punctuation, and leading `"the "`.
pub fn normalize_artist_key(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_lowercase();
    let stripped = lower.strip_prefix("the ").unwrap_or(&lower);
    normalize_match_key(stripped)
}

/// Resolves the canonical `<albums_dir>/<Artist>` folder, matching existing case/prefix variants if present.
pub fn find_canonical_artist_dir(albums_dir: &Path, artist: &str) -> PathBuf {
    let clean = sanitize_filename(artist);
    let exact = albums_dir.join(&clean);
    if exact.is_dir() {
        return exact;
    }

    let want = normalize_artist_key(artist);
    if !want.is_empty()
        && let Ok(entries) = fs::read_dir(albums_dir)
    {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name.eq_ignore_ascii_case("radio") {
                continue;
            }
            if normalize_artist_key(name) == want {
                return p;
            }
        }
    }

    exact
}

pub fn parse_ytm_artist_search_json(root: &serde_json::Value) -> Vec<ArtistSummary> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    walk_json(root, &mut |node| {
        let Some(r) = node.get("musicResponsiveListItemRenderer") else {
            return;
        };
        let Some(browse_id) = r
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(|v| v.as_str())
            .filter(|id| id.starts_with("UC"))
        else {
            return;
        };
        if !seen.insert(browse_id.to_string()) {
            return;
        }
        let Some(cols) = r.get("flexColumns").and_then(|v| v.as_array()) else {
            return;
        };
        let name = cols
            .first()
            .map(|c| {
                join_runs(
                    c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                        .and_then(|v| v.as_array()),
                )
            })
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let subtitle = cols
            .get(1)
            .map(|c| {
                join_runs(
                    c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                        .and_then(|v| v.as_array()),
                )
            })
            .unwrap_or_default();
        let avatar_url = best_thumbnail_url(r);
        out.push(ArtistSummary {
            browse_id: browse_id.to_string(),
            name,
            subtitle,
            avatar_url,
        });
    });

    out
}

pub fn search_artists(query: &str) -> Result<Vec<ArtistSummary>> {
    let trimmed = query.trim();
    anyhow::ensure!(!trimmed.is_empty(), "Empty artist search query");
    let payload = json!({
        "context": ytm_context(),
        "query": trimmed,
        "params": YTM_ARTIST_SEARCH_PARAMS,
    });
    let root = post_ytm_json(YTM_SEARCH_URL, &payload)
        .context("Failed to search YouTube Music artists")?;
    Ok(parse_ytm_artist_search_json(&root))
}

fn parse_two_row_release(
    tr: &serde_json::Value,
    artist_name: &str,
    default_type: &str,
) -> Option<AlbumSummary> {
    let browse_id = tr
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())?
        .to_string();
    let title = join_runs(tr.pointer("/title/runs").and_then(|v| v.as_array()));
    if title.is_empty() {
        return None;
    }
    let sub = join_runs(tr.pointer("/subtitle/runs").and_then(|v| v.as_array()));
    let (parsed_type, _, year) = extract_subtitle_parts(&sub);
    let release_type = if sub.to_lowercase().contains("single") {
        "Single".to_string()
    } else if sub.to_lowercase().contains("ep") {
        "EP".to_string()
    } else if !parsed_type.is_empty() && parsed_type != "Album" {
        parsed_type
    } else {
        default_type.to_string()
    };
    let cover_url = best_thumbnail_url(tr).or_else(|| {
        tr.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(ToOwned::to_owned)
    });
    let playlist_id = tr
        .pointer("/thumbnailOverlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchPlaylistEndpoint/playlistId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    Some(AlbumSummary {
        browse_id,
        playlist_id,
        title,
        artist: artist_name.to_string(),
        release_type,
        year,
        cover_url,
    })
}

fn collect_two_row_releases(
    root: &serde_json::Value,
    artist_name: &str,
    default_type: &str,
) -> Vec<AlbumSummary> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk_json(root, &mut |node| {
        if let Some(tr) = node.get("musicTwoRowItemRenderer")
            && let Some(item) = parse_two_row_release(tr, artist_name, default_type)
            && seen.insert(item.browse_id.clone())
        {
            out.push(item);
        }
    });
    out
}

pub fn parse_ytm_artist_browse_releases(
    root: &serde_json::Value,
    artist_name: &str,
) -> (
    Vec<AlbumSummary>,
    Option<(String, Option<String>)>,
    Vec<AlbumSummary>,
    Option<(String, Option<String>)>,
) {
    let mut albums = Vec::new();
    let mut albums_more: Option<(String, Option<String>)> = None;
    let mut singles = Vec::new();
    let mut singles_more: Option<(String, Option<String>)> = None;

    walk_json(root, &mut |node| {
        let Some(sh) = node.get("musicCarouselShelfRenderer") else {
            return;
        };
        let hdr = sh.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
        let shelf_title = hdr
            .map(|h| join_runs(h.pointer("/title/runs").and_then(|v| v.as_array())))
            .unwrap_or_default();
        let lower = shelf_title.to_lowercase();
        let ep = hdr
            .and_then(|h| {
                h.pointer("/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint")
                    .or_else(|| h.pointer("/title/runs/0/navigationEndpoint/browseEndpoint"))
            })
            .and_then(|ep| {
                let bid = ep.get("browseId").and_then(|v| v.as_str())?.to_string();
                let params = ep
                    .get("params")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                Some((bid, params))
            });

        if lower == "albums" {
            if albums.is_empty() {
                albums = collect_two_row_releases(sh, artist_name, "Album");
            }
            if albums_more.is_none() {
                albums_more = ep;
            }
        } else if lower.contains("singles") || lower.contains("ep") {
            if singles.is_empty() {
                singles = collect_two_row_releases(sh, artist_name, "Single");
            }
            if singles_more.is_none() {
                singles_more = ep;
            }
        }
    });

    (albums, albums_more, singles, singles_more)
}

fn fetch_discography_more_shelf(
    browse_id: &str,
    params: Option<&str>,
    artist_name: &str,
    default_type: &str,
) -> Result<Vec<AlbumSummary>> {
    let payload = if let Some(p) = params {
        json!({
            "context": ytm_context(),
            "browseId": browse_id,
            "params": p,
        })
    } else {
        json!({
            "context": ytm_context(),
            "browseId": browse_id,
        })
    };
    let root = post_ytm_json(YTM_BROWSE_URL, &payload)?;
    Ok(collect_two_row_releases(&root, artist_name, default_type))
}

pub fn fetch_artist_discography(artist: &ArtistSummary) -> Result<ArtistDiscography> {
    let payload = json!({
        "context": ytm_context(),
        "browseId": artist.browse_id,
    });
    let root = post_ytm_json(YTM_BROWSE_URL, &payload)
        .with_context(|| format!("Failed to load discography for {}", artist.name))?;

    let mut artist_out = artist.clone();
    if let Some(hdr) = root.pointer("/header/musicImmersiveHeaderRenderer")
        && let Some(url) = best_thumbnail_url(hdr)
    {
        artist_out.avatar_url = Some(url);
    }

    let (mut albums, albums_more, mut singles_and_eps, singles_more) =
        parse_ytm_artist_browse_releases(&root, &artist_out.name);

    if let Some((bid, params)) = albums_more
        && let Ok(full) =
            fetch_discography_more_shelf(&bid, params.as_deref(), &artist_out.name, "Album")
        && !full.is_empty()
    {
        albums = full;
    }

    if let Some((bid, params)) = singles_more
        && let Ok(full) =
            fetch_discography_more_shelf(&bid, params.as_deref(), &artist_out.name, "Single")
        && !full.is_empty()
    {
        singles_and_eps = full;
    }

    Ok(ArtistDiscography {
        artist: artist_out,
        albums,
        singles_and_eps,
    })
}

// ---- Clean Edition & Bloat Filtering ----

const HARD_BLOAT_RELEASE_MARKERS: &[&str] = &[
    "(live",
    "[live",
    "live at ",
    "live in ",
    "live around the world",
    "live on stage",
    "live fm broadcast",
    "live & acoustic",
    "live shit",
    "live s**t",
    "unplugged",
    "in concert",
    "woodstock",
    "a cappella",
    "acapella",
    "instrumental",
    "sped up",
    "slowed",
    "reverb",
    "nightcore",
    "8d audio",
    "karaoke",
    "commentary",
    "track by track",
    "soundcheck",
    "b-sides",
    "rarities",
    "outtakes",
    "rehearsal",
    "rough mix",
    "demos",
    "(demo)",
    "[demo]",
    "film soundtrack (live",
    "tour edition",
];

pub fn is_hard_bloat_release(title: &str) -> bool {
    let lower = title.to_lowercase();
    HARD_BLOAT_RELEASE_MARKERS
        .iter()
        .any(|m| lower.contains(m))
}

pub fn edition_bloat_rank(title: &str) -> u8 {
    let lower = title.to_lowercase();
    if lower.contains("anniversary")
        || lower.contains("box set")
        || lower.contains("super deluxe")
        || lower.contains("collector")
    {
        3
    } else if lower.contains("deluxe")
        || lower.contains("expanded")
        || lower.contains("special edition")
        || lower.contains("bonus track")
        || lower.contains("(more)")
        || lower.contains("shorter, thicker")
        || lower.contains("reimagined")
    {
        2
    } else if lower.contains("remaster") || lower.contains("reissue") {
        1
    } else {
        0
    }
}

const EDITION_KEYWORDS: &[&str] = &[
    "deluxe",
    "remaster",
    "anniversary",
    "expanded",
    "special",
    "bonus",
    "box set",
    "super deluxe",
    "collector",
    "complete",
    "explicit",
    "clean",
    "more",
    "sped up",
    "version",
    "edition",
    "reissue",
    "shorter, thicker",
];

fn contains_edition_keyword(seg: &str) -> bool {
    let lower = seg.to_lowercase();
    EDITION_KEYWORDS.iter().any(|k| lower.contains(k))
}

/// Strips edition/remaster/box-set suffixes so duplicate editions map to one canonical album key.
pub fn canonical_release_key(title: &str) -> String {
    let mut s = title.trim().to_string();

    loop {
        let trimmed = s.trim_end().to_string();
        let mut changed = false;

        if let Some((open, _close)) = [('(', ')'), ('[', ']')]
            .into_iter()
            .find(|&(_, c)| trimmed.ends_with(c))
            && let Some(pos) = trimmed.rfind(open)
        {
            let inner = &trimmed[pos + 1..trimmed.len() - 1];
            if contains_edition_keyword(inner) {
                s = trimmed[..pos].trim().to_string();
                changed = true;
            }
        }

        if !changed {
            for sep in [" - ", ": "] {
                if let Some((head, tail)) = trimmed.rsplit_once(sep)
                    && !head.trim().is_empty()
                    && contains_edition_keyword(tail)
                {
                    s = head.trim().to_string();
                    changed = true;
                    break;
                }
            }
        }

        if !changed {
            // Also catch unparenthesized trailing "20th Anniversary Edition" etc.
            let lower = trimmed.to_lowercase();
            if let Some(pos) = lower.rfind(" 20th anniversary")
                .or_else(|| lower.rfind(" 25th anniversary"))
                .or_else(|| lower.rfind(" 30th anniversary"))
                .or_else(|| lower.rfind(" 40th anniversary"))
                .or_else(|| lower.rfind(" deluxe edition"))
                .or_else(|| lower.rfind(" deluxe version"))
                && pos > 0
            {
                s = trimmed[..pos].trim().to_string();
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    normalize_match_key(&s)
}

pub fn filter_clean_releases(releases: &[AlbumSummary], clean_editions: bool) -> Vec<AlbumSummary> {
    let mut by_browse = HashSet::new();
    let mut deduped = Vec::new();
    for r in releases {
        if by_browse.insert(r.browse_id.clone()) {
            deduped.push(r.clone());
        }
    }

    if !clean_editions {
        return deduped;
    }

    let mut groups: HashMap<String, (usize, AlbumSummary)> = HashMap::new();
    for (idx, r) in deduped.into_iter().enumerate() {
        if is_hard_bloat_release(&r.title) {
            continue;
        }
        let key = canonical_release_key(&r.title);
        let key = if key.is_empty() {
            r.browse_id.clone()
        } else {
            key
        };

        groups
            .entry(key)
            .and_modify(|(best_idx, best)| {
                let rank_new = edition_bloat_rank(&r.title);
                let rank_old = edition_bloat_rank(&best.title);
                let better = if rank_new != rank_old {
                    rank_new < rank_old
                } else if r.title.len() != best.title.len() {
                    r.title.len() < best.title.len()
                } else {
                    r.year.unwrap_or(u16::MAX) < best.year.unwrap_or(u16::MAX)
                };
                if better {
                    *best = r.clone();
                    // Keep the earliest slot in the list if year is unavailable, or re-sort later.
                    let _ = best_idx;
                }
            })
            .or_insert((idx, r));
    }

    let mut kept: Vec<(usize, AlbumSummary)> = groups.into_values().collect();
    kept.sort_by(|a, b| {
        b.1.year
            .unwrap_or(0)
            .cmp(&a.1.year.unwrap_or(0))
            .then_with(|| a.0.cmp(&b.0))
    });
    kept.into_iter().map(|(_, item)| item).collect()
}

const BLOAT_TRACK_MARKERS: &[&str] = &[
    "(live",
    "[live",
    "live at ",
    "live in ",
    "live - ",
    "live from ",
    "(demo",
    "[demo",
    "demo)",
    "demo]",
    "work in progress",
    "riff tape",
    "rough mix",
    "rehearsal",
    "interview",
    "commentary",
    "a cappella",
    "acapella",
    "(instrumental",
    "[instrumental",
    "instrumental)",
    "instrumental]",
    "sped up",
    "slowed",
    "soundcheck",
    "outtake",
    "lpu ",
    "underground demo",
    "radio edit",
    "studio banter",
    "tuning room",
    "remaster) - live",
    "pre-production",
    "writing session",
    "writing in progress",
    " demo ",
    "unreleased demo",
];

pub fn is_bloat_track_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    BLOAT_TRACK_MARKERS.iter().any(|m| lower.contains(m))
}

pub fn is_bloat_track(track_number: Option<u32>, title: &str) -> bool {
    if is_bloat_track_title(title) {
        return true;
    }
    let lower = title.to_lowercase();
    // Live-concert walk-on tape inserted after the core studio album (e.g. AJFA track 10)
    if track_number.is_some_and(|n| n > 1) && lower.contains("ecstasy of gold") {
        return true;
    }
    false
}

/// Removes live/demo/instrumental/bloat bonus tracks and renumbers remaining tracks sequentially.
pub fn filter_album_tracks(tracks: &[AlbumTrack], clean_editions: bool) -> (Vec<AlbumTrack>, usize) {
    if !clean_editions || tracks.is_empty() {
        return (tracks.to_vec(), 0);
    }

    let kept: Vec<AlbumTrack> = tracks
        .iter()
        .filter(|t| !is_bloat_track(Some(t.track_number), &t.title))
        .cloned()
        .collect();

    if kept.is_empty() {
        // Safety guard: never wipe an album down to zero tracks if every track is labeled "(Live)" etc.
        return (tracks.to_vec(), 0);
    }

    let removed = tracks.len() - kept.len();
    let renumbered: Vec<AlbumTrack> = kept
        .into_iter()
        .enumerate()
        .map(|(i, mut t)| {
            t.track_number = (i + 1) as u32;
            t
        })
        .collect();
    (renumbered, removed)
}

fn format_failed_at() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

pub fn run_artist_sync(
    albums_dir: &Path,
    format: SaveAudioFormat,
    cookies_browser: Option<&str>,
    followed: &FollowedArtist,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(ArtistSyncStatus),
) -> (ArtistSyncStatus, Vec<FailedDownloadItem>) {
    on_progress(ArtistSyncStatus::FetchingDiscography {
        artist: followed.name.clone(),
    });

    let summary = ArtistSummary {
        browse_id: followed.browse_id.clone(),
        name: followed.name.clone(),
        subtitle: String::new(),
        avatar_url: followed.avatar_url.clone(),
    };

    let disco = match fetch_artist_discography(&summary) {
        Ok(d) => d,
        Err(err) => {
            let st = ArtistSyncStatus::Failed {
                artist: followed.name.clone(),
                error: format!("{err}"),
            };
            on_progress(st.clone());
            return (st, Vec::new());
        }
    };

    if cancel.load(Ordering::Acquire) {
        let st = ArtistSyncStatus::Cancelled {
            artist: disco.artist.name,
            saved: 0,
            failed: 0,
        };
        on_progress(st.clone());
        return (st, Vec::new());
    }

    let artist_dir = find_canonical_artist_dir(albums_dir, &disco.artist.name);
    let _ = fs::create_dir_all(&artist_dir);

    // Collect every normalized track title already stored under `<Artist>/` so singles don't dupe albums.
    let mut seen_track_keys = collect_artist_track_keys(&artist_dir);

    let clean_albums = filter_clean_releases(&disco.albums, followed.clean_editions);
    let clean_singles = if followed.include_singles {
        filter_clean_releases(&disco.singles_and_eps, followed.clean_editions)
    } else {
        Vec::new()
    };

    let total_releases = clean_albums.len() + clean_singles.len();
    let mut total_saved = 0usize;
    let mut total_skipped = 0usize;
    let mut all_failures = Vec::new();
    let lyrics_agent = crate::lyrics::new_http_agent();

    for (rel_idx, rel) in clean_albums
        .into_iter()
        .chain(clean_singles.into_iter())
        .enumerate()
    {
        if cancel.load(Ordering::Acquire) {
            let st = ArtistSyncStatus::Cancelled {
                artist: disco.artist.name.clone(),
                saved: total_saved,
                failed: all_failures.len(),
            };
            on_progress(st.clone());
            return (st, all_failures);
        }

        let rel_one_based = rel_idx + 1;
        on_progress(ArtistSyncStatus::Syncing {
            artist: disco.artist.name.clone(),
            release_idx: rel_one_based,
            release_total: total_releases,
            current_release: rel.title.clone(),
            current_track: "Loading tracklist…".to_string(),
            saved: total_saved,
            skipped: total_skipped,
            failed: all_failures.len(),
        });

        let details = match fetch_album_details(&rel) {
            Ok(d) => d,
            Err(err) => {
                let msg = format!("Artist sync release fetch failed ({}): {err}", rel.title);
                crate::log::log_warn("discography", &msg);
                eprintln!("Vibes {msg}");
                all_failures.push(FailedDownloadItem {
                    id: format!("rel-{}", rel.browse_id),
                    kind: FailedDownloadKind::ArtistSync,
                    artist: disco.artist.name.clone(),
                    album: Some(rel.title.clone()),
                    title: format!("Release: {}", rel.title),
                    track_number: None,
                    url: rel
                        .playlist_id
                        .as_ref()
                        .map(|p| format!("https://music.youtube.com/playlist?list={p}")),
                    dest_dir: artist_dir.clone(),
                    error: format!("{err}"),
                    failed_at: format_failed_at(),
                });
                continue;
            }
        };

        let (filtered_tracks, _) = filter_album_tracks(&details.tracks, followed.clean_editions);
        let is_single_bucket = filtered_tracks.len() <= 2
            && (rel.release_type.eq_ignore_ascii_case("Single") || filtered_tracks.len() == 1);

        if is_single_bucket {
            let singles_dir = artist_dir.join("Singles");
            if let Err(err) = fs::create_dir_all(&singles_dir) {
                all_failures.push(FailedDownloadItem {
                    id: format!("dir-{}", rel.browse_id),
                    kind: FailedDownloadKind::ArtistSync,
                    artist: disco.artist.name.clone(),
                    album: Some("Singles".to_string()),
                    title: rel.title.clone(),
                    track_number: None,
                    url: None,
                    dest_dir: singles_dir,
                    error: format!("Cannot create Singles folder: {err}"),
                    failed_at: format_failed_at(),
                });
                continue;
            }

            let mut cached_cover: Option<Vec<u8>> = None;
            for tr in &filtered_tracks {
                if cancel.load(Ordering::Acquire) {
                    let st = ArtistSyncStatus::Cancelled {
                        artist: disco.artist.name.clone(),
                        saved: total_saved,
                        failed: all_failures.len(),
                    };
                    on_progress(st.clone());
                    return (st, all_failures);
                }

                let tkey = normalize_match_key(&tr.title);
                if (!tkey.is_empty() && seen_track_keys.contains(&tkey))
                    || artist_has_track(&artist_dir, &tr.title).is_some()
                {
                    total_skipped += 1;
                    continue;
                }

                on_progress(ArtistSyncStatus::Syncing {
                    artist: disco.artist.name.clone(),
                    release_idx: rel_one_based,
                    release_total: total_releases,
                    current_release: "Singles".to_string(),
                    current_track: tr.title.clone(),
                    saved: total_saved,
                    skipped: total_skipped,
                    failed: all_failures.len(),
                });

                let clean_name = sanitize_filename(&tr.title);
                match extract_ytdlp_audio_with_retry(
                    &tr.url,
                    &singles_dir,
                    &clean_name,
                    format,
                    false,
                    cookies_browser,
                ) {
                    Ok(actual_path) => {
                        if cached_cover.is_none()
                            && let Some(ref curl) = details.cover_url.as_ref().or(rel.cover_url.as_ref())
                        {
                            cached_cover = fetch_square_cover_jpg(curl, &singles_dir);
                        }
                        let track_artist = if tr.artist.trim().is_empty() {
                            disco.artist.name.clone()
                        } else {
                            tr.artist.clone()
                        };
                        let lyrics_query = crate::lyrics::LyricsQuery {
                            artist: track_artist.clone(),
                            title: tr.title.clone(),
                            album: None,
                            duration_secs: tr.duration_secs,
                        };
                        let res = crate::lyrics::fetch_lrclib_lyrics(&lyrics_agent, &lyrics_query);
                        let lyrics = crate::lyrics::lrclib_text_to_embed(&res).map(str::to_string);
                        let spec = TrackTagSpec {
                            title: tr.title.clone(),
                            artist: Some(track_artist),
                            album_artist: Some(disco.artist.name.clone()),
                            album: Some("Singles".to_string()),
                            track_number: None,
                            track_total: None,
                            date: rel.year.map(|year| lofty::tag::items::Timestamp {
                                year,
                                ..Default::default()
                            }),
                            genre: None,
                            publisher: None,
                            source_url: Some(tr.url.clone()),
                            radio_station: None,
                            comment: "Saved via Vibes".to_string(),
                            lyrics,
                        };
                        let _ = write_tags_and_cover(&actual_path, &spec, cached_cover.as_deref());
                        if !tkey.is_empty() {
                            seen_track_keys.insert(tkey);
                        }
                        total_saved += 1;
                    }
                    Err(err) => {
                        all_failures.push(FailedDownloadItem {
                            id: format!("trk-{}", tr.video_id),
                            kind: FailedDownloadKind::ArtistSync,
                            artist: disco.artist.name.clone(),
                            album: Some("Singles".to_string()),
                            title: tr.title.clone(),
                            track_number: None,
                            url: Some(tr.url.clone()),
                            dest_dir: singles_dir.clone(),
                            error: format!("{err}"),
                            failed_at: format_failed_at(),
                        });
                    }
                }
            }
        } else {
            let mut filtered_details = details.clone();
            filtered_details.tracks = filtered_tracks;
            let (alb_status, failures) = download_album_tracks_with_failures(
                albums_dir,
                AlbumFolderMode::ArtistAndAlbum,
                format,
                cookies_browser,
                &filtered_details,
                Some(&cancel),
                |current_track, _done, _tot| {
                    on_progress(ArtistSyncStatus::Syncing {
                        artist: disco.artist.name.clone(),
                        release_idx: rel_one_based,
                        release_total: total_releases,
                        current_release: rel.title.clone(),
                        current_track,
                        saved: total_saved,
                        skipped: total_skipped,
                        failed: all_failures.len(),
                    });
                },
            );
            for tr in &filtered_details.tracks {
                let k = normalize_match_key(&tr.title);
                if !k.is_empty() {
                    seen_track_keys.insert(k);
                }
            }
            if let crate::albums::AlbumDownloadStatus::Completed {
                saved,
                skipped,
                ..
            } = alb_status
            {
                total_saved += saved;
                total_skipped += skipped;
            }
            all_failures.extend(failures);
        }
    }

    let final_st = ArtistSyncStatus::Completed {
        artist: disco.artist.name,
        saved: total_saved,
        skipped: total_skipped,
        failed: all_failures.len(),
    };
    on_progress(final_st.clone());
    (final_st, all_failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_rel(title: &str, year: Option<u16>, rtype: &str, bid: &str) -> AlbumSummary {
        AlbumSummary {
            browse_id: bid.to_string(),
            playlist_id: None,
            title: title.to_string(),
            artist: "Test Artist".to_string(),
            release_type: rtype.to_string(),
            year,
            cover_url: None,
        }
    }

    #[test]
    fn clean_edition_filter_strips_live_box_sets_acapellas_and_deduplicates_metallica_linkin_park() {
        let releases = vec![
            mk_rel("ReLoad (Remastered Deluxe Box Set)", Some(2026), "Album", "b1"),
            mk_rel("Reload", Some(1997), "Album", "b2"),
            mk_rel("Woodstock '94 (Metallica Live FM Broadcast)", Some(2020), "Album", "b3"),
            mk_rel("Hybrid Theory (20th Anniversary Edition)", Some(2020), "Album", "b4"),
            mk_rel("Hybrid Theory", Some(2000), "Album", "b5"),
            mk_rel("From Zero: A Cappellas + Instrumentals", Some(2024), "Album", "b6"),
            mk_rel("From Zero", Some(2024), "Album", "b7"),
            mk_rel("Alone In A Crowd (Sped Up)", Some(2023), "Album", "b8"),
            mk_rel("Alone in a Crowd", Some(2023), "Album", "b9"),
            mk_rel("I Disagree (more)", Some(2020), "Album", "b10"),
            mk_rel("I Disagree", Some(2020), "Album", "b11"),
        ];

        let kept = filter_clean_releases(&releases, true);
        let titles: Vec<&str> = kept.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "From Zero",
                "Alone in a Crowd",
                "I Disagree",
                "Hybrid Theory",
                "Reload",
            ]
        );
    }

    #[test]
    fn bloat_track_filter_strips_live_demos_and_renumbers_studio_tracks() {
        let tracks = vec![
            AlbumTrack {
                track_number: 1,
                title: "Blackened".into(),
                artist: "Metallica".into(),
                duration_text: "6:42".into(),
                duration_secs: Some(402.0),
                video_id: "v1".into(),
                url: "u1".into(),
            },
            AlbumTrack {
                track_number: 2,
                title: "Dyers Eve".into(),
                artist: "Metallica".into(),
                duration_text: "5:13".into(),
                duration_secs: Some(313.0),
                video_id: "v2".into(),
                url: "u2".into(),
            },
            AlbumTrack {
                track_number: 3,
                title: "The Ecstasy of Gold".into(),
                artist: "Metallica".into(),
                duration_text: "1:50".into(),
                duration_secs: Some(110.0),
                video_id: "v3".into(),
                url: "u3".into(),
            },
            AlbumTrack {
                track_number: 4,
                title: "Blackened (Live - Seattle '89)".into(),
                artist: "Metallica".into(),
                duration_text: "6:10".into(),
                duration_secs: Some(370.0),
                video_id: "v4".into(),
                url: "u4".into(),
            },
            AlbumTrack {
                track_number: 5,
                title: "One (Rough Mix)".into(),
                artist: "Metallica".into(),
                duration_text: "7:00".into(),
                duration_secs: Some(420.0),
                video_id: "v5".into(),
                url: "u5".into(),
            },
            AlbumTrack {
                track_number: 6,
                title: "In the End (Demo)".into(),
                artist: "Linkin Park".into(),
                duration_text: "3:30".into(),
                duration_secs: Some(210.0),
                video_id: "v6".into(),
                url: "u6".into(),
            },
        ];

        let (clean, removed) = filter_album_tracks(&tracks, true);
        assert_eq!(removed, 4);
        assert_eq!(clean.len(), 2);
        assert_eq!(clean[0].title, "Blackened");
        assert_eq!(clean[0].track_number, 1);
        assert_eq!(clean[1].title, "Dyers Eve");
        assert_eq!(clean[1].track_number, 2);
    }

    #[test]
    fn canonical_artist_dir_matches_existing_case_and_the_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let korn = tmp.path().join("Korn");
        let dandy = tmp.path().join("The Dandy Warhols");
        fs::create_dir_all(&korn).unwrap();
        fs::create_dir_all(&dandy).unwrap();

        assert_eq!(find_canonical_artist_dir(tmp.path(), "KoRn"), korn);
        assert_eq!(find_canonical_artist_dir(tmp.path(), "Dandy Warhols"), dandy);
        assert_eq!(
            find_canonical_artist_dir(tmp.path(), "Poppy"),
            tmp.path().join("Poppy")
        );
    }
}
