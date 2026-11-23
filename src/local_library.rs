use crate::albums::resolve_album_dir;
use crate::artwork::{resolve_local_album_art, resolve_local_single_art};
use crate::config::AlbumFolderMode;
use crate::downloader::sanitize_filename;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalLibrarySnapshot {
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
        let (fallback_artist, fallback_album) = infer_album_fallback(albums_dir, dir);
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

    LocalLibrarySnapshot { albums, singles }
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
}
