use crate::local_library::{is_audio_file, read_local_track};
use anyhow::{Context, Result};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagExt, TagType};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const LRCLIB_USER_AGENT: &str = "Vibes/0.1.0 (https://github.com/vovakaravov/Vibes)";

#[derive(Debug, Clone, PartialEq)]
pub struct SyncedLine {
    pub time_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LyricsKind {
    Synced(Vec<SyncedLine>),
    Plain(Vec<String>),
    Instrumental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsOrigin {
    Embedded,
    Lrclib,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricsDocument {
    pub raw: String,
    pub kind: LyricsKind,
    pub origin: LyricsOrigin,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum LyricsState {
    #[default]
    Empty,
    Loading,
    Ready(LyricsDocument),
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricsQuery {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration_secs: Option<f64>,
}

impl LyricsQuery {
    pub fn cache_key(&self) -> String {
        let a = normalize_for_match(&self.artist);
        let t = normalize_for_match(&clean_track_for_lookup(&self.title));
        let al = self
            .album
            .as_deref()
            .map(normalize_for_match)
            .unwrap_or_default();
        let d = self
            .duration_secs
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| v.round() as i64)
            .unwrap_or(0);
        format!("{a}|{t}|{al}|{d}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LrclibResult {
    Synced(String),
    Plain(String),
    Instrumental,
    NotFound,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LyricsBackfillProgress {
    pub active: bool,
    pub processed: usize,
    pub total: usize,
    pub embedded: usize,
    pub skipped_existing: usize,
    pub instrumental: usize,
    pub not_found: usize,
    pub failed: usize,
    pub current_track: String,
}

impl LyricsBackfillProgress {
    pub fn summary(&self) -> String {
        if self.active {
            let track = if self.current_track.is_empty() {
                "Scanning library…".to_string()
            } else {
                self.current_track.clone()
            };
            format!(
                "Backfilling lyrics ({}/{}) · {} embedded · {} skipped · {track}",
                self.processed, self.total, self.embedded, self.skipped_existing
            )
        } else {
            format!(
                "Last run: {} embedded · {} already tagged · {} instrumental · {} not found{}",
                self.embedded,
                self.skipped_existing,
                self.instrumental,
                self.not_found,
                if self.failed > 0 {
                    format!(" · {} failed", self.failed)
                } else {
                    String::new()
                }
            )
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LrclibEntry {
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

pub fn new_http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .max_redirects(4)
        .build()
        .into()
}

fn strip_paren_suffix(input: &str, open: char, close: char, keywords: &[&str]) -> Option<String> {
    let trimmed = input.trim_end();
    if !trimmed.ends_with(close) {
        return None;
    }
    let start = trimmed.rfind(open)?;
    let inner = trimmed[start + open.len_utf8()..trimmed.len() - close.len_utf8()]
        .trim()
        .to_lowercase();
    if keywords.iter().any(|k| inner.contains(k)) {
        Some(trimmed[..start].trim().to_string())
    } else {
        None
    }
}

/// Removes video/release noise tags (e.g. `(Official Video)`, `(feat. X)`, `(2011 Remaster)`) for cleaner lyrics matching.
pub fn clean_track_for_lookup(raw: &str) -> String {
    const TAG_KEYWORDS: &[&str] = &[
        "official",
        "lyric",
        "lyrics",
        "audio",
        "video",
        "visualizer",
        "remaster",
        "feat.",
        "feat ",
        "ft.",
        "ft ",
        "explicit",
        "clean",
        "hd",
        "hq",
        "4k",
    ];
    let mut s = raw.trim().to_string();
    loop {
        let before = s.clone();
        if let Some(next) = strip_paren_suffix(&s, '(', ')', TAG_KEYWORDS)
            .or_else(|| strip_paren_suffix(&s, '[', ']', TAG_KEYWORDS))
        {
            s = next;
        }
        let lower = s.to_lowercase();
        for sep in [" feat. ", " ft. ", " - remaster", " - official"] {
            if let Some(idx) = lower.find(sep) {
                s = s[..idx].trim().to_string();
                break;
            }
        }
        if s == before || s.is_empty() {
            break;
        }
    }
    if s.is_empty() {
        raw.trim().to_string()
    } else {
        s
    }
}

fn normalize_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn token_overlap_ratio(a: &str, b: &str) -> f32 {
    let na = normalize_for_match(a);
    let nb = normalize_for_match(b);
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    let ta: HashSet<&str> = na.split_whitespace().collect();
    let tb: HashSet<&str> = nb.split_whitespace().collect();
    let inter = ta.intersection(&tb).count() as f32;
    let denom = ta.len().max(tb.len()) as f32;
    if denom > 0.0 {
        inter / denom
    } else {
        0.0
    }
}

fn parse_lrc_timestamp_secs(tag_inner: &str) -> Option<f64> {
    let (min_str, sec_part) = tag_inner.split_once(':')?;
    if min_str.is_empty() || !min_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mins: f64 = min_str.parse().ok()?;
    let secs: f64 = if let Some((whole, frac)) = sec_part.split_once('.').or_else(|| sec_part.split_once(':')) {
        if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) || frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let s: f64 = whole.parse().ok()?;
        let denom = 10f64.powi(frac.len().min(6) as i32);
        let f: f64 = frac[..frac.len().min(6)].parse().ok()?;
        s + f / denom
    } else {
        if sec_part.is_empty() || !sec_part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        sec_part.parse().ok()?
    };
    if !(0.0..60.0).contains(&secs) {
        return None;
    }
    Some(mins * 60.0 + secs)
}

fn strip_inline_word_timestamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('>') {
            let inner = &after[..close];
            if parse_lrc_timestamp_secs(inner).is_some() {
                rest = &after[close + 1..];
                continue;
            }
        }
        out.push('<');
        rest = after;
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Parses standard LRC timestamps (`[mm:ss.xx]`, multi-tags per line, `[offset:ms]`) into sorted synced lines.
pub fn parse_lrc(raw: &str) -> Vec<SyncedLine> {
    let mut offset_ms: f64 = 0.0;
    let mut raw_entries: Vec<(f64, String)> = Vec::new();

    for line in raw.lines() {
        let mut cursor = line.trim();
        if cursor.is_empty() {
            continue;
        }

        let mut timestamps = Vec::new();
        while cursor.starts_with('[') {
            let Some(close) = cursor.find(']') else {
                break;
            };
            let inner = cursor[1..close].trim();
            if let Some(ts) = parse_lrc_timestamp_secs(inner) {
                timestamps.push(ts);
                cursor = cursor[close + 1..].trim_start();
            } else if let Some((k, v)) = inner.split_once(':') {
                if k.trim().eq_ignore_ascii_case("offset")
                    && let Ok(ms) = v.trim().parse::<f64>()
                {
                    offset_ms = ms;
                }
                cursor = "";
                break;
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            continue;
        }
        let text = strip_inline_word_timestamps(cursor);
        if text.is_empty() {
            continue;
        }
        for ts in timestamps {
            raw_entries.push((ts, text.clone()));
        }
    }

    let shift_secs = offset_ms / 1000.0;
    let mut parsed: Vec<SyncedLine> = raw_entries
        .into_iter()
        .map(|(ts, text)| SyncedLine {
            time_secs: (ts - shift_secs).max(0.0),
            text,
        })
        .collect();
    parsed.sort_by(|a, b| {
        a.time_secs
            .partial_cmp(&b.time_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    parsed
}

/// Classifies raw lyrics text into synced LRC, plain lines, or instrumental marker.
pub fn parse_lyrics_text(raw: &str, origin: LyricsOrigin) -> Option<LyricsDocument> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("[instrumental]") || trimmed.eq_ignore_ascii_case("instrumental") {
        return Some(LyricsDocument {
            raw: "[Instrumental]".to_string(),
            kind: LyricsKind::Instrumental,
            origin,
        });
    }

    let synced = parse_lrc(trimmed);
    if synced.len() >= 2 {
        return Some(LyricsDocument {
            raw: trimmed.to_string(),
            kind: LyricsKind::Synced(synced),
            origin,
        });
    }

    let mut plain = Vec::new();
    let mut prev_blank = false;
    for line in trimmed.lines() {
        let clean = line.trim();
        if clean.is_empty() {
            if !prev_blank && !plain.is_empty() {
                plain.push(String::new());
                prev_blank = true;
            }
        } else {
            plain.push(clean.to_string());
            prev_blank = false;
        }
    }
    if matches!(plain.last(), Some(s) if s.is_empty()) {
        plain.pop();
    }
    if plain.is_empty() {
        return None;
    }

    Some(LyricsDocument {
        raw: trimmed.to_string(),
        kind: LyricsKind::Plain(plain),
        origin,
    })
}

/// Returns the active line index for playback `pos_secs` (with a slight visual lead).
pub fn active_synced_line_index(lines: &[SyncedLine], pos_secs: f64) -> Option<usize> {
    if lines.is_empty() || pos_secs < 0.0 {
        return None;
    }
    let t = pos_secs + 0.18;
    let mut active = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.time_secs <= t {
            active = Some(idx);
        } else {
            break;
        }
    }
    active
}

/// Writes lyrics onto a `lofty::Tag` using the container's native standard key (`USLT` for ID3v2; `LYRICS`/`©lyr` otherwise).
pub fn set_lyrics_on_tag(tag: &mut Tag, lyrics: &str) {
    let key = match tag.tag_type() {
        TagType::Id3v2 => ItemKey::UnsyncLyrics,
        _ => ItemKey::Lyrics,
    };
    tag.remove_key(ItemKey::Lyrics);
    tag.remove_key(ItemKey::UnsyncLyrics);
    tag.insert_text(key, lyrics.trim().to_string());
}

/// Reads embedded lyrics from an audio file (`ItemKey::Lyrics` or `ItemKey::UnsyncLyrics`).
pub fn read_embedded_lyrics(audio_path: &Path) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let tagged = Probe::open(audio_path)
            .ok()?
            .guess_file_type()
            .ok()?
            .options(ParseOptions::new().read_properties(false).read_cover_art(false))
            .read()
            .ok()?;

        for tag in [tagged.primary_tag(), tagged.first_tag()].into_iter().flatten() {
            for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
                if let Some(val) = tag.get_string(key) {
                    let clean = val.trim();
                    if !clean.is_empty() {
                        return Some(clean.to_string());
                    }
                }
            }
        }
        None
    }))
    .ok()
    .flatten()
}

/// Embeds lyrics into an existing audio file while preserving existing metadata tags and cover art.
pub fn embed_lyrics_in_file(audio_path: &Path, lyrics: &str) -> Result<()> {
    let text = lyrics.trim().to_string();
    anyhow::ensure!(!text.is_empty(), "Cannot embed empty lyrics");

    let path_owned = audio_path.to_path_buf();
    let inner = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || -> Result<()> {
        let mut tagged_file = Probe::open(&path_owned)
            .with_context(|| format!("Failed to open {}", path_owned.display()))?
            .guess_file_type()?
            .read()?;

        let tag_type = tagged_file.primary_tag_type();
        if tagged_file.primary_tag_mut().is_none() {
            tagged_file.insert_tag(Tag::new(tag_type));
        }
        let tag = tagged_file
            .primary_tag_mut()
            .context("No writable metadata tag available")?;

        set_lyrics_on_tag(tag, &text);
        tag.save_to_path(&path_owned, WriteOptions::default())
            .with_context(|| format!("Failed to embed lyrics into {}", path_owned.display()))?;
        Ok(())
    }));

    match inner {
        Ok(res) => res,
        Err(_) => anyhow::bail!("Metadata tagger panicked on {}", audio_path.display()),
    }
}

fn entry_to_lrclib_result(entry: &LrclibEntry) -> Option<LrclibResult> {
    if let Some(ref synced) = entry.synced_lyrics {
        let t = synced.trim();
        if !t.is_empty() {
            return Some(LrclibResult::Synced(t.to_string()));
        }
    }
    if let Some(ref plain) = entry.plain_lyrics {
        let t = plain.trim();
        if !t.is_empty() {
            return Some(LrclibResult::Plain(t.to_string()));
        }
    }
    if entry.instrumental {
        return Some(LrclibResult::Instrumental);
    }
    None
}

/// Scores LRCLIB search candidates by artist/title overlap, duration proximity, and synced-lyrics availability.
pub fn pick_best_lrclib_candidate(entries: &[LrclibEntry], query: &LyricsQuery) -> LrclibResult {
    let want_title = clean_track_for_lookup(&query.title);
    let want_artist = query.artist.trim();
    let want_dur = query.duration_secs.filter(|d| d.is_finite() && *d > 0.0);

    let mut best: Option<(i32, i32, LrclibResult)> = None;

    for entry in entries {
        let Some(res) = entry_to_lrclib_result(entry) else {
            continue;
        };

        let dur_diff = match (want_dur, entry.duration.filter(|d| d.is_finite() && *d > 0.0)) {
            (Some(q), Some(e)) => {
                let d = (q - e).abs();
                if d > 12.0 {
                    continue;
                }
                Some(d)
            }
            _ => None,
        };

        let entry_title = clean_track_for_lookup(&entry.track_name);
        let title_sim = token_overlap_ratio(&want_title, &entry_title)
            .max(token_overlap_ratio(&want_title, &entry.track_name));
        if title_sim < 0.45 {
            continue;
        }

        let artist_sim = if want_artist.is_empty() {
            0.5
        } else {
            let sim = token_overlap_ratio(want_artist, &entry.artist_name);
            if sim < 0.35 {
                continue;
            }
            sim
        };

        let kind_bonus = match res {
            LrclibResult::Synced(_) => 1000,
            LrclibResult::Plain(_) => 400,
            LrclibResult::Instrumental => 100,
            LrclibResult::NotFound => 0,
        };
        let dur_bonus = match dur_diff {
            Some(d) if d <= 2.5 => 200,
            Some(d) if d <= 6.0 => 100,
            _ => 0,
        };
        let score = kind_bonus + dur_bonus + ((title_sim * 300.0) as i32) + ((artist_sim * 250.0) as i32);
        let neg_diff = -(dur_diff.unwrap_or(6.0).round() as i32);

        let replace = match &best {
            None => true,
            Some((b_score, b_neg_diff, _)) => {
                (score, neg_diff) > (*b_score, *b_neg_diff)
            }
        };
        if replace {
            best = Some((score, neg_diff, res));
        }
    }

    best.map(|(_, _, r)| r).unwrap_or(LrclibResult::NotFound)
}

fn enc(v: &str) -> String {
    utf8_percent_encode(v.trim(), NON_ALPHANUMERIC).to_string()
}

fn fetch_json_string(agent: &ureq::Agent, url: &str) -> Option<String> {
    let mut resp = agent
        .get(url)
        .header("User-Agent", LRCLIB_USER_AGENT)
        .call()
        .ok()?;
    resp.body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_string()
        .ok()
}

/// Fetches synced/plain lyrics or instrumental status from LRCLIB (`/api/get` with `/api/search` fallback).
pub fn fetch_lrclib_lyrics(agent: &ureq::Agent, query: &LyricsQuery) -> LrclibResult {
    let clean_title = clean_track_for_lookup(&query.title);
    let clean_artist = query.artist.trim();
    if clean_title.is_empty() {
        return LrclibResult::NotFound;
    }

    if !clean_artist.is_empty() {
        let mut get_url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}",
            enc(clean_artist),
            enc(&clean_title)
        );
        if let Some(ref album) = query.album
            && !album.trim().is_empty()
        {
            get_url.push_str(&format!("&album_name={}", enc(album)));
        }
        if let Some(dur) = query.duration_secs.filter(|d| d.is_finite() && *d > 0.0) {
            get_url.push_str(&format!("&duration={}", dur.round() as u64));
        }

        if let Some(body) = fetch_json_string(agent, &get_url)
            && let Ok(entry) = serde_json::from_str::<LrclibEntry>(&body)
            && let Some(res) = entry_to_lrclib_result(&entry)
        {
            if matches!(res, LrclibResult::Synced(_)) {
                return res;
            }
            // If exact /api/get returned only Plain/Instrumental, check /api/search for a Synced hit first.
            let search_url = format!(
                "https://lrclib.net/api/search?artist_name={}&track_name={}",
                enc(clean_artist),
                enc(&clean_title)
            );
            if let Some(sbody) = fetch_json_string(agent, &search_url)
                && let Ok(entries) = serde_json::from_str::<Vec<LrclibEntry>>(&sbody)
            {
                let cand = pick_best_lrclib_candidate(&entries, query);
                if matches!(cand, LrclibResult::Synced(_)) {
                    return cand;
                }
            }
            return res;
        }

        let search_url = format!(
            "https://lrclib.net/api/search?artist_name={}&track_name={}",
            enc(clean_artist),
            enc(&clean_title)
        );
        if let Some(body) = fetch_json_string(agent, &search_url)
            && let Ok(entries) = serde_json::from_str::<Vec<LrclibEntry>>(&body)
        {
            let picked = pick_best_lrclib_candidate(&entries, query);
            if picked != LrclibResult::NotFound {
                return picked;
            }
        }
    }

    let q_str = if clean_artist.is_empty() {
        clean_title
    } else {
        format!("{clean_artist} {clean_title}")
    };
    let fallback_url = format!("https://lrclib.net/api/search?q={}", enc(&q_str));
    if let Some(body) = fetch_json_string(agent, &fallback_url)
        && let Ok(entries) = serde_json::from_str::<Vec<LrclibEntry>>(&body)
    {
        return pick_best_lrclib_candidate(&entries, query);
    }

    LrclibResult::NotFound
}

pub fn lrclib_to_document(res: LrclibResult) -> Option<LyricsDocument> {
    match res {
        LrclibResult::Synced(s) | LrclibResult::Plain(s) => {
            parse_lyrics_text(&s, LyricsOrigin::Lrclib)
        }
        LrclibResult::Instrumental => Some(LyricsDocument {
            raw: "[Instrumental]".to_string(),
            kind: LyricsKind::Instrumental,
            origin: LyricsOrigin::Lrclib,
        }),
        LrclibResult::NotFound => None,
    }
}

pub fn lrclib_text_to_embed(res: &LrclibResult) -> Option<&str> {
    match res {
        LrclibResult::Synced(s) | LrclibResult::Plain(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        }
        LrclibResult::Instrumental | LrclibResult::NotFound => None,
    }
}

fn collect_audio_files_recursive(
    dir: &Path,
    root: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<(PathBuf, PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_audio_files_recursive(&p, root, seen, out);
        } else if is_audio_file(&p) && seen.insert(p.clone()) {
            out.push((p, root.to_path_buf()));
        }
    }
}

fn query_from_local_path(path: &Path, root: &Path) -> LyricsQuery {
    let rel_comps: Vec<String> = path
        .strip_prefix(root)
        .ok()
        .map(|rel| {
            rel.components()
                .filter_map(|c| c.as_os_str().to_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let (fallback_artist, fallback_album) = match rel_comps.as_slice() {
        [artist, album, _, ..] => (
            Some(artist.as_str()),
            (!album.eq_ignore_ascii_case("singles")).then_some(album.as_str()),
        ),
        [artist, _] => (Some(artist.as_str()), None),
        _ => (None, None),
    };

    let (track, _) = read_local_track(path, fallback_artist, fallback_album);
    let artist = if track.artist.eq_ignore_ascii_case("Unknown Artist") {
        String::new()
    } else {
        track.artist
    };
    let album = (!track.album.trim().is_empty()).then_some(track.album);
    LyricsQuery {
        artist,
        title: track.title,
        album,
        duration_secs: track.duration_secs,
    }
}

/// Scans `roots`, skips files that already carry embedded lyrics, and embeds LRCLIB lyrics in place.
pub fn run_library_lyrics_backfill(
    roots: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(LyricsBackfillProgress),
) {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for root in roots {
        if root.is_dir() {
            collect_audio_files_recursive(&root, &root, &mut seen, &mut files);
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let total = files.len();
    let mut prog = LyricsBackfillProgress {
        active: true,
        total,
        ..Default::default()
    };
    on_progress(prog.clone());

    let mut pending: Vec<(PathBuf, LyricsQuery)> = Vec::new();
    for (path, root) in files {
        if cancel.load(Ordering::Acquire) {
            prog.active = false;
            on_progress(prog);
            return;
        }
        if read_embedded_lyrics(&path).is_some() {
            prog.processed += 1;
            prog.skipped_existing += 1;
        } else {
            let query = query_from_local_path(&path, &root);
            pending.push((path, query));
        }
    }
    on_progress(prog.clone());

    let agent = new_http_agent();
    for (path, query) in pending {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        prog.current_track = if query.artist.is_empty() {
            query.title.clone()
        } else {
            format!("{} - {}", query.artist, query.title)
        };
        on_progress(prog.clone());

        match fetch_lrclib_lyrics(&agent, &query) {
            LrclibResult::Synced(text) | LrclibResult::Plain(text) => {
                match embed_lyrics_in_file(&path, &text) {
                    Ok(()) => prog.embedded += 1,
                    Err(_) => prog.failed += 1,
                }
            }
            LrclibResult::Instrumental => prog.instrumental += 1,
            LrclibResult::NotFound => prog.not_found += 1,
        }
        prog.processed += 1;
        on_progress(prog.clone());

        for _ in 0..4 {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    prog.active = false;
    prog.current_track.clear();
    on_progress(prog);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::Accessor;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn parses_lrc_timestamps_multi_tags_offset_and_active_line() {
        let lrc = "\
[ti:Papercut]
[ar:Linkin Park]
[offset:500]
[00:10.50][00:30.00] Repeated chorus
[00:15.250] <00:15.25> Word synced <00:16.00> line
[00:20] Simple seconds line
";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 4);
        // offset:500 shifts 10.50 -> 10.00, 15.25 -> 14.75, 20.00 -> 19.50, 30.00 -> 29.50
        assert!((lines[0].time_secs - 10.0).abs() < 1e-6);
        assert_eq!(lines[0].text, "Repeated chorus");
        assert!((lines[1].time_secs - 14.75).abs() < 1e-6);
        assert_eq!(lines[1].text, "Word synced  line");
        assert!((lines[2].time_secs - 19.5).abs() < 1e-6);
        assert_eq!(lines[2].text, "Simple seconds line");
        assert!((lines[3].time_secs - 29.5).abs() < 1e-6);

        assert_eq!(active_synced_line_index(&lines, 2.0), None);
        assert_eq!(active_synced_line_index(&lines, 9.9), Some(0));
        assert_eq!(active_synced_line_index(&lines, 15.0), Some(1));
        assert_eq!(active_synced_line_index(&lines, 40.0), Some(3));

        let doc = parse_lyrics_text(lrc, LyricsOrigin::Embedded).unwrap();
        assert!(matches!(doc.kind, LyricsKind::Synced(_)));
        let inst = parse_lyrics_text("[Instrumental]", LyricsOrigin::Lrclib).unwrap();
        assert_eq!(inst.kind, LyricsKind::Instrumental);
        let plain = parse_lyrics_text("Line one\n\n\nLine two\n", LyricsOrigin::Lrclib).unwrap();
        assert_eq!(
            plain.kind,
            LyricsKind::Plain(vec![
                "Line one".to_string(),
                String::new(),
                "Line two".to_string()
            ])
        );
    }

    #[test]
    fn picks_best_lrclib_candidate_respecting_duration_and_synced_preference() {
        let query = LyricsQuery {
            artist: "Linkin Park".to_string(),
            title: "Papercut (Official HD Video)".to_string(),
            album: Some("Hybrid Theory".to_string()),
            duration_secs: Some(185.0),
        };
        let entries = vec![
            LrclibEntry {
                track_name: "Papercut".to_string(),
                artist_name: "Linkin Park".to_string(),
                duration: Some(230.0), // > 12s diff -> rejected
                instrumental: false,
                plain_lyrics: Some("Wrong length".to_string()),
                synced_lyrics: Some("[00:10.00] Wrong length\n[00:12.00] Line".to_string()),
            },
            LrclibEntry {
                track_name: "Papercut".to_string(),
                artist_name: "Linkin Park".to_string(),
                duration: Some(186.0),
                instrumental: false,
                plain_lyrics: Some("Plain only".to_string()),
                synced_lyrics: None,
            },
            LrclibEntry {
                track_name: "Papercut".to_string(),
                artist_name: "LINKIN PARK".to_string(),
                duration: Some(185.0),
                instrumental: false,
                plain_lyrics: Some("Why does it feel".to_string()),
                synced_lyrics: Some("[00:23.12] Why does it feel like night today?\n[00:24.58] Somethin'".to_string()),
            },
        ];

        assert_eq!(
            pick_best_lrclib_candidate(&entries, &query),
            LrclibResult::Synced(
                "[00:23.12] Why does it feel like night today?\n[00:24.58] Somethin'".to_string()
            )
        );
    }

    #[test]
    fn embeds_and_reads_lyrics_in_real_opus_and_mp3_preserving_tags() {
        let tmp = TempDir::new().unwrap();
        let cover_bytes = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
        let lrc = "[00:01.00] Hello world\n[00:02.00] Preserved cover and tags";

        for (name, codec) in [("test.opus", "libopus"), ("test.mp3", "libmp3lame")] {
            let p = tmp.path().join(name);
            let st = Command::new("ffmpeg")
                .args([
                    "-y", "-v", "error", "-f", "lavfi", "-i",
                    "sine=frequency=440:duration=0.4", "-c:a", codec,
                ])
                .arg(&p)
                .status()
                .unwrap();
            assert!(st.success(), "ffmpeg failed for {name}");

            // Seed initial title, artist & cover art
            {
                let mut tf = Probe::open(&p).unwrap().guess_file_type().unwrap().read().unwrap();
                let tt = tf.primary_tag_type();
                if tf.primary_tag_mut().is_none() {
                    tf.insert_tag(Tag::new(tt));
                }
                let tag = tf.primary_tag_mut().unwrap();
                tag.set_title("Test Title".to_string());
                tag.set_artist("Test Artist".to_string());
                tag.push_picture(
                    Picture::unchecked(cover_bytes.clone())
                        .pic_type(PictureType::CoverFront)
                        .mime_type(MimeType::Jpeg)
                        .build(),
                );
                tag.save_to_path(&p, WriteOptions::default()).unwrap();
            }

            embed_lyrics_in_file(&p, lrc).unwrap();
            assert_eq!(read_embedded_lyrics(&p).as_deref(), Some(lrc));

            let tf = Probe::open(&p).unwrap().guess_file_type().unwrap().read().unwrap();
            let tag = tf.primary_tag().unwrap();
            assert_eq!(tag.title().as_deref(), Some("Test Title"));
            assert_eq!(tag.artist().as_deref(), Some("Test Artist"));
            assert!(!tag.pictures().is_empty(), "cover art missing for {name}");
        }
    }
}
