use crate::albums::resolve_album_dir;
use crate::artwork::{resolve_local_album_art, resolve_local_single_art};
use crate::config::AlbumFolderMode;
use crate::discography::{
    canonical_release_key, edition_bloat_rank, find_canonical_artist_dir, is_bloat_track,
    is_hard_bloat_release, normalize_artist_key,
};
use crate::downloader::sanitize_filename;
use anyhow::Result;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, TagExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const AUDIO_EXTS: &[&str] = &["opus", "ogg", "mp3", "m4a", "flac", "wav"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalTrack {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub cover_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalAlbum {
    pub title: String,
    pub artist: String,
    pub folder: PathBuf,
    pub year: Option<u16>,
    pub tracks: Vec<LocalTrack>,
    pub cover_path: Option<PathBuf>,
}

impl LocalAlbum {
    pub fn total_duration_secs(&self) -> f64 {
        self.tracks.iter().filter_map(|t| t.duration_secs).sum()
    }

    pub fn format_summary(&self) -> String {
        let mut seen: Option<&str> = None;
        for tr in &self.tracks {
            let f = tr.format.trim();
            if f.is_empty() {
                continue;
            }
            match seen {
                None => seen = Some(f),
                Some(prev) if prev.eq_ignore_ascii_case(f) => {}
                Some(_) => return "MIXED".to_string(),
            }
        }
        seen.unwrap_or("AUDIO").to_uppercase()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalArtist {
    pub name: String,
    pub key: String,
    pub folder: PathBuf,
    pub albums: Vec<LocalAlbum>,
    pub singles: Vec<LocalTrack>,
    pub cover_path: Option<PathBuf>,
    pub total_tracks: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalLibrarySnapshot {
    pub artists: Vec<LocalArtist>,
    pub albums: Vec<LocalAlbum>,
    pub singles: Vec<LocalTrack>,
}

pub fn is_audio_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| AUDIO_EXTS.iter().any(|&k| k.eq_ignore_ascii_case(ext)))
}

fn split_leading_track_prefix(stem: &str) -> (Option<u32>, &str) {
    let trimmed = stem.trim_start();
    let digit_len = trimmed
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digit_len == 0 || digit_len > 3 {
        return (None, stem);
    }
    let Ok(num) = trimmed[..digit_len].parse::<u32>() else {
        return (None, stem);
    };
    let after_digits = &trimmed[digit_len..];
    let after_ws = after_digits.trim_start();
    let Some(sep) = after_ws.chars().next() else {
        return (None, stem);
    };
    if !matches!(sep, '-' | '.' | '_') {
        return (None, stem);
    }
    let after_sep = &after_ws[sep.len_utf8()..];
    if !after_sep.chars().next().is_some_and(|c| c.is_whitespace()) {
        return (None, stem);
    }
    let rest = after_sep.trim_start();
    if rest.is_empty() {
        return (None, stem);
    }
    (Some(num), rest)
}

pub fn strip_track_prefix(stem: &str) -> &str {
    split_leading_track_prefix(stem).1
}

fn parse_leading_track_num(stem: &str) -> Option<u32> {
    split_leading_track_prefix(stem).0
}

pub fn normalize_match_key(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn is_hidden_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|s| s.starts_with('.'))
}

fn dir_has_audio(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if is_audio_file(&entry.path()) {
            return true;
        }
    }
    false
}

pub fn find_local_album_dir(albums_dir: &Path, artist: &str, album_title: &str) -> Option<PathBuf> {
    if !albums_dir.is_dir() {
        return None;
    }

    for mode in AlbumFolderMode::ALL {
        let cand = resolve_album_dir(albums_dir, mode, artist, album_title);
        if cand.is_dir() && dir_has_audio(&cand) {
            return Some(cand);
        }
    }

    let want_album = normalize_match_key(album_title);
    if want_album.is_empty() {
        return None;
    }
    let want_artist = normalize_match_key(artist);
    let want_prefixed = normalize_match_key(&format!("Album - {album_title}"));
    let want_dashed = normalize_match_key(&format!("{artist} - {album_title}"));

    let entries = fs::read_dir(albums_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || is_hidden_name(&path) {
            continue;
        }
        let folder_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let folder_key = normalize_match_key(folder_name);

        if (folder_key == want_album
            || folder_key == want_prefixed
            || (!want_artist.is_empty() && folder_key == want_dashed))
            && dir_has_audio(&path)
        {
            return Some(path);
        }

        if let Ok(sub_entries) = fs::read_dir(&path) {
            for sub in sub_entries.flatten() {
                let sub_path = sub.path();
                if !sub_path.is_dir() || is_hidden_name(&sub_path) {
                    continue;
                }
                let child_name = sub_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let child_key = normalize_match_key(child_name);
                let artist_matches = want_artist.is_empty()
                    || folder_key == want_artist
                    || folder_key.contains(&want_artist)
                    || want_artist.contains(&folder_key);
                if child_key == want_album && artist_matches && dir_has_audio(&sub_path) {
                    return Some(sub_path);
                }
            }
        }
    }

    None
}

pub fn find_local_track_in_dir(dir: &Path, track_number: u32, track_title: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }

    let clean_title = sanitize_filename(track_title);
    for ext in AUDIO_EXTS {
        let exact = dir.join(format!("{track_number:02} - {clean_title}.{ext}"));
        if exact.is_file() {
            return Some(exact);
        }
    }

    let want_key = normalize_match_key(strip_track_prefix(track_title));
    if want_key.is_empty() {
        return None;
    }

    let mut fallback_match: Option<PathBuf> = None;
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_audio_file(&path) {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let stripped = strip_track_prefix(stem);
        let stem_key = normalize_match_key(stripped);
        if stem_key == want_key {
            return Some(path);
        }
        if fallback_match.is_none()
            && want_key.len() >= 4
            && parse_leading_track_num(stem) == Some(track_number)
            && (stem_key.contains(&want_key) || want_key.contains(&stem_key))
        {
            fallback_match = Some(path);
        }
    }

    fallback_match
}

#[cfg(test)]
pub fn find_local_album_track(
    albums_dir: &Path,
    artist: &str,
    album_title: &str,
    track_number: u32,
    track_title: &str,
) -> Option<PathBuf> {
    let dir = find_local_album_dir(albums_dir, artist, album_title)?;
    find_local_track_in_dir(&dir, track_number, track_title)
}

/// Collects normalized track keys across an artist's `Singles/` and album folders.
pub fn collect_artist_track_keys(artist_dir: &Path) -> HashSet<String> {
    let mut keys = HashSet::new();
    let Ok(entries) = fs::read_dir(artist_dir) else {
        return keys;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && !is_hidden_name(&p) {
            if let Ok(sub) = fs::read_dir(&p) {
                for f in sub.flatten() {
                    let fp = f.path();
                    if is_audio_file(&fp) {
                        let stem = fp.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                        let k = normalize_match_key(strip_track_prefix(stem));
                        if !k.is_empty() {
                            keys.insert(k);
                        }
                    }
                }
            }
        } else if is_audio_file(&p) {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            let k = normalize_match_key(strip_track_prefix(stem));
            if !k.is_empty() {
                keys.insert(k);
            }
        }
    }
    keys
}

/// Checks if any folder under `artist_dir` already contains a track matching `track_title`.
pub fn artist_has_track(artist_dir: &Path, track_title: &str) -> Option<PathBuf> {
    let want = normalize_match_key(strip_track_prefix(track_title));
    if want.is_empty() || !artist_dir.is_dir() {
        return None;
    }
    let Ok(entries) = fs::read_dir(artist_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && !is_hidden_name(&p) {
            if let Some(hit) = find_local_track_in_dir(&p, 0, track_title) {
                return Some(hit);
            }
        } else if is_audio_file(&p) {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            if normalize_match_key(strip_track_prefix(stem)) == want {
                return Some(p);
            }
        }
    }
    None
}

pub fn read_local_track(
    path: &Path,
    fallback_artist: Option<&str>,
    fallback_album: Option<&str>,
) -> (LocalTrack, Option<u16>) {
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_uppercase();

    let tagged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Probe::open(path)
            .ok()
            .and_then(|p| p.guess_file_type().ok())
            .and_then(|p| {
                p.options(ParseOptions::new().read_cover_art(false))
                    .read()
                    .ok()
            })
    }))
    .ok()
    .flatten();

    let duration_secs = tagged
        .as_ref()
        .map(|f| f.properties().duration().as_secs_f64())
        .filter(|&d| d > 0.0);

    let tag = tagged
        .as_ref()
        .and_then(|f| f.primary_tag().or_else(|| f.first_tag()));

    let tag_str = |v: Option<std::borrow::Cow<'_, str>>| -> Option<String> {
        v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };

    let title = tag
        .and_then(|t| tag_str(t.title()))
        .unwrap_or_else(|| {
            let stripped = strip_track_prefix(file_stem).trim();
            if !stripped.is_empty() {
                stripped.to_string()
            } else {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown Track")
                    .to_string()
            }
        });

    let artist = tag
        .and_then(|t| {
            tag_str(t.artist())
                .or_else(|| t.get_string(ItemKey::AlbumArtist).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        })
        .or_else(|| {
            fallback_artist
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let album = tag
        .and_then(|t| tag_str(t.album()))
        .or_else(|| {
            fallback_album
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default();

    let track_number = tag
        .and_then(|t| t.track())
        .filter(|&n| n > 0)
        .or_else(|| parse_leading_track_num(file_stem));

    let year = tag.and_then(|t| t.date()).map(|ts| ts.year).filter(|&y| y > 0);

    (
        LocalTrack {
            path: path.to_path_buf(),
            title,
            artist,
            album,
            track_number,
            duration_secs,
            format,
            cover_path: None,
        },
        year,
    )
}

fn canonical_or_raw(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn infer_album_fallback(albums_dir: &Path, dir: &Path) -> (Option<String>, Option<String>) {
    let folder_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .trim();

    if let Some(prefix) = folder_name.get(..8)
        && prefix.eq_ignore_ascii_case("album - ")
        && let Some(rest) = folder_name.get(8..)
    {
        let rest = rest.trim();
        if !rest.is_empty() {
            return (None, Some(rest.to_string()));
        }
    }

    if let Some(parent) = dir.parent()
        && canonical_or_raw(parent) != canonical_or_raw(albums_dir)
    {
        let parent_name = parent
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .trim();
        let artist = (!parent_name.is_empty()).then(|| parent_name.to_string());
        let album = (!folder_name.is_empty()).then(|| folder_name.to_string());
        return (artist, album);
    }

    if let Some((a, b)) = folder_name.split_once(" - ") {
        let a = a.trim();
        let b = b.trim();
        if !a.is_empty() && !b.is_empty() {
            return (Some(a.to_string()), Some(b.to_string()));
        }
    }

    (
        None,
        (!folder_name.is_empty()).then(|| folder_name.to_string()),
    )
}

fn most_common_string<'a>(items: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counts: HashMap<String, (usize, String)> = HashMap::new();
    for raw in items {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        let entry = counts
            .entry(key)
            .or_insert_with(|| (0, trimmed.to_string()));
        entry.0 += 1;
    }
    counts
        .into_values()
        .max_by(|(ca, sa), (cb, sb)| ca.cmp(cb).then_with(|| sb.cmp(sa)))
        .map(|(_, s)| s)
}

fn collect_singles_dir(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    seen_dirs: &mut HashSet<PathBuf>,
    seen_files: &mut HashSet<PathBuf>,
    singles: &mut Vec<LocalTrack>,
) {
    if depth > max_depth || !dir.is_dir() || is_hidden_name(dir) {
        return;
    }
    let canon = canonical_or_raw(dir);
    if !seen_dirs.insert(canon) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !is_hidden_name(&path) {
                subdirs.push(path);
            }
        } else if is_audio_file(&path) {
            let fcanon = canonical_or_raw(&path);
            if seen_files.insert(fcanon) {
                let (mut track, _) = read_local_track(&path, None, Some("Radio & Singles"));
                track.cover_path = resolve_local_single_art(&track.path);
                singles.push(track);
            }
        }
    }
    for sub in subdirs {
        collect_singles_dir(&sub, depth + 1, max_depth, seen_dirs, seen_files, singles);
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_albums_dir(
    albums_dir: &Path,
    radio_canon: &Path,
    albums_equal_radio: bool,
    dir: &Path,
    depth: usize,
    seen_dirs: &mut HashSet<PathBuf>,
    seen_files: &mut HashSet<PathBuf>,
    albums: &mut Vec<LocalAlbum>,
    singles: &mut Vec<LocalTrack>,
) {
    if depth > 3 || !dir.is_dir() {
        return;
    }
    if depth > 0 && is_hidden_name(dir) {
        return;
    }

    let canon = canonical_or_raw(dir);
    if !albums_equal_radio && depth > 0 && canon == radio_canon {
        collect_singles_dir(dir, 0, 2, seen_dirs, seen_files, singles);
        return;
    }
    if !seen_dirs.insert(canon) {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    let mut audio_files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !is_hidden_name(&path) {
                subdirs.push(path);
            }
        } else if is_audio_file(&path) {
            audio_files.push(path);
        }
    }

    if depth == 0 {
        for file in audio_files {
            if seen_files.insert(canonical_or_raw(&file)) {
                let (mut track, _) = read_local_track(&file, None, Some("Singles"));
                track.cover_path = resolve_local_single_art(&track.path);
                singles.push(track);
            }
        }
    } else if !audio_files.is_empty() {
        let is_singles_dir = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("singles"));
        let (fallback_artist, fallback_album) = infer_album_fallback(albums_dir, dir);

        if is_singles_dir {
            for file in audio_files {
                if !seen_files.insert(canonical_or_raw(&file)) {
                    continue;
                }
                let (mut track, _) = read_local_track(
                    &file,
                    fallback_artist.as_deref(),
                    Some("Singles"),
                );
                if track.album.trim().is_empty() {
                    track.album = "Singles".to_string();
                }
                track.cover_path = resolve_local_single_art(&track.path)
                    .or_else(|| resolve_local_album_art(dir, std::slice::from_ref(&track.path)));
                singles.push(track);
            }
            for sub in subdirs {
                walk_albums_dir(
                    albums_dir,
                    radio_canon,
                    albums_equal_radio,
                    &sub,
                    depth + 1,
                    seen_dirs,
                    seen_files,
                    albums,
                    singles,
                );
            }
            return;
        }

        let mut tracks = Vec::with_capacity(audio_files.len());
        let mut year: Option<u16> = None;

        for file in audio_files {
            if !seen_files.insert(canonical_or_raw(&file)) {
                continue;
            }
            let (track, tr_year) = read_local_track(
                &file,
                fallback_artist.as_deref(),
                fallback_album.as_deref(),
            );
            if year.is_none() {
                year = tr_year;
            }
            tracks.push(track);
        }

        if !tracks.is_empty() {
            tracks.sort_by(|a, b| {
                a.track_number
                    .unwrap_or(u32::MAX)
                    .cmp(&b.track_number.unwrap_or(u32::MAX))
                    .then_with(|| {
                        let fa = a
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase());
                        let fb = b
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase());
                        fa.cmp(&fb)
                    })
            });

            let album_artist = most_common_string(
                tracks
                    .iter()
                    .map(|t| t.artist.as_str())
                    .filter(|&a| !a.eq_ignore_ascii_case("Unknown Artist")),
            )
            .or_else(|| fallback_artist.clone())
            .unwrap_or_else(|| "Unknown Artist".to_string());

            let folder_fallback = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown Album")
                .to_string();
            let album_title = most_common_string(tracks.iter().map(|t| t.album.as_str()))
                .or(fallback_album)
                .unwrap_or(folder_fallback);

            let track_paths: Vec<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
            let cover_path = resolve_local_album_art(dir, &track_paths);
            let is_singles_dir = dir
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("singles"));
            for tr in &mut tracks {
                tr.cover_path = if is_singles_dir {
                    resolve_local_single_art(&tr.path).or_else(|| cover_path.clone())
                } else {
                    cover_path.clone()
                };
            }

            albums.push(LocalAlbum {
                title: album_title,
                artist: album_artist,
                folder: dir.to_path_buf(),
                year,
                tracks,
                cover_path,
            });
        }
    }

    for sub in subdirs {
        walk_albums_dir(
            albums_dir,
            radio_canon,
            albums_equal_radio,
            &sub,
            depth + 1,
            seen_dirs,
            seen_files,
            albums,
            singles,
        );
    }
}

pub fn scan_local_library(albums_dir: &Path, radio_music_dir: &Path) -> LocalLibrarySnapshot {
    let mut albums = Vec::new();
    let mut singles = Vec::new();
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();

    let albums_canon = canonical_or_raw(albums_dir);
    let radio_canon = canonical_or_raw(radio_music_dir);
    let albums_equal_radio = albums_canon == radio_canon;

    if albums_dir.is_dir() {
        walk_albums_dir(
            albums_dir,
            &radio_canon,
            albums_equal_radio,
            albums_dir,
            0,
            &mut seen_dirs,
            &mut seen_files,
            &mut albums,
            &mut singles,
        );
    }

    if radio_music_dir.is_dir() {
        collect_singles_dir(
            radio_music_dir,
            0,
            2,
            &mut seen_dirs,
            &mut seen_files,
            &mut singles,
        );
    }

    albums.sort_by(|a, b| {
        a.artist
            .to_lowercase()
            .cmp(&b.artist.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    singles.sort_by(|a, b| {
        let ma = fs::metadata(&a.path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mb = fs::metadata(&b.path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        mb.cmp(&ma)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    // Build author-first grouping across albums + singles.
    struct ArtistAcc {
        display_names: Vec<String>,
        albums: Vec<LocalAlbum>,
        singles: Vec<LocalTrack>,
    }
    let mut artist_map: HashMap<String, ArtistAcc> = HashMap::new();
    for alb in &albums {
        let k = {
            let ak = normalize_artist_key(&alb.artist);
            if ak.is_empty() {
                "unknown".to_string()
            } else {
                ak
            }
        };
        let acc = artist_map.entry(k).or_insert_with(|| ArtistAcc {
            display_names: Vec::new(),
            albums: Vec::new(),
            singles: Vec::new(),
        });
        acc.display_names.push(alb.artist.clone());
        acc.albums.push(alb.clone());
    }
    for sng in &singles {
        let k = {
            let ak = normalize_artist_key(&sng.artist);
            if ak.is_empty() {
                "unknown".to_string()
            } else {
                ak
            }
        };
        let acc = artist_map.entry(k).or_insert_with(|| ArtistAcc {
            display_names: Vec::new(),
            albums: Vec::new(),
            singles: Vec::new(),
        });
        acc.display_names.push(sng.artist.clone());
        acc.singles.push(sng.clone());
    }

    let mut artists: Vec<LocalArtist> = artist_map
        .into_iter()
        .map(|(key, mut acc)| {
            let name = most_common_string(acc.display_names.iter().map(|s| s.as_str()))
                .unwrap_or_else(|| "Unknown Artist".to_string());
            acc.albums.sort_by(|a, b| {
                b.year
                    .unwrap_or(0)
                    .cmp(&a.year.unwrap_or(0))
                    .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            });
            acc.singles
                .sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            let cover_path = acc
                .albums
                .iter()
                .find_map(|a| a.cover_path.clone())
                .or_else(|| acc.singles.iter().find_map(|s| s.cover_path.clone()));
            let folder = find_canonical_artist_dir(albums_dir, &name);
            let total_tracks = acc.albums.iter().map(|a| a.tracks.len()).sum::<usize>()
                + acc.singles.len();
            LocalArtist {
                name,
                key,
                folder,
                albums: acc.albums,
                singles: acc.singles,
                cover_path,
                total_tracks,
            }
        })
        .collect();

    artists.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    LocalLibrarySnapshot {
        artists,
        albums,
        singles,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReorgReport {
    pub moved_radio: usize,
    pub consolidated_singles: usize,
    pub removed_bloat_tracks: usize,
    pub removed_duplicate_albums: usize,
    pub removed_duplicate_files: usize,
    pub renamed_folders: usize,
    pub backup_dir: PathBuf,
}

impl ReorgReport {
    pub fn summary(&self) -> String {
        format!(
            "Reorganized: {} Radio routed · {} Singles consolidated · {} bloat tracks, {} dup albums, {} dup files moved to backup ({})",
            self.moved_radio,
            self.consolidated_singles,
            self.removed_bloat_tracks,
            self.removed_duplicate_albums,
            self.removed_duplicate_files,
            self.backup_dir.display()
        )
    }
}

fn format_priority(path: &Path) -> u8 {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("flac") => 5,
        Some("opus") => 4,
        Some("m4a") => 3,
        Some("ogg") => 2,
        Some("mp3") => 1,
        _ => 0,
    }
}

fn move_file_reorg(src: &Path, dst: &Path) -> Result<()> {
    if src == dst {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    fs::copy(src, dst)?;
    let _ = fs::remove_file(src);
    Ok(())
}

fn move_to_backup(src: &Path, base_dir: &Path, backup_dir: &Path) -> Result<PathBuf> {
    let rel = src.strip_prefix(base_dir).unwrap_or_else(|_| {
        src.file_name()
            .map(Path::new)
            .unwrap_or_else(|| Path::new("item"))
    });
    let mut dst = backup_dir.join(rel);
    if dst.exists() {
        let stem = dst
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("item")
            .to_string();
        let ext = dst
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        dst = dst.with_file_name(format!(
            "{stem}-{}{ext}",
            chrono::Utc::now().timestamp_millis()
        ));
    }
    move_file_reorg(src, &dst)?;
    Ok(dst)
}

fn is_fake_radio_album(album: &str, title: &str, radio_station: Option<&str>) -> bool {
    let trimmed = album.trim();
    if trimmed.is_empty() {
        return true;
    }
    let ak = normalize_match_key(trimmed);
    let tk = normalize_match_key(title);
    if ak.is_empty() || ak == tk {
        return true;
    }
    let lower = trimmed.to_lowercase();
    lower.contains("radio")
        || lower.contains(".fm")
        || trimmed.eq_ignore_ascii_case("radio & singles")
        || radio_station.is_some_and(|st| st.trim().eq_ignore_ascii_case(trimmed))
}

fn rewrite_single_album_tag_if_needed(audio_path: &Path, new_album: &str) {
    if let Ok(p) = Probe::open(audio_path)
        && let Ok(probe) = p.guess_file_type()
        && let Ok(mut tf) = probe.read()
        && let Some(tag) = tf.primary_tag_mut()
    {
        tag.set_album(new_album.to_string());
        let _ = tag.save_to_path(audio_path, WriteOptions::default());
    }
}

fn remove_dir_if_only_junk_or_empty(dir: &Path, albums_dir: &Path, backup_dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut any_dir = false;
    let mut has_audio = false;
    let mut non_audio = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            any_dir = true;
        } else if is_audio_file(&p) {
            has_audio = true;
        } else {
            non_audio.push(p);
        }
    }
    if !any_dir && !has_audio {
        for p in non_audio {
            let _ = move_to_backup(&p, albums_dir, backup_dir);
        }
        let _ = fs::remove_dir(dir);
    }
}

/// Safely reorganizes `~/Music` into an author-first layout, backing up removed duplicates/bloat instead of deleting.
pub fn reorganize_music_library(
    albums_dir: &Path,
    radio_dir: &Path,
    backup_dir: &Path,
) -> Result<ReorgReport> {
    let mut report = ReorgReport {
        backup_dir: backup_dir.to_path_buf(),
        ..Default::default()
    };

    // 1. Move files out of `~/Music/Radio/` into proper `<Artist>/<Album>/` or `<Artist>/Singles/`.
    if radio_dir.is_dir() && canonical_or_raw(radio_dir) != canonical_or_raw(albums_dir) {
        let entries: Vec<PathBuf> = fs::read_dir(radio_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        for path in entries {
            if !is_audio_file(&path) {
                continue;
            }
            let tagged = Probe::open(&path)
                .ok()
                .and_then(|p| p.guess_file_type().ok())
                .and_then(|p| {
                    p.options(ParseOptions::new().read_cover_art(false))
                        .read()
                        .ok()
                });
            let tag = tagged
                .as_ref()
                .and_then(|f| f.primary_tag().or_else(|| f.first_tag()));
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
            let mut title = tag
                .and_then(|t| t.title().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| stem.to_string());
            let mut artist = tag
                .and_then(|t| {
                    t.artist()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_default();
            if artist.is_empty()
                && let Some((a, t)) = stem.split_once(" - ")
            {
                artist = a.trim().to_string();
                if title == stem {
                    title = t.trim().to_string();
                }
            }
            if artist.is_empty() {
                artist = "Unknown Artist".to_string();
            }
            let raw_album = tag
                .and_then(|t| t.album().map(|s| s.trim().to_string()))
                .unwrap_or_default();
            let station = tag
                .and_then(|t| t.get_string(ItemKey::InternetRadioStationName))
                .map(ToOwned::to_owned);

            let artist_dir = find_canonical_artist_dir(albums_dir, &artist);
            if artist_has_track(&artist_dir, &title).is_some() {
                move_to_backup(&path, albums_dir, backup_dir)?;
                report.removed_duplicate_files += 1;
                continue;
            }

            let fake_album = is_fake_radio_album(&raw_album, &title, station.as_deref());
            let dest_dir = if !fake_album {
                let mut chosen = artist_dir.join(sanitize_filename(&raw_album));
                if let Ok(subs) = fs::read_dir(&artist_dir) {
                    let want = normalize_match_key(&raw_album);
                    for sub in subs.flatten() {
                        let sp = sub.path();
                        if sp.is_dir()
                            && let Some(n) = sp.file_name().and_then(|x| x.to_str())
                            && normalize_match_key(n) == want
                        {
                            chosen = sp;
                            break;
                        }
                    }
                }
                chosen
            } else {
                artist_dir.join("Singles")
            };
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("opus");
            let dst = dest_dir.join(format!("{}.{ext}", sanitize_filename(&title)));
            move_file_reorg(&path, &dst)?;
            if fake_album {
                rewrite_single_album_tag_if_needed(&dst, "Singles");
            }
            report.moved_radio += 1;
        }
        remove_dir_if_only_junk_or_empty(radio_dir, albums_dir, backup_dir);
    }

    // 2. Per-artist cleanup: mixed-ext dupes, bloat track removal, duplicate edition dirs, 1-2 track singles consolidation.
    let top_dirs: Vec<PathBuf> = fs::read_dir(albums_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !is_hidden_name(p))
        .collect();

    // Pre-collect collab target folders like "Bad Omens & Poppy" where one artist already has > 1 release.
    let mut existing_multi_artists: HashSet<String> = HashSet::new();
    for adir in &top_dirs {
        let Some(aname) = adir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let sub_count = fs::read_dir(adir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && !is_hidden_name(&e.path()))
            .count();
        if sub_count >= 2 {
            existing_multi_artists.insert(normalize_artist_key(aname));
        }
    }

    for artist_dir in &top_dirs {
        let Some(artist_name) = artist_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if artist_name.eq_ignore_ascii_case("radio") {
            continue;
        }

        let subdirs: Vec<PathBuf> = fs::read_dir(artist_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !is_hidden_name(p))
            .collect();

        // Check if this is a standalone 1-2 track collab directory (e.g. "Bad Omens & Poppy")
        if subdirs.len() == 1
            && artist_name.contains('&')
        {
            let parts: Vec<&str> = artist_name.split('&').map(str::trim).collect();
            let target_partner = parts
                .into_iter()
                .rev()
                .find(|p| existing_multi_artists.contains(&normalize_artist_key(p)));
            if let Some(partner) = target_partner {
                let solo = &subdirs[0];
                let files: Vec<PathBuf> = fs::read_dir(solo)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| is_audio_file(p))
                    .collect();
                if !files.is_empty() && files.len() <= 2 {
                    let target_artist_dir = find_canonical_artist_dir(albums_dir, partner);
                    let target_singles = target_artist_dir.join("Singles");
                    for f in files {
                        let (tr, _) = read_local_track(&f, Some(artist_name), Some("Singles"));
                        if artist_has_track(&target_artist_dir, &tr.title).is_some() {
                            move_to_backup(&f, albums_dir, backup_dir)?;
                            report.removed_duplicate_files += 1;
                        } else {
                            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("opus");
                            let dst =
                                target_singles.join(format!("{}.{ext}", sanitize_filename(&tr.title)));
                            move_file_reorg(&f, &dst)?;
                            rewrite_single_album_tag_if_needed(&dst, "Singles");
                            report.consolidated_singles += 1;
                        }
                    }
                    remove_dir_if_only_junk_or_empty(solo, albums_dir, backup_dir);
                    remove_dir_if_only_junk_or_empty(artist_dir, albums_dir, backup_dir);
                    continue;
                }
            }
        }

        // 2a. Within each album folder: deduplicate mixed extensions (.opus beats .mp3) & strip bloat tracks.
        let mut album_dirs: Vec<PathBuf> = fs::read_dir(artist_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && !is_hidden_name(p)
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| !n.eq_ignore_ascii_case("singles"))
            })
            .collect();

        for alb_dir in &album_dirs {
            let mut audio_files: Vec<PathBuf> = fs::read_dir(alb_dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| is_audio_file(p))
                .collect();
            audio_files.sort();

            // Deduplicate identical tracks differing by prefix/extension (e.g. "01 - X.opus" vs "X.mp3")
            let mut by_key: HashMap<String, Vec<PathBuf>> = HashMap::new();
            for f in &audio_files {
                let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                let k = normalize_match_key(strip_track_prefix(stem));
                by_key.entry(k).or_default().push(f.clone());
            }
            let mut removed_set = HashSet::new();
            for (_k, mut group) in by_key {
                if group.len() > 1 {
                    group.sort_by(|a, b| {
                        format_priority(b)
                            .cmp(&format_priority(a))
                            .then_with(|| b.cmp(a))
                    });
                    for dupe in group.into_iter().skip(1) {
                        move_to_backup(&dupe, albums_dir, backup_dir)?;
                        removed_set.insert(dupe);
                        report.removed_duplicate_files += 1;
                    }
                }
            }
            audio_files.retain(|p| !removed_set.contains(p));

            // Strip bloat tracks (Live, Demo, Rough Mix, etc.) if at least 1 non-bloat track remains
            let mut parsed_tracks: Vec<(PathBuf, Option<u32>, String)> = audio_files
                .iter()
                .map(|p| {
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                    let num = parse_leading_track_num(stem);
                    let title = strip_track_prefix(stem).to_string();
                    (p.clone(), num, title)
                })
                .collect();
            let non_bloat_count = parsed_tracks
                .iter()
                .filter(|(_, num, t)| !is_bloat_track(*num, t))
                .count();
            if non_bloat_count > 0 && non_bloat_count < parsed_tracks.len() {
                for (p, num, t) in parsed_tracks.drain(..) {
                    if is_bloat_track(num, &t) {
                        move_to_backup(&p, albums_dir, backup_dir)?;
                        report.removed_bloat_tracks += 1;
                    }
                }
            }
            if let Ok(entries) = fs::read_dir(alb_dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_file()
                        && p.extension()
                            .and_then(|x| x.to_str())
                            .is_some_and(|x| x.eq_ignore_ascii_case("part") || x.eq_ignore_ascii_case("ytdl"))
                    {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        }

        // Clean rename if a box-set folder now only holds studio tracks (e.g. "And Justice for All (Remastered Deluxe Box Set)")
        for alb_dir in &mut album_dirs {
            let fname = alb_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let lower = fname.to_lowercase();
            if lower.contains("(remastered deluxe box set)")
                && let Some(clean_name) = fname.split('(').next().map(str::trim).filter(|s| !s.is_empty())
            {
                let renamed = artist_dir.join(sanitize_filename(clean_name));
                if !renamed.exists() && fs::rename(&*alb_dir, &renamed).is_ok() {
                    *alb_dir = renamed;
                    report.renamed_folders += 1;
                }
            }
        }

        // 2b. Remove duplicate/hard-bloat edition album folders within the same artist.
        let mut by_canonical: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for alb_dir in &album_dirs {
            if !alb_dir.is_dir() {
                continue;
            }
            let fname = alb_dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if is_hard_bloat_release(fname) {
                move_to_backup(alb_dir, albums_dir, backup_dir)?;
                report.removed_duplicate_albums += 1;
                continue;
            }
            let key = canonical_release_key(fname);
            if !key.is_empty() {
                by_canonical.entry(key).or_default().push(alb_dir.clone());
            }
        }
        for (_key, mut group) in by_canonical {
            if group.len() <= 1 {
                continue;
            }
            group.sort_by_key(|p| {
                let n = p.file_name().and_then(|x| x.to_str()).unwrap_or_default();
                (edition_bloat_rank(n), n.len())
            });
            let primary = group.remove(0);
            let primary_keys = collect_artist_track_keys(&primary);
            for other in group {
                let other_name = other.file_name().and_then(|x| x.to_str()).unwrap_or_default();
                let other_keys = collect_artist_track_keys(&other);
                let overlap = other_keys.iter().filter(|k| primary_keys.contains(*k)).count();
                let is_box_or_anniversary = edition_bloat_rank(other_name) >= 3;
                let primary_covered =
                    !primary_keys.is_empty() && overlap * 10 >= primary_keys.len() * 8;
                // Remove duplicate deluxe/anniversary folders if either:
                // - most of `other`'s tracks are already in `primary`, OR
                // - `other` is a bloated Anniversary/Box-Set edition that duplicates >=80% of `primary`.
                if other_keys.is_empty()
                    || overlap * 10 >= other_keys.len() * 6
                    || (is_box_or_anniversary && primary_covered)
                {
                    move_to_backup(&other, albums_dir, backup_dir)?;
                    report.removed_duplicate_albums += 1;
                }
            }
        }

        // 2c. Consolidate 1-2 track release folders (singles) into `<Artist>/Singles/`, skipping tracks already present in multi-track albums.
        let remaining_dirs: Vec<PathBuf> = fs::read_dir(artist_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && !is_hidden_name(p)
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| !n.eq_ignore_ascii_case("singles"))
            })
            .collect();

        let mut album_track_keys: HashSet<String> = HashSet::new();
        let mut small_single_dirs = Vec::new();
        for d in &remaining_dirs {
            let audio_files: Vec<PathBuf> = fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| is_audio_file(p))
                .collect();
            if audio_files.len() >= 3 {
                for f in audio_files {
                    let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                    let k = normalize_match_key(strip_track_prefix(stem));
                    if !k.is_empty() {
                        album_track_keys.insert(k);
                    }
                }
            } else if !audio_files.is_empty() {
                small_single_dirs.push((d.clone(), audio_files));
            }
        }

        let singles_dir = artist_dir.join("Singles");
        let mut existing_singles = collect_artist_track_keys(&singles_dir);
        for (dir, files) in small_single_dirs {
            for f in files {
                let (tr, _) = read_local_track(&f, Some(artist_name), Some("Singles"));
                let k = normalize_match_key(&tr.title);
                if !k.is_empty() && (album_track_keys.contains(&k) || existing_singles.contains(&k)) {
                    move_to_backup(&f, albums_dir, backup_dir)?;
                    report.removed_duplicate_files += 1;
                } else {
                    let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("opus");
                    let dst = singles_dir.join(format!("{}.{ext}", sanitize_filename(&tr.title)));
                    move_file_reorg(&f, &dst)?;
                    rewrite_single_album_tag_if_needed(&dst, "Singles");
                    if !k.is_empty() {
                        existing_singles.insert(k);
                    }
                    report.consolidated_singles += 1;
                }
            }
            remove_dir_if_only_junk_or_empty(&dir, albums_dir, backup_dir);
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_track_prefix_and_normalize_handle_numbers_and_punctuation() {
        assert_eq!(strip_track_prefix("06 - Feel Good Inc."), "Feel Good Inc.");
        assert_eq!(strip_track_prefix("01. Intro"), "Intro");
        assert_eq!(strip_track_prefix("19-2000"), "19-2000");

        assert_eq!(
            normalize_match_key(strip_track_prefix("06 - Feel Good Inc.")),
            normalize_match_key("Feel Good Inc")
        );
        assert_eq!(normalize_match_key("19-2000"), "192000");
        assert_eq!(parse_leading_track_num("06 - Feel Good Inc."), Some(6));
        assert_eq!(parse_leading_track_num("19-2000"), None);
    }

    #[test]
    fn find_local_album_track_matches_vibes_opus_and_existing_mp3_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        let albums_dir = tmp.path();

        // Existing user style: Album - <Name>/Title..mp3
        let demon_days = albums_dir.join("Album - Demon Days");
        fs::create_dir_all(&demon_days).unwrap();
        let feel_good = demon_days.join("Feel Good Inc..mp3");
        fs::write(&feel_good, b"dummy").unwrap();
        let n19_2000 = demon_days.join("19-2000.mp3");
        fs::write(&n19_2000, b"dummy").unwrap();

        // Vibes Artist/Album style: 03 - Title.opus
        let plastic_beach = albums_dir.join("Gorillaz").join("Plastic Beach");
        fs::create_dir_all(&plastic_beach).unwrap();
        let rhinestone = plastic_beach.join("03 - Rhinestone Eyes.opus");
        fs::write(&rhinestone, b"dummy").unwrap();

        assert_eq!(
            find_local_album_track(albums_dir, "Gorillaz", "Demon Days", 6, "Feel Good Inc"),
            Some(feel_good)
        );
        assert_eq!(
            find_local_album_track(albums_dir, "Gorillaz", "Demon Days", 5, "19-2000"),
            Some(n19_2000)
        );
        assert_eq!(
            find_local_album_track(
                albums_dir,
                "Gorillaz",
                "Plastic Beach",
                3,
                "Rhinestone Eyes"
            ),
            Some(rhinestone)
        );
        assert_eq!(
            find_local_album_track(albums_dir, "Gorillaz", "Plastic Beach", 4, "Missing"),
            None
        );
    }

    #[test]
    fn scan_local_library_separates_albums_and_radio_singles() {
        let tmp = tempfile::tempdir().unwrap();
        let albums_dir = tmp.path();
        let radio_dir = albums_dir.join("Radio");
        fs::create_dir_all(&radio_dir).unwrap();

        let hybrid = albums_dir.join("Album - Hybrid Theory");
        fs::create_dir_all(&hybrid).unwrap();
        fs::write(hybrid.join("02 - One Step Closer.opus"), b"dummy").unwrap();
        fs::write(hybrid.join("01 - Papercut.opus"), b"dummy").unwrap();
        fs::write(hybrid.join("cover.jpg"), b"not audio").unwrap();

        fs::write(radio_dir.join("Burzum - Dunkelheit.opus"), b"dummy").unwrap();
        fs::write(albums_dir.join("Loose Demo.flac"), b"dummy").unwrap();

        let snap = scan_local_library(albums_dir, &radio_dir);
        assert_eq!(snap.albums.len(), 1);
        let alb = &snap.albums[0];
        assert_eq!(alb.title, "Hybrid Theory");
        assert_eq!(alb.tracks.len(), 2);
        assert_eq!(alb.tracks[0].title, "Papercut");
        assert_eq!(alb.tracks[0].track_number, Some(1));
        assert_eq!(alb.tracks[1].title, "One Step Closer");
        assert_eq!(alb.tracks[1].track_number, Some(2));
        assert_eq!(snap.singles.len(), 2);
    }

    #[test]
    fn scan_local_library_handles_multibyte_utf8_folder_names() {
        let tmp = tempfile::tempdir().unwrap();
        let albums_dir = tmp.path();
        let radio_dir = albums_dir.join("Radio");
        fs::create_dir_all(&radio_dir).unwrap();

        let voidstar = albums_dir.join("Grim Salvo").join("VOIDST★R");
        fs::create_dir_all(&voidstar).unwrap();
        fs::write(voidstar.join("BAPHOMET.mp3"), b"dummy").unwrap();

        let cyrillic = albums_dir.join("Артист").join("Альбом");
        fs::create_dir_all(&cyrillic).unwrap();
        fs::write(cyrillic.join("01 - Трек.mp3"), b"dummy").unwrap();

        let snap = scan_local_library(albums_dir, &radio_dir);
        assert_eq!(snap.albums.len(), 2);
    }

    #[test]
    fn scan_groups_artists_and_reorg_cleans_radio_singles_and_bloat() {
        let tmp = tempfile::tempdir().unwrap();
        let albums_dir = tmp.path().join("Music");
        let radio_dir = albums_dir.join("Radio");
        let backup_dir = tmp.path().join("backup");

        // Metallica with studio album + bloated tracks
        let ajfa = albums_dir
            .join("Metallica")
            .join("And Justice for All (Remastered Deluxe Box Set)");
        fs::create_dir_all(&ajfa).unwrap();
        fs::write(ajfa.join("01 - Blackened.opus"), b"a").unwrap();
        fs::write(ajfa.join("02 - And Justice for All.opus"), b"a2").unwrap();
        fs::write(ajfa.join("04 - One.opus"), b"a4").unwrap();
        fs::write(ajfa.join("10 - The Ecstasy of Gold.opus"), b"b").unwrap();
        fs::write(ajfa.join("11 - Blackened (Live - Seattle '89).opus"), b"c").unwrap();

        // Poppy with multi-track album + redundant 1-track single album
        let neg = albums_dir.join("Poppy").join("Negative Spaces");
        fs::create_dir_all(&neg).unwrap();
        fs::write(neg.join("01 - have you had enough.opus"), b"a").unwrap();
        fs::write(neg.join("02 - new way out.opus"), b"b").unwrap();
        fs::write(neg.join("03 - halo.opus"), b"c").unwrap();
        let dup_single = albums_dir.join("Poppy").join("new way out");
        fs::create_dir_all(&dup_single).unwrap();
        fs::write(dup_single.join("01 - new way out.opus"), b"d").unwrap();
        let unique_single = albums_dir.join("Poppy").join("Her");
        fs::create_dir_all(&unique_single).unwrap();
        fs::write(unique_single.join("01 - Her.opus"), b"e").unwrap();

        // Radio loose track
        fs::create_dir_all(&radio_dir).unwrap();
        fs::write(radio_dir.join("Jinjer - Someone's Daughter.opus"), b"r").unwrap();

        let report = reorganize_music_library(&albums_dir, &radio_dir, &backup_dir).unwrap();
        assert_eq!(report.moved_radio, 1);
        assert_eq!(report.removed_bloat_tracks, 2);
        assert!(report.removed_duplicate_files >= 1);
        assert!(report.consolidated_singles >= 1);

        assert!(albums_dir.join("Metallica").join("And Justice for All").join("01 - Blackened.opus").is_file());
        assert!(albums_dir.join("Poppy").join("Singles").join("Her.opus").is_file());
        assert!(albums_dir.join("Jinjer").join("Singles").join("Someone's Daughter.opus").is_file());
        assert!(!dup_single.exists());
        assert!(!unique_single.exists());

        let snap = scan_local_library(&albums_dir, &radio_dir);
        assert!(snap.artists.iter().any(|a| a.name == "Metallica" && a.albums.len() == 1));
        assert!(snap.artists.iter().any(|a| a.name == "Poppy" && a.albums.len() == 1 && a.singles.len() == 1));
        assert!(snap.artists.iter().any(|a| a.name == "Jinjer" && a.singles.len() == 1));
    }
}
