use crate::config::{AlbumFolderMode, FailedDownloadItem, FailedDownloadKind, SaveAudioFormat};
use crate::discography::{filter_album_tracks, find_canonical_artist_dir};
use crate::downloader::{
    TrackTagSpec, crop_thumbnail_to_square_jpg, extract_ytdlp_audio_with_retry, sanitize_filename,
    write_tags_and_cover,
};
use crate::youtube::{format_duration, parse_ytdlp_ndjson};
use anyhow::{Context, Result};
use chrono::Local;
use lofty::tag::items::Timestamp;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const YTM_SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
const YTM_BROWSE_URL: &str = "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false";
const YTM_ALBUM_SEARCH_PARAMS: &str = "EgWKAQIYAWoKEAMQBBAJEAoQBQ==";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumSummary {
    pub browse_id: String,
    pub playlist_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub release_type: String,
    pub year: Option<u16>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlbumTrack {
    pub track_number: u32,
    pub title: String,
    pub artist: String,
    pub duration_text: String,
    pub duration_secs: Option<f64>,
    pub video_id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlbumDetails {
    pub summary: AlbumSummary,
    pub cover_url: Option<String>,
    pub tracks: Vec<AlbumTrack>,
}

impl AlbumDetails {
    pub fn total_duration_text(&self) -> String {
        let tot: f64 = self.tracks.iter().filter_map(|t| t.duration_secs).sum();
        if tot > 0.0 {
            format_duration(tot)
        } else {
            String::new()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlbumDownloadStatus {
    Queued,
    Downloading {
        completed: usize,
        total: usize,
        current_track: String,
    },
    Completed {
        folder: PathBuf,
        saved: usize,
        skipped: usize,
        failed: usize,
    },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct AlbumDownloadJob {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub year: Option<u16>,
    pub folder: PathBuf,
    pub status: AlbumDownloadStatus,
    pub created_at: String,
    pub details: AlbumDetails,
}

pub(crate) fn ytm_context() -> Value {
    json!({
        "client": {
            "clientName": "WEB_REMIX",
            "clientVersion": "1.20250101.01.00",
            "hl": "en",
            "gl": "US"
        }
    })
}

fn build_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(9)))
        .build()
        .into()
}

pub(crate) fn join_runs(runs: Option<&Vec<Value>>) -> String {
    runs.map(|arr| {
        arr.iter()
            .filter_map(|r| r.get("text").and_then(|v| v.as_str()))
            .collect::<String>()
    })
    .unwrap_or_default()
    .trim()
    .to_string()
}

pub(crate) fn best_thumbnail_url(val: &Value) -> Option<String> {
    val.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
}

pub fn parse_duration_str(raw: &str) -> Option<f64> {
    let parts: Vec<u64> = raw
        .trim()
        .split(':')
        .map(|p| p.trim().parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [m, s] => Some((m * 60 + s) as f64),
        [h, m, s] => Some((h * 3600 + m * 60 + s) as f64),
        _ => None,
    }
}

fn parse_year_token(text: &str) -> Option<u16> {
    let t = text.trim();
    if t.len() == 4
        && t.chars().all(|c| c.is_ascii_digit())
        && let Ok(y) = t.parse::<u16>()
        && (1900..=2100).contains(&y)
    {
        return Some(y);
    }
    None
}

pub(crate) fn extract_subtitle_parts(subtitle: &str) -> (String, String, Option<u16>) {
    let parts: Vec<&str> = subtitle
        .split('•')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let mut release_type = "Album".to_string();
    let mut artist_parts = Vec::new();
    let mut year = None;

    for (idx, part) in parts.iter().enumerate() {
        if let Some(y) = parse_year_token(part) {
            year = Some(y);
        } else if idx == 0
            && matches!(
                part.to_lowercase().as_str(),
                "album" | "ep" | "single" | "live" | "compilation"
            )
        {
            release_type = (*part).to_string();
        } else {
            artist_parts.push(*part);
        }
    }
    let artist = if artist_parts.is_empty() {
        "Unknown Artist".to_string()
    } else {
        artist_parts.join(", ")
    };
    (release_type, artist, year)
}

pub(crate) fn walk_json<'a>(val: &'a Value, visit: &mut impl FnMut(&'a Value)) {
    visit(val);
    match val {
        Value::Object(map) => {
            for v in map.values() {
                walk_json(v, visit);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                walk_json(v, visit);
            }
        }
        _ => {}
    }
}

pub fn parse_ytm_album_search_json(root: &Value) -> Vec<AlbumSummary> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    walk_json(root, &mut |node| {
        let Some(r) = node.get("musicResponsiveListItemRenderer") else {
            return;
        };
        let Some(browse_id) = r
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        if !seen.insert(browse_id.to_string()) {
            return;
        }
        let Some(cols) = r.get("flexColumns").and_then(|v| v.as_array()) else {
            return;
        };
        let title = cols
            .first()
            .map(|c| {
                join_runs(
                    c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                        .and_then(|v| v.as_array()),
                )
            })
            .unwrap_or_default();
        if title.is_empty() {
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
        let (release_type, artist, year) = extract_subtitle_parts(&subtitle);

        let playlist_id = r
            .pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchPlaylistEndpoint/playlistId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let cover_url = best_thumbnail_url(r);

        out.push(AlbumSummary {
            browse_id: browse_id.to_string(),
            playlist_id,
            title,
            artist,
            release_type,
            year,
            cover_url,
        });
    });

    out
}

pub fn parse_ytm_album_browse_json(root: &Value, summary: &AlbumSummary) -> AlbumDetails {
    let mut updated_summary = summary.clone();
    let mut cover_url = summary.cover_url.clone();
    let mut tracks = Vec::new();
    let mut seen_vids = HashSet::new();

    walk_json(root, &mut |node| {
        if let Some(hdr) = node.get("musicResponsiveHeaderRenderer") {
            let hdr_title = join_runs(hdr.pointer("/title/runs").and_then(|v| v.as_array()));
            if !hdr_title.is_empty() {
                updated_summary.title = hdr_title;
            }
            let strap_artist = join_runs(
                hdr.pointer("/straplineTextOne/runs")
                    .and_then(|v| v.as_array()),
            );
            if !strap_artist.is_empty() {
                updated_summary.artist = strap_artist;
            }
            let sub = join_runs(hdr.pointer("/subtitle/runs").and_then(|v| v.as_array()));
            if !sub.is_empty() {
                let (rt, _, yr) = extract_subtitle_parts(&sub);
                if !rt.is_empty() {
                    updated_summary.release_type = rt;
                }
                if yr.is_some() {
                    updated_summary.year = yr;
                }
            }
            if let Some(url) = best_thumbnail_url(hdr) {
                cover_url = Some(url.clone());
                updated_summary.cover_url = Some(url);
            }
        }

        let Some(r) = node.get("musicResponsiveListItemRenderer") else {
            return;
        };
        let Some(cols) = r.get("flexColumns").and_then(|v| v.as_array()) else {
            return;
        };
        let Some(col0_runs) = cols
            .first()
            .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
            .and_then(|v| v.as_array())
        else {
            return;
        };

        let title = join_runs(Some(col0_runs));
        let video_id = col0_runs
            .iter()
            .find_map(|run| {
                run.pointer("/navigationEndpoint/watchEndpoint/videoId")
                    .and_then(|v| v.as_str())
            })
            .or_else(|| {
                r.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
                    .and_then(|v| v.as_str())
            })
            .map(|s| s.to_string());

        let Some(vid) = video_id.filter(|s| !s.is_empty()) else {
            return;
        };
        if title.is_empty() || !seen_vids.insert(vid.clone()) {
            return;
        }

        let track_artist = cols
            .get(1)
            .map(|c| {
                join_runs(
                    c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                        .and_then(|v| v.as_array()),
                )
            })
            .filter(|a| !a.is_empty() && !a.ends_with("plays") && !a.ends_with("views"))
            .unwrap_or_else(|| updated_summary.artist.clone());

        let duration_text = r
            .get("fixedColumns")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .map(|fc| {
                join_runs(
                    fc.pointer("/musicResponsiveListItemFixedColumnRenderer/text/runs")
                        .and_then(|v| v.as_array()),
                )
            })
            .unwrap_or_default();
        let duration_secs = parse_duration_str(&duration_text);

        let track_number = (tracks.len() + 1) as u32;
        let url = format!("https://music.youtube.com/watch?v={vid}");

        tracks.push(AlbumTrack {
            track_number,
            title,
            artist: track_artist,
            duration_text,
            duration_secs,
            video_id: vid,
            url,
        });
    });

    AlbumDetails {
        summary: updated_summary,
        cover_url,
        tracks,
    }
}

pub fn extract_playlist_id_from_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !(trimmed.contains("youtube.com/") || trimmed.contains("youtu.be/")) {
        return None;
    }
    if let Some(pos) = trimmed.find("list=") {
        let rest = &trimmed[pos + 5..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

pub(crate) fn post_ytm_json(url: &str, payload: &Value) -> Result<Value> {
    let agent = build_agent();
    let json_body = serde_json::to_string(payload)?;
    let mut resp = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "Mozilla/5.0")
        .send(&json_body)
        .with_context(|| format!("Failed POST to {url}"))?;
    let body = resp.body_mut().read_to_string()?;
    serde_json::from_str(&body).context("Invalid Innertube JSON")
}

pub fn search_albums(query: &str) -> Result<Vec<AlbumSummary>> {
    let trimmed = query.trim();
    anyhow::ensure!(!trimmed.is_empty(), "Empty album search query");

    if let Some(pl_id) = extract_playlist_id_from_url(trimmed) {
        return Ok(vec![AlbumSummary {
            browse_id: String::new(),
            playlist_id: Some(pl_id.clone()),
            title: format!("Playlist {pl_id}"),
            artist: "YouTube Music".to_string(),
            release_type: "Playlist".to_string(),
            year: None,
            cover_url: None,
        }]);
    }

    let payload = json!({
        "context": ytm_context(),
        "query": trimmed,
        "params": YTM_ALBUM_SEARCH_PARAMS
    });

    let root = post_ytm_json(YTM_SEARCH_URL, &payload)
        .context("Failed to search YouTube Music albums")?;
    Ok(parse_ytm_album_search_json(&root))
}

pub fn fetch_album_details(summary: &AlbumSummary) -> Result<AlbumDetails> {
    if !summary.browse_id.is_empty() {
        let payload = json!({
            "context": ytm_context(),
            "browseId": summary.browse_id
        });
        let root = post_ytm_json(YTM_BROWSE_URL, &payload)
            .context("Failed to fetch album tracklist from YouTube Music")?;
        let details = parse_ytm_album_browse_json(&root, summary);
        if !details.tracks.is_empty() {
            return Ok(details);
        }
    }

    if let Some(ref pl) = summary.playlist_id {
        let url = format!("https://music.youtube.com/playlist?list={pl}");
        let output = Command::new("yt-dlp")
            .args([
                "--flat-playlist",
                "--dump-json",
                "--no-warnings",
                "--ignore-errors",
                &url,
            ])
            .output()
            .context("Failed to query playlist with yt-dlp")?;

        let items = parse_ytdlp_ndjson(&String::from_utf8_lossy(&output.stdout));
        let tracks: Vec<AlbumTrack> = items
            .into_iter()
            .enumerate()
            .map(|(ix, it)| AlbumTrack {
                track_number: (ix + 1) as u32,
                title: it.title,
                artist: if it.channel == "YouTube" {
                    summary.artist.clone()
                } else {
                    it.channel
                },
                duration_text: it
                    .duration_secs
                    .map(format_duration)
                    .unwrap_or_default(),
                duration_secs: it.duration_secs,
                video_id: it.id,
                url: it.url,
            })
            .collect();
        anyhow::ensure!(!tracks.is_empty(), "No tracks found in album playlist");
        return Ok(AlbumDetails {
            summary: summary.clone(),
            cover_url: summary.cover_url.clone(),
            tracks,
        });
    }

    anyhow::bail!("Could not load tracks for album '{}'", summary.title)
}

pub fn resolve_album_dir(
    albums_dir: &Path,
    mode: AlbumFolderMode,
    artist: &str,
    album_title: &str,
) -> PathBuf {
    let clean_artist = sanitize_filename(artist);
    let clean_album = sanitize_filename(album_title);
    match mode {
        AlbumFolderMode::ArtistAndAlbum => {
            find_canonical_artist_dir(albums_dir, artist).join(clean_album)
        }
        AlbumFolderMode::AlbumPrefix => albums_dir.join(format!("Album - {clean_album}")),
        AlbumFolderMode::ArtistDashAlbum => {
            albums_dir.join(format!("{clean_artist} - {clean_album}"))
        }
    }
}

pub fn fetch_square_cover_jpg(cover_url: &str, tmp_dir: &Path) -> Option<Vec<u8>> {
    let agent = build_agent();
    let mut resp = agent
        .get(cover_url)
        .header("User-Agent", "Mozilla/5.0")
        .call()
        .ok()?;
    let bytes = resp.body_mut().read_to_vec().ok()?;
    if bytes.is_empty() {
        return None;
    }
    let raw_tmp = tmp_dir.join(".vibes_cover_raw.img");
    let sq_tmp = tmp_dir.join(".vibes_cover_sq.jpg");
    if fs::write(&raw_tmp, &bytes).is_err() {
        return None;
    }
    let res = if crop_thumbnail_to_square_jpg(&raw_tmp, &sq_tmp).is_ok() {
        fs::read(&sq_tmp).ok()
    } else {
        None
    };
    let _ = fs::remove_file(&raw_tmp);
    let _ = fs::remove_file(&sq_tmp);
    res
}

pub fn apply_album_track_tags(
    audio_path: &Path,
    details: &AlbumDetails,
    track: &AlbumTrack,
    total_tracks: u32,
    cover_jpg: Option<&[u8]>,
    lyrics: Option<String>,
) -> Result<()> {
    let artist = if track.artist.trim().is_empty() {
        details.summary.artist.trim().to_string()
    } else {
        track.artist.trim().to_string()
    };
    let album_artist = (!details.summary.artist.trim().is_empty())
        .then(|| details.summary.artist.trim().to_string())
        .or_else(|| Some(artist.clone()));

    let spec = TrackTagSpec {
        title: track.title.clone(),
        artist: Some(artist),
        album_artist,
        album: Some(details.summary.title.clone()),
        track_number: Some(track.track_number),
        track_total: Some(total_tracks),
        date: details.summary.year.map(|year| Timestamp {
            year,
            ..Default::default()
        }),
        genre: None,
        publisher: None,
        source_url: Some(track.url.clone()),
        radio_station: None,
        comment: "Saved via Vibes".to_string(),
        lyrics,
    };
    write_tags_and_cover(audio_path, &spec, cover_jpg)
}

pub fn download_album_tracks_with_failures(
    albums_dir: &Path,
    folder_mode: AlbumFolderMode,
    format: SaveAudioFormat,
    cookies_browser: Option<&str>,
    details: &AlbumDetails,
    cancel: Option<&AtomicBool>,
    mut on_track_progress: impl FnMut(String, usize, usize),
) -> (AlbumDownloadStatus, Vec<FailedDownloadItem>) {
    let is_single_release = details.tracks.len() <= 2
        && (details.summary.release_type.eq_ignore_ascii_case("Single")
            || details.tracks.len() == 1);

    let album_dir = if is_single_release && folder_mode == AlbumFolderMode::ArtistAndAlbum {
        find_canonical_artist_dir(albums_dir, &details.summary.artist).join("Singles")
    } else {
        resolve_album_dir(
            albums_dir,
            folder_mode,
            &details.summary.artist,
            &details.summary.title,
        )
    };

    if let Err(err) = fs::create_dir_all(&album_dir) {
        return (
            AlbumDownloadStatus::Failed(format!("Cannot create {}: {err}", album_dir.display())),
            Vec::new(),
        );
    }

    let cover_jpg = if is_single_release {
        details
            .cover_url
            .as_deref()
            .and_then(|url| fetch_square_cover_jpg(url, &album_dir))
    } else {
        let cover_file = album_dir.join("cover.jpg");
        if cover_file.exists() {
            fs::read(&cover_file).ok()
        } else if let Some(ref url) = details.cover_url {
            let fetched = fetch_square_cover_jpg(url, &album_dir);
            if let Some(ref bytes) = fetched {
                let _ = fs::write(&cover_file, bytes);
            }
            fetched
        } else {
            None
        }
    };

    let lyrics_agent = crate::lyrics::new_http_agent();
    let total = details.tracks.len();
    let total_u32 = total as u32;
    let mut saved = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    let artist_dir = find_canonical_artist_dir(albums_dir, &details.summary.artist);

    for (idx, track) in details.tracks.iter().enumerate() {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            break;
        }
        let clean_name = if is_single_release {
            sanitize_filename(&track.title)
        } else {
            format!("{:02} - {}", track.track_number, sanitize_filename(&track.title))
        };
        on_track_progress(clean_name.clone(), idx, total);

        let expected_path = album_dir.join(format!("{clean_name}.{}", format.extension()));
        if expected_path.exists()
            || crate::local_library::find_local_track_in_dir(
                &album_dir,
                track.track_number,
                &track.title,
            )
            .is_some()
            || (is_single_release
                && crate::local_library::artist_has_track(&artist_dir, &track.title).is_some())
        {
            skipped += 1;
            continue;
        }

        match extract_ytdlp_audio_with_retry(
            &track.url,
            &album_dir,
            &clean_name,
            format,
            false,
            cookies_browser,
        ) {
            Ok(actual_path) => {
                let track_artist = if track.artist.trim().is_empty() {
                    details.summary.artist.trim().to_string()
                } else {
                    track.artist.trim().to_string()
                };
                let fetched_lyrics = if !track_artist.is_empty() {
                    let query = crate::lyrics::LyricsQuery {
                        artist: track_artist,
                        title: track.title.clone(),
                        album: (!is_single_release).then(|| details.summary.title.clone()),
                        duration_secs: track.duration_secs,
                    };
                    let res = crate::lyrics::fetch_lrclib_lyrics(&lyrics_agent, &query);
                    crate::lyrics::lrclib_text_to_embed(&res).map(str::to_string)
                } else {
                    None
                };

                let mut tagged_details = details.clone();
                if is_single_release {
                    tagged_details.summary.title = "Singles".to_string();
                }
                if let Err(err) = apply_album_track_tags(
                    &actual_path,
                    &tagged_details,
                    track,
                    total_u32,
                    cover_jpg.as_deref(),
                    fetched_lyrics,
                ) {
                    let msg = format!("Album tag warning on {}: {err}", actual_path.display());
                    crate::log::log_warn("albums", &msg);
                    eprintln!("Vibes {msg}");
                }
                saved += 1;
            }
            Err(err) => {
                let err_str = format!("{err}");
                let msg = format!("Album track download failed ({}): {err_str}", track.title);
                crate::log::log_warn("albums", &msg);
                eprintln!("Vibes {msg}");
                failures.push(FailedDownloadItem {
                    id: format!("alb-{}-{}", track.video_id, clean_name),
                    kind: FailedDownloadKind::AlbumTrack,
                    artist: details.summary.artist.clone(),
                    album: Some(if is_single_release {
                        "Singles".to_string()
                    } else {
                        details.summary.title.clone()
                    }),
                    title: track.title.clone(),
                    track_number: (!is_single_release).then_some(track.track_number),
                    url: Some(track.url.clone()),
                    dest_dir: album_dir.clone(),
                    error: err_str.clone(),
                    failed_at: Local::now().format("%Y-%m-%d %H:%M").to_string(),
                });
                if crate::downloader::is_yt_auth_gate_error(&err_str) {
                    crate::log::log_warn(
                        "albums",
                        "Stopping album download early: YouTube requested sign-in",
                    );
                    break;
                }
            }
        }
    }

    let failed = failures.len();
    let final_status = if saved == 0 && skipped == 0 && failed > 0 {
        AlbumDownloadStatus::Failed(format!("All {failed} track downloads failed"))
    } else {
        AlbumDownloadStatus::Completed {
            folder: album_dir,
            saved,
            skipped,
            failed,
        }
    };
    (final_status, failures)
}

pub fn run_download_album(
    albums_dir: &Path,
    folder_mode: AlbumFolderMode,
    format: SaveAudioFormat,
    cookies_browser: Option<&str>,
    details: &AlbumDetails,
    clean_editions: bool,
    mut on_progress: impl FnMut(AlbumDownloadStatus),
) -> (AlbumDownloadStatus, Vec<FailedDownloadItem>) {
    let mut effective = details.clone();
    let (filtered_tracks, _) = filter_album_tracks(&details.tracks, clean_editions);
    effective.tracks = filtered_tracks;

    let (final_status, failures) = download_album_tracks_with_failures(
        albums_dir,
        folder_mode,
        format,
        cookies_browser,
        &effective,
        None,
        |current_track, completed, total| {
            on_progress(AlbumDownloadStatus::Downloading {
                completed,
                total,
                current_track,
            });
        },
    );
    on_progress(final_status.clone());
    (final_status, failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::file::TaggedFileExt;
    use lofty::picture::{MimeType, PictureType};
    use lofty::probe::Probe;
    use lofty::tag::{Accessor, ItemKey};

    #[test]
    fn parses_ytm_album_search_and_browse_json() {
        let search_json = json!({
            "contents": {
                "tabbedSearchResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicShelfRenderer": {
                                            "contents": [{
                                                "musicResponsiveListItemRenderer": {
                                                    "flexColumns": [
                                                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Demon Days"}]}}},
                                                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Album • Gorillaz • 2005"}]}}}
                                                    ],
                                                    "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_2az20wG4HxN"}},
                                                    "overlay": {"musicItemThumbnailOverlayRenderer": {"content": {"musicPlayButtonRenderer": {"playNavigationEndpoint": {"watchPlaylistEndpoint": {"playlistId": "OLAK5uy_test"}}}}}},
                                                    "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "https://img/cover544.jpg"}]}}}
                                                }
                                            }]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });

        let found = parse_ytm_album_search_json(&search_json);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Demon Days");
        assert_eq!(found[0].artist, "Gorillaz");
        assert_eq!(found[0].year, Some(2005));
        assert_eq!(found[0].playlist_id.as_deref(), Some("OLAK5uy_test"));

        let browse_json = json!({
            "header": {
                "musicResponsiveHeaderRenderer": {
                    "title": {"runs": [{"text": "Demon Days"}]},
                    "straplineTextOne": {"runs": [{"text": "Gorillaz"}]},
                    "subtitle": {"runs": [{"text": "Album • 2005"}]},
                    "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "https://img/hdr544.jpg"}]}}}
                }
            },
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "secondaryContents": {
                        "sectionListRenderer": {
                            "contents": [{
                                "musicShelfRenderer": {
                                    "contents": [
                                        {
                                            "musicResponsiveListItemRenderer": {
                                                "flexColumns": [
                                                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Intro", "navigationEndpoint": {"watchEndpoint": {"videoId": "vid1"}}}]}}}
                                                ],
                                                "fixedColumns": [
                                                    {"musicResponsiveListItemFixedColumnRenderer": {"text": {"runs": [{"text": "1:08"}]}}}
                                                ]
                                            }
                                        },
                                        {
                                            "musicResponsiveListItemRenderer": {
                                                "flexColumns": [
                                                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Feel Good Inc.", "navigationEndpoint": {"watchEndpoint": {"videoId": "vid2"}}}]}}},
                                                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Gorillaz, De La Soul"}]}}}
                                                ],
                                                "fixedColumns": [
                                                    {"musicResponsiveListItemFixedColumnRenderer": {"text": {"runs": [{"text": "3:43"}]}}}
                                                ]
                                            }
                                        }
                                    ]
                                }
                            }]
                        }
                    }
                }
            }
        });

        let details = parse_ytm_album_browse_json(&browse_json, &found[0]);
        assert_eq!(details.cover_url.as_deref(), Some("https://img/hdr544.jpg"));
        assert_eq!(details.tracks.len(), 2);
        assert_eq!(details.tracks[0].track_number, 1);
        assert_eq!(details.tracks[0].duration_secs, Some(68.0));
        assert_eq!(details.tracks[1].track_number, 2);
        assert_eq!(details.tracks[1].artist, "Gorillaz, De La Soul");
        assert_eq!(details.tracks[1].duration_secs, Some(223.0));
        assert_eq!(details.total_duration_text(), "04:51");
    }

    #[test]
    fn resolves_album_folder_modes_cleanly() {
        let base = Path::new("/home/user/Music");
        assert_eq!(
            resolve_album_dir(base, AlbumFolderMode::ArtistAndAlbum, "AC/DC", "Back In Black"),
            PathBuf::from("/home/user/Music/AC-DC/Back In Black")
        );
        assert_eq!(
            resolve_album_dir(base, AlbumFolderMode::AlbumPrefix, "Gorillaz", "Demon Days"),
            PathBuf::from("/home/user/Music/Album - Demon Days")
        );
        assert_eq!(
            resolve_album_dir(base, AlbumFolderMode::ArtistDashAlbum, "Gorillaz", "Demon Days"),
            PathBuf::from("/home/user/Music/Gorillaz - Demon Days")
        );
    }

    #[test]
    fn tags_album_track_with_track_numbers_album_artist_and_cover() {
        let tmp = tempfile::tempdir().unwrap();
        let opus_path = tmp.path().join("06 - Feel Good Inc.opus");
        let jpg_path = tmp.path().join("cover.jpg");

        let st_audio = Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-t",
                "0.2",
                "-c:a",
                "libopus",
            ])
            .arg(&opus_path)
            .status()
            .unwrap();
        assert!(st_audio.success());

        let st_img = Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0x8ba4b0:s=128x128",
                "-frames:v",
                "1",
            ])
            .arg(&jpg_path)
            .status()
            .unwrap();
        assert!(st_img.success());
        let cover_bytes = fs::read(&jpg_path).unwrap();

        let details = AlbumDetails {
            summary: AlbumSummary {
                browse_id: "MPREb_test".into(),
                playlist_id: None,
                title: "Demon Days".into(),
                artist: "Gorillaz".into(),
                release_type: "Album".into(),
                year: Some(2005),
                cover_url: None,
            },
            cover_url: None,
            tracks: Vec::new(),
        };
        let track = AlbumTrack {
            track_number: 6,
            title: "Feel Good Inc.".into(),
            artist: "Gorillaz".into(),
            duration_text: "3:43".into(),
            duration_secs: Some(223.0),
            video_id: "HyHNuVaZJ-k".into(),
            url: "https://music.youtube.com/watch?v=HyHNuVaZJ-k".into(),
        };

        let sample_lrc = "[00:14.20] Feel Good";
        apply_album_track_tags(
            &opus_path,
            &details,
            &track,
            15,
            Some(&cover_bytes),
            Some(sample_lrc.to_string()),
        )
        .unwrap();

        let probed = Probe::open(&opus_path).unwrap().read().unwrap();
        let tag = probed.primary_tag().expect("album tag must exist");
        assert_eq!(tag.title().as_deref(), Some("Feel Good Inc."));
        assert_eq!(tag.artist().as_deref(), Some("Gorillaz"));
        assert_eq!(tag.album().as_deref(), Some("Demon Days"));
        assert_eq!(tag.get_string(ItemKey::AlbumArtist), Some("Gorillaz"));
        assert_eq!(tag.track(), Some(6));
        assert_eq!(tag.track_total(), Some(15));
        assert_eq!(tag.date().map(|d| d.year), Some(2005));
        assert_eq!(
            crate::lyrics::read_embedded_lyrics(&opus_path).as_deref(),
            Some(sample_lrc)
        );
        assert!(
            tag.pictures()
                .iter()
                .any(|p| p.pic_type() == PictureType::CoverFront
                    && p.mime_type() == Some(&MimeType::Jpeg)
                    && !p.data().is_empty())
        );
    }
}
