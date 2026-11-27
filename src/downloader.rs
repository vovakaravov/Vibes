use crate::config::{FailedDownloadItem, SaveAudioFormat};
use crate::discography::find_canonical_artist_dir;
use crate::local_library::{artist_has_track, normalize_match_key};
use crate::youtube::{YoutubeItem, parse_ytdlp_ndjson};
use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTrack {
    pub artist: Option<String>,
    pub title: String,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    Searching,
    Downloading(String),
    AlreadyExists(PathBuf),
    Saved(PathBuf),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub id: u64,
    pub label: String,
    pub source_station: String,
    pub direct_url: Option<String>,
    pub parsed: ParsedTrack,
    pub status: DownloadStatus,
    pub created_at: String,
}

const NOISE_PHRASES: &[&str] = &[
    "commercial",
    "advertisement",
    "ad break",
    "station id",
    "jingle",
    "promo",
    "tunein",
    "streamguys",
    "shoutcast",
    "icecast",
    "unknown artist",
    "unknown title",
    "live stream",
];

fn strip_trailing_noise_tags(input: &str) -> String {
    let mut s = input.trim().to_string();
    loop {
        let lower = s.to_lowercase();
        let trimmed_len = s.len();
        for suffix in [
            "(official video)",
            "(official music video)",
            "(official audio)",
            "(official lyric video)",
            "(lyric video)",
            "(lyrics)",
            "[official video]",
            "[official music video]",
            "[official audio]",
            "[lyrics]",
            "[hd]",
            "[hq]",
            "[4k]",
            "(hd)",
            "(live)",
            "[live]",
        ] {
            if lower.ends_with(suffix) {
                s.truncate(s.len() - suffix.len());
                s = s.trim().to_string();
            }
        }
        if s.len() == trimmed_len {
            break;
        }
    }
    s
}

/// Parses an ICY radio title or YouTube title into an identifiable song if valid.
pub fn parse_identifiable_track(raw: &str, station_name: Option<&str>) -> Option<ParsedTrack> {
    let collapsed: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = strip_trailing_noise_tags(&collapsed);
    if cleaned.len() < 3 {
        return None;
    }
    let lower = cleaned.to_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.ends_with(".mp3")
        || lower.ends_with(".aac")
        || lower.ends_with(".ogg")
        || lower.ends_with(".m3u8")
        || lower.ends_with(".pls")
    {
        return None;
    }
    if let Some(st) = station_name {
        let st_lower = st.trim().to_lowercase();
        if !st_lower.is_empty() && lower == st_lower {
            return None;
        }
    }
    if NOISE_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }

    for sep in [" — ", " – ", " - "] {
        if let Some((a, t)) = cleaned.split_once(sep) {
            let artist = a.trim();
            let title = t.trim();
            if !artist.is_empty() && !title.is_empty() {
                let display = format!("{artist} - {title}");
                return Some(ParsedTrack {
                    artist: Some(artist.to_string()),
                    title: title.to_string(),
                    display,
                });
            }
        }
    }

    Some(ParsedTrack {
        artist: None,
        title: cleaned.clone(),
        display: cleaned,
    })
}

pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let safe = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        };
        out.push(safe);
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(['.', ' ', '-']);
    if trimmed.is_empty() {
        "track".to_string()
    } else {
        trimmed.chars().take(160).collect()
    }
}

fn tokenize(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_string())
        .collect()
}

/// Scores YouTube search candidates to pick clean official audio matching a radio song.
pub fn pick_best_youtube_audio_match(
    candidates: &[YoutubeItem],
    artist: Option<&str>,
    title: &str,
) -> Option<YoutubeItem> {
    let mut target = String::new();
    if let Some(a) = artist {
        target.push_str(a);
        target.push(' ');
    }
    target.push_str(title);

    let want_tokens = tokenize(&target);
    let artist_tokens = artist.map(tokenize).unwrap_or_default();
    let title_tokens = tokenize(title);
    let want_lower = target.to_lowercase();

    let mut best: Option<(i32, &YoutubeItem)> = None;

    for item in candidates {
        if item.is_live {
            continue;
        }
        if let Some(dur) = item.duration_secs
            && !(45.0..=900.0).contains(&dur)
        {
            continue;
        }

        let cand_lower = item.title.to_lowercase();
        let chan_lower = item.channel.to_lowercase();
        let cand_tokens = tokenize(&format!("{} {}", item.title, item.channel));

        let mut score: i32 = 0;
        for tok in &title_tokens {
            if cand_tokens.contains(tok) {
                score += 18;
            } else {
                score -= 10;
            }
        }
        for tok in &artist_tokens {
            if cand_tokens.contains(tok) {
                score += 16;
            }
        }
        if want_tokens
            .iter()
            .filter(|t| cand_tokens.contains(*t))
            .count()
            == want_tokens.len()
            && !want_tokens.is_empty()
        {
            score += 20;
        }

        if chan_lower.ends_with(" - topic") {
            score += 35;
        }
        if cand_lower.contains("official audio") || cand_lower.contains("audio") {
            score += 14;
        }

        for bad in [
            "cover",
            "nightcore",
            "slowed",
            "reverb",
            "1 hour",
            "10 hours",
            "reaction",
            "karaoke",
            "live",
            "remix",
            "instrumental",
            "sped up",
            "8d audio",
        ] {
            if cand_lower.contains(bad) && !want_lower.contains(bad) {
                score -= 35;
            }
        }

        match best {
            Some((best_score, _)) if score <= best_score => {}
            _ => best = Some((score, item)),
        }
    }

    best.map(|(_, item)| item.clone())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct YtInfoMetadata {
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub artists: Option<Vec<String>>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub release_year: Option<u16>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub upload_date: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub webpage_url: Option<String>,
}

fn clean_channel_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(prefix) = trimmed.strip_suffix(" - Topic") {
        return prefix.trim().to_string();
    }
    if let Some(prefix) = trimmed.strip_suffix(" - topic") {
        return prefix.trim().to_string();
    }
    if trimmed.len() > 4 && trimmed.ends_with("VEVO") {
        return trimmed[..trimmed.len() - 4].trim().to_string();
    }
    trimmed.to_string()
}

fn parse_yyyymmdd(raw: &str) -> Option<Timestamp> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 8 {
        let year = digits[0..4].parse::<u16>().ok()?;
        let month = digits[4..6].parse::<u8>().ok().filter(|m| (1..=12).contains(m));
        let day = digits[6..8].parse::<u8>().ok().filter(|d| (1..=31).contains(d));
        return Some(Timestamp {
            year,
            month,
            day,
            ..Default::default()
        });
    }
    if digits.len() >= 4 {
        let year = digits[0..4].parse::<u16>().ok()?;
        return Some(Timestamp {
            year,
            ..Default::default()
        });
    }
    None
}

pub fn parse_yt_timestamp(
    release_date: Option<&str>,
    release_year: Option<u16>,
    upload_date: Option<&str>,
) -> Option<Timestamp> {
    if let Some(rd) = release_date
        && let Some(ts) = parse_yyyymmdd(rd)
    {
        return Some(ts);
    }
    if let Some(yr) = release_year.filter(|&y| y > 0) {
        return Some(Timestamp {
            year: yr,
            ..Default::default()
        });
    }
    upload_date.and_then(parse_yyyymmdd)
}

/// Crops a video/track thumbnail into a square JPEG cover (up to 800x800).
pub fn crop_thumbnail_to_square_jpg(src: &Path, dst: &Path) -> Result<()> {
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(src)
        .args([
            "-vf",
            "crop='min(iw,ih)':'min(iw,ih)',scale='trunc(min(800,iw)/2)*2':'trunc(min(800,iw)/2)*2'",
            "-q:v",
            "2",
        ])
        .arg(dst)
        .status()
        .context("Failed to execute ffmpeg for thumbnail cropping")?;
    anyhow::ensure!(status.success(), "ffmpeg thumbnail conversion failed");
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct TrackTagSpec {
    pub title: String,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub date: Option<Timestamp>,
    pub genre: Option<String>,
    pub publisher: Option<String>,
    pub source_url: Option<String>,
    pub radio_station: Option<String>,
    pub comment: String,
    pub lyrics: Option<String>,
}

/// Writes structured tags and front cover JPEG onto any `lofty`-supported audio file.
pub fn write_tags_and_cover(
    audio_path: &Path,
    spec: &TrackTagSpec,
    cover_jpg: Option<&[u8]>,
) -> Result<()> {
    let mut tagged_file = Probe::open(audio_path)
        .with_context(|| format!("Failed to open {}", audio_path.display()))?
        .guess_file_type()?
        .read()?;

    let tag_type = tagged_file.primary_tag_type();
    if tagged_file.primary_tag_mut().is_none() {
        tagged_file.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged_file
        .primary_tag_mut()
        .context("No writable metadata tag available")?;

    tag.set_title(spec.title.clone());
    if let Some(ref a) = spec.artist {
        tag.set_artist(a.clone());
    }
    if let Some(ref aa) = spec.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, aa.clone());
    }
    if let Some(ref alb) = spec.album {
        tag.set_album(alb.clone());
    }
    if let Some(trk) = spec.track_number {
        tag.set_track(trk);
    }
    if let Some(tot) = spec.track_total {
        tag.set_track_total(tot);
    }
    if let Some(ts) = spec.date {
        tag.set_date(ts);
    }
    if let Some(ref g) = spec.genre {
        tag.set_genre(g.clone());
    }
    if let Some(ref pub_name) = spec.publisher {
        tag.insert_text(ItemKey::Publisher, pub_name.clone());
    }
    if let Some(ref url) = spec.source_url
        && !url.is_empty()
    {
        tag.insert_text(ItemKey::Description, url.clone());
        tag.insert_text(ItemKey::AudioSourceUrl, url.clone());
    }
    if let Some(ref st) = spec.radio_station {
        tag.insert_text(ItemKey::InternetRadioStationName, st.clone());
    }
    if !spec.comment.is_empty() {
        tag.set_comment(spec.comment.clone());
    }
    if let Some(ref lyr) = spec.lyrics
        && !lyr.trim().is_empty()
    {
        crate::lyrics::set_lyrics_on_tag(tag, lyr);
    }

    if let Some(jpg) = cover_jpg.filter(|b| !b.is_empty()) {
        tag.remove_picture_type(PictureType::CoverFront);
        let pic = Picture::unchecked(jpg.to_vec())
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Jpeg)
            .description("Cover")
            .build();
        tag.push_picture(pic);
    }

    tag.save_to_path(audio_path, WriteOptions::default())
        .with_context(|| format!("Failed to save metadata to {}", audio_path.display()))?;
    Ok(())
}

/// Retries `extract_ytdlp_audio` once after a short backoff and removes partial leftovers on failure.
pub fn extract_ytdlp_audio_with_retry(
    url: &str,
    dest_dir: &Path,
    clean_name: &str,
    format: SaveAudioFormat,
    write_sidecars: bool,
) -> Result<PathBuf> {
    match extract_ytdlp_audio(url, dest_dir, clean_name, format, write_sidecars) {
        Ok(p) => Ok(p),
        Err(_first_err) => {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let res = extract_ytdlp_audio(url, dest_dir, clean_name, format, write_sidecars);
            if res.is_err() {
                if let Ok(entries) = fs::read_dir(dest_dir) {
                    for e in entries.flatten() {
                        let p = e.path();
                        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                        if fname.starts_with(clean_name) && fname.ends_with(".part") {
                            let _ = fs::remove_file(&p);
                        }
                    }
                }
            }
            res
        }
    }
}

/// Runs `yt-dlp` to extract clean audio in `format` into `dest_dir/<clean_name>.<ext>`.
pub fn extract_ytdlp_audio(
    url: &str,
    dest_dir: &Path,
    clean_name: &str,
    format: SaveAudioFormat,
    write_sidecars: bool,
) -> Result<PathBuf> {
    let final_path = dest_dir.join(format!("{clean_name}.{}", format.extension()));
    let template = format!("{}.%(ext)s", dest_dir.join(clean_name).to_string_lossy());

    let mut cmd = Command::new("yt-dlp");
    cmd.args(["--no-playlist", "--no-warnings"]);
    if format == SaveAudioFormat::Opus {
        cmd.args(["-f", "bestaudio[acodec^=opus]/bestaudio/best"]);
    }
    cmd.args([
        "-x",
        "--audio-format",
        format.ytdlp_format(),
        "--audio-quality",
        "0",
        "--embed-metadata",
        "--no-embed-chapters",
        "--no-embed-info-json",
    ]);
    if write_sidecars {
        cmd.args(["--write-thumbnail", "--write-info-json"]);
    }
    cmd.args(["-o", &template, url]);

    let out = cmd.output().context("Failed to launch yt-dlp")?;
    if !out.status.success() {
        let err_txt = String::from_utf8_lossy(&out.stderr);
        let msg = err_txt.lines().last().unwrap_or("yt-dlp download failed");
        anyhow::bail!("{}", msg.trim());
    }

    if final_path.exists() {
        return Ok(final_path);
    }
    let ext_want = format.extension();
    let found = fs::read_dir(dest_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem == clean_name)
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext_want))
        });
    Ok(found.unwrap_or(final_path))
}

/// Embeds complete artist/album/date/genre/source metadata and front cover art using `lofty`.
pub fn apply_rich_metadata_and_cover(
    audio_path: &Path,
    info: Option<&YtInfoMetadata>,
    cover_jpg: Option<&[u8]>,
    parsed: &ParsedTrack,
    radio_station: Option<&str>,
    fallback_url: &str,
) -> Result<()> {
    let non_empty = |opt: Option<&String>| -> Option<String> {
        opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };

    let info_track = info.and_then(|i| non_empty(i.track.as_ref()));
    let info_title = info.and_then(|i| non_empty(i.title.as_ref()));
    let title = info_track
        .or_else(|| {
            let t = parsed.title.trim();
            (!t.is_empty()).then(|| t.to_string())
        })
        .or(info_title)
        .unwrap_or_else(|| parsed.display.clone());

    let info_artists_joined = info.and_then(|i| {
        i.artists.as_ref().and_then(|list| {
            let clean: Vec<&str> = list.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            (!clean.is_empty()).then(|| clean.join(", "))
        })
    });
    let channel_fallback = info
        .and_then(|i| non_empty(i.channel.as_ref()).or_else(|| non_empty(i.uploader.as_ref())))
        .map(|c| clean_channel_name(&c))
        .filter(|c| !c.is_empty());

    let artist = info
        .and_then(|i| non_empty(i.artist.as_ref()))
        .or(info_artists_joined)
        .or_else(|| info.and_then(|i| non_empty(i.creator.as_ref())))
        .or_else(|| parsed.artist.as_deref().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()))
        .or_else(|| channel_fallback.clone());

    let station_clean = radio_station.map(str::trim).filter(|s| !s.is_empty());
    let title_key = normalize_match_key(&title);
    let album = info
        .and_then(|i| non_empty(i.album.as_ref()))
        .filter(|alb| {
            let ak = normalize_match_key(alb);
            !ak.is_empty()
                && ak != title_key
                && !station_clean.is_some_and(|st| st.eq_ignore_ascii_case(alb))
        });

    let genre = info
        .and_then(|i| non_empty(i.genre.as_ref()))
        .or_else(|| {
            info.and_then(|i| {
                i.categories
                    .as_ref()
                    .and_then(|cats| cats.first())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
        });

    let date = info.and_then(|i| {
        parse_yt_timestamp(
            i.release_date.as_deref(),
            i.release_year,
            i.upload_date.as_deref(),
        )
    });

    let source_url = info
        .and_then(|i| non_empty(i.webpage_url.as_ref()))
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| fallback_url.to_string());

    let comment = match station_clean {
        Some(st) => format!("Saved via Vibes · Station: {st}"),
        None => "Saved via Vibes".to_string(),
    };

    let lyrics = artist.as_ref().and_then(|art| {
        let query = crate::lyrics::LyricsQuery {
            artist: art.clone(),
            title: title.clone(),
            album: album.clone(),
            duration_secs: info.and_then(|i| i.duration),
        };
        let agent = crate::lyrics::new_http_agent();
        let res = crate::lyrics::fetch_lrclib_lyrics(&agent, &query);
        crate::lyrics::lrclib_text_to_embed(&res).map(str::to_string)
    });

    let spec = TrackTagSpec {
        title,
        album_artist: artist.clone(),
        artist,
        album,
        track_number: None,
        track_total: None,
        date,
        genre,
        publisher: channel_fallback,
        source_url: (!source_url.is_empty()).then_some(source_url),
        radio_station: station_clean.map(|s| s.to_string()),
        comment,
        lyrics,
    };

    write_tags_and_cover(audio_path, &spec, cover_jpg)
}

fn find_and_clean_sidecars(
    dest_dir: &Path,
    clean_name: &str,
) -> (Option<YtInfoMetadata>, Option<Vec<u8>>) {
    let info_path = dest_dir.join(format!("{clean_name}.info.json"));
    let info = fs::read_to_string(&info_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<YtInfoMetadata>(&raw).ok());
    let _ = fs::remove_file(&info_path);

    let mut raw_thumb_path = None;
    for ext in ["webp", "jpg", "jpeg", "png"] {
        let candidate = dest_dir.join(format!("{clean_name}.{ext}"));
        if candidate.exists() {
            raw_thumb_path = Some(candidate);
            break;
        }
    }

    let cover_jpg = if let Some(ref thumb) = raw_thumb_path {
        let cropped_path = dest_dir.join(format!(".{clean_name}.cover.jpg"));
        let res = if crop_thumbnail_to_square_jpg(thumb, &cropped_path).is_ok() {
            fs::read(&cropped_path).ok()
        } else {
            None
        };
        let _ = fs::remove_file(&cropped_path);
        res
    } else {
        None
    };

    for ext in ["webp", "jpg", "jpeg", "png"] {
        let _ = fs::remove_file(dest_dir.join(format!("{clean_name}.{ext}")));
    }

    (info, cover_jpg)
}

fn move_audio_file(src: &Path, dst: &Path) -> Result<()> {
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

fn find_matching_album_dir(artist_dir: &Path, album: &str) -> PathBuf {
    let clean_alb = sanitize_filename(album);
    let exact = artist_dir.join(&clean_alb);
    if exact.is_dir() {
        return exact;
    }
    let want = normalize_match_key(album);
    if !want.is_empty()
        && let Ok(entries) = fs::read_dir(artist_dir)
    {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir()
                && let Some(name) = p.file_name().and_then(|n| n.to_str())
                && !name.eq_ignore_ascii_case("singles")
                && normalize_match_key(name) == want
            {
                return p;
            }
        }
    }
    exact
}

pub fn resolve_song_destination_from_meta(
    albums_dir: &Path,
    artist: &str,
    album: Option<&str>,
    title: &str,
    radio_station: Option<&str>,
    ext: &str,
) -> (PathBuf, PathBuf) {
    let artist_dir = find_canonical_artist_dir(albums_dir, artist);
    let clean_title = sanitize_filename(title);
    let title_key = normalize_match_key(title);
    let real_album = album
        .map(str::trim)
        .filter(|alb| !alb.is_empty())
        .filter(|alb| {
            let ak = normalize_match_key(alb);
            !ak.is_empty()
                && ak != title_key
                && !alb.eq_ignore_ascii_case("singles")
                && !alb.to_lowercase().contains("radio")
                && !alb.to_lowercase().contains(".fm")
                && !radio_station.is_some_and(|st| st.eq_ignore_ascii_case(alb))
        });

    let dest_dir = match real_album {
        Some(alb) => find_matching_album_dir(&artist_dir, alb),
        None => artist_dir.join("Singles"),
    };
    let final_path = dest_dir.join(format!("{clean_title}.{ext}"));
    (dest_dir, final_path)
}

pub fn run_save_song(
    albums_dir: &Path,
    format: SaveAudioFormat,
    parsed: &ParsedTrack,
    source_station: &str,
    direct_url: Option<&str>,
    mut on_progress: impl FnMut(DownloadStatus),
) -> DownloadStatus {
    // Fast duplicate check if we already know artist & title
    if let Some(ref known_artist) = parsed.artist {
        let artist_dir = find_canonical_artist_dir(albums_dir, known_artist);
        if let Some(existing) = artist_has_track(&artist_dir, &parsed.title) {
            let status = DownloadStatus::AlreadyExists(existing);
            on_progress(status.clone());
            return status;
        }
    }

    let is_from_radio = direct_url.is_none();
    let chosen_url = if let Some(u) = direct_url {
        u.to_string()
    } else {
        on_progress(DownloadStatus::Searching);
        let search_arg = format!("ytsearch6:{}", parsed.display);
        let search_out = Command::new("yt-dlp")
            .args([
                "--flat-playlist",
                "--dump-json",
                "--no-warnings",
                "--ignore-errors",
                &search_arg,
            ])
            .output();

        let output = match search_out {
            Ok(o) => o,
            Err(err) => {
                let status = DownloadStatus::Failed(format!("yt-dlp search failed: {err}"));
                on_progress(status.clone());
                return status;
            }
        };

        let candidates = parse_ytdlp_ndjson(&String::from_utf8_lossy(&output.stdout));
        let Some(picked) = pick_best_youtube_audio_match(
            &candidates,
            parsed.artist.as_deref(),
            &parsed.title,
        )
        .or_else(|| candidates.into_iter().find(|c| !c.is_live)) else {
            let status = DownloadStatus::Failed(format!(
                "No downloadable YouTube match found for '{}'",
                parsed.display
            ));
            on_progress(status.clone());
            return status;
        };
        picked.url
    };

    on_progress(DownloadStatus::Downloading(parsed.display.clone()));
    let staging_root = albums_dir.join(".vibes-staging");
    let unique = format!(
        "song-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let stage_dir = staging_root.join(&unique);
    if let Err(err) = fs::create_dir_all(&stage_dir) {
        let status = DownloadStatus::Failed(format!("Cannot create staging dir: {err}"));
        on_progress(status.clone());
        return status;
    }

    let stage_name = sanitize_filename(&parsed.display);
    let status = match extract_ytdlp_audio_with_retry(&chosen_url, &stage_dir, &stage_name, format, true) {
        Ok(staged_audio) => {
            let (info, cover_jpg) = find_and_clean_sidecars(&stage_dir, &stage_name);
            let radio_name = is_from_radio.then_some(source_station);

            let resolved_title = info
                .as_ref()
                .and_then(|i| i.track.as_deref().map(str::trim).filter(|s| !s.is_empty()))
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| parsed.title.clone());

            let resolved_artist = info
                .as_ref()
                .and_then(|i| {
                    i.artist
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned)
                        .or_else(|| {
                            i.artists.as_ref().and_then(|list| {
                                let clean: Vec<&str> =
                                    list.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                                (!clean.is_empty()).then(|| clean.join(", "))
                            })
                        })
                        .or_else(|| {
                            i.creator
                                .as_deref()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(ToOwned::to_owned)
                        })
                })
                .or_else(|| {
                    parsed
                        .artist
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned)
                })
                .or_else(|| {
                    info.as_ref()
                        .and_then(|i| i.channel.as_deref().or(i.uploader.as_deref()))
                        .map(clean_channel_name)
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| "Unknown Artist".to_string());

            let artist_dir = find_canonical_artist_dir(albums_dir, &resolved_artist);
            if let Some(existing) = artist_has_track(&artist_dir, &resolved_title) {
                let _ = fs::remove_dir_all(&stage_dir);
                let st = DownloadStatus::AlreadyExists(existing);
                on_progress(st.clone());
                return st;
            }

            let ext = staged_audio
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_else(|| format.extension());
            let (dest_dir, final_path) = resolve_song_destination_from_meta(
                albums_dir,
                &resolved_artist,
                info.as_ref().and_then(|i| i.album.as_deref()),
                &resolved_title,
                radio_name,
                ext,
            );

            if final_path.exists() {
                let _ = fs::remove_dir_all(&stage_dir);
                let st = DownloadStatus::AlreadyExists(final_path);
                on_progress(st.clone());
                return st;
            }

            // If saving into Singles/, make sure album tag reads "Singles" instead of blank
            let mut info_for_tags = info.unwrap_or_default();
            let in_singles = dest_dir
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("singles"));
            if in_singles {
                info_for_tags.album = Some("Singles".to_string());
            }

            if let Err(err) = apply_rich_metadata_and_cover(
                &staged_audio,
                Some(&info_for_tags),
                cover_jpg.as_deref(),
                &ParsedTrack {
                    artist: Some(resolved_artist),
                    title: resolved_title,
                    display: parsed.display.clone(),
                },
                radio_name,
                &chosen_url,
            ) {
                eprintln!("Vibes metadata warning for {}: {err}", staged_audio.display());
            }

            match move_audio_file(&staged_audio, &final_path) {
                Ok(()) => DownloadStatus::Saved(final_path),
                Err(err) => DownloadStatus::Failed(format!("Failed moving saved song: {err}")),
            }
        }
        Err(err) => DownloadStatus::Failed(format!("{err}")),
    };
    let _ = fs::remove_dir_all(&stage_dir);
    on_progress(status.clone());
    status
}

pub fn run_retry_failed_item(
    item: &FailedDownloadItem,
    albums_dir: &Path,
    format: SaveAudioFormat,
) -> Result<PathBuf> {
    let url = item
        .url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .with_context(|| format!("No URL recorded for failed item '{}'", item.title))?;

    let is_singles = item
        .album
        .as_deref()
        .is_none_or(|a| a.eq_ignore_ascii_case("singles"));
    if item.track_number.is_some() && !is_singles {
        fs::create_dir_all(&item.dest_dir)?;
        let num = item.track_number.unwrap_or(1);
        let clean_name = format!("{num:02} - {}", sanitize_filename(&item.title));
        let path = extract_ytdlp_audio_with_retry(url, &item.dest_dir, &clean_name, format, true)?;
        let (info, cover_jpg) = find_and_clean_sidecars(&item.dest_dir, &clean_name);
        let cover_fallback = cover_jpg.or_else(|| fs::read(item.dest_dir.join("cover.jpg")).ok());
        let parsed = ParsedTrack {
            artist: Some(item.artist.clone()),
            title: item.title.clone(),
            display: format!("{} - {}", item.artist, item.title),
        };
        let mut meta = info.unwrap_or_default();
        if meta.album.is_none() {
            meta.album = item.album.clone();
        }
        let _ = apply_rich_metadata_and_cover(
            &path,
            Some(&meta),
            cover_fallback.as_deref(),
            &parsed,
            None,
            url,
        );
        Ok(path)
    } else {
        let parsed = ParsedTrack {
            artist: (!item.artist.trim().is_empty()).then(|| item.artist.clone()),
            title: item.title.clone(),
            display: format!("{} - {}", item.artist, item.title),
        };
        match run_save_song(albums_dir, format, &parsed, "", Some(url), |_| {}) {
            DownloadStatus::Saved(p) | DownloadStatus::AlreadyExists(p) => Ok(p),
            DownloadStatus::Failed(e) => anyhow::bail!("{e}"),
            _ => anyhow::bail!("Retry did not finish"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identifiable_tracks_and_filters_station_junk() {
        assert_eq!(parse_identifiable_track("https://radio.example", None), None);
        assert_eq!(
            parse_identifiable_track("Nightride FM", Some("Nightride FM")),
            None
        );
        assert_eq!(parse_identifiable_track("Commercial Break", None), None);

        let parsed = parse_identifiable_track(
            "Gorillaz - On Melancholy Hill (Official Video)",
            Some("Radio"),
        )
        .expect("should parse track");
        assert_eq!(parsed.artist.as_deref(), Some("Gorillaz"));
        assert_eq!(parsed.title, "On Melancholy Hill");
        assert_eq!(parsed.display, "Gorillaz - On Melancholy Hill");
    }

    #[test]
    fn sanitizes_filenames_safely() {
        assert_eq!(
            sanitize_filename("Artist / Name: Track? *1*"),
            "Artist - Name- Track- -1"
        );
        assert_eq!(sanitize_filename("..."), "track");
    }

    #[test]
    fn picks_clean_topic_or_official_audio_over_live_or_covers() {
        let candidates = vec![
            YoutubeItem {
                id: "1".into(),
                title: "Gorillaz - Feel Good Inc (Acoustic Cover)".into(),
                channel: "CoverGuy".into(),
                duration_secs: Some(210.0),
                url: "https://youtu.be/1".into(),
                is_live: false,
                view_count: None,
            },
            YoutubeItem {
                id: "2".into(),
                title: "Feel Good Inc.".into(),
                channel: "Gorillaz - Topic".into(),
                duration_secs: Some(222.0),
                url: "https://youtu.be/2".into(),
                is_live: false,
                view_count: None,
            },
            YoutubeItem {
                id: "3".into(),
                title: "Gorillaz - Feel Good Inc Live Stream".into(),
                channel: "Radio".into(),
                duration_secs: None,
                url: "https://youtu.be/3".into(),
                is_live: true,
                view_count: None,
            },
        ];
        let picked = pick_best_youtube_audio_match(&candidates, Some("Gorillaz"), "Feel Good Inc")
            .expect("should pick topic audio");
        assert_eq!(picked.id, "2");
    }

    #[test]
    fn route_single_saves_puts_singles_in_artist_singles_and_ignores_station_album() {
        let tmp = tempfile::tempdir().unwrap();
        let korn_dir = tmp.path().join("Korn");
        let existing_alb = korn_dir.join("Follow The Leader");
        fs::create_dir_all(&existing_alb).unwrap();

        let (dir_alb, path_alb) = resolve_song_destination_from_meta(
            tmp.path(),
            "KoRn",
            Some("Follow The Leader"),
            "Got the Life",
            Some("Nuclear Fallout Radio"),
            "opus",
        );
        assert_eq!(dir_alb, existing_alb);
        assert_eq!(path_alb, existing_alb.join("Got the Life.opus"));

        let (dir_radio, path_radio) = resolve_song_destination_from_meta(
            tmp.path(),
            "Jinjer",
            Some("Nuclear Fallout Radio"),
            "Someone's Daughter",
            Some("Nuclear Fallout Radio"),
            "opus",
        );
        assert_eq!(dir_radio, tmp.path().join("Jinjer").join("Singles"));
        assert_eq!(
            path_radio,
            tmp.path().join("Jinjer").join("Singles").join("Someone's Daughter.opus")
        );
    }

    #[test]
    fn embeds_cover_art_and_full_metadata_into_real_opus_file() {
        let tmp = tempfile::tempdir().unwrap();
        let opus_path = tmp.path().join("track.opus");
        let thumb_src = tmp.path().join("thumb.jpg");
        let thumb_sq = tmp.path().join("cover.jpg");

        // Synthesize a 0.2s silent Opus file and a 160x90 test JPG via ffmpeg
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
                "color=c=0x8ba4b0:s=160x90",
                "-frames:v",
                "1",
            ])
            .arg(&thumb_src)
            .status()
            .unwrap();
        assert!(st_img.success());

        crop_thumbnail_to_square_jpg(&thumb_src, &thumb_sq).unwrap();
        let cover_bytes = fs::read(&thumb_sq).unwrap();

        let parsed = ParsedTrack {
            artist: Some("Carpenter Brut".into()),
            title: "Turbo Killer".into(),
            display: "Carpenter Brut - Turbo Killer".into(),
        };
        let info = YtInfoMetadata {
            track: Some("Turbo Killer".into()),
            title: Some("Carpenter Brut - Turbo Killer".into()),
            artist: Some("Carpenter Brut".into()),
            album: Some("Trilogy".into()),
            channel: Some("Carpenter Brut - Topic".into()),
            release_date: Some("20160115".into()),
            genre: Some("Synthwave".into()),
            webpage_url: Some("https://www.youtube.com/watch?v=example".into()),
            ..Default::default()
        };

        apply_rich_metadata_and_cover(
            &opus_path,
            Some(&info),
            Some(&cover_bytes),
            &parsed,
            Some("Nightride FM"),
            "https://www.youtube.com/watch?v=example",
        )
        .unwrap();

        let probed = Probe::open(&opus_path).unwrap().read().unwrap();
        let tag = probed.primary_tag().expect("opus tag must exist");
        assert_eq!(tag.title().as_deref(), Some("Turbo Killer"));
        assert_eq!(tag.artist().as_deref(), Some("Carpenter Brut"));
        assert_eq!(tag.album().as_deref(), Some("Trilogy"));
        assert_eq!(tag.genre().as_deref(), Some("Synthwave"));
        assert_eq!(tag.get_string(ItemKey::AlbumArtist), Some("Carpenter Brut"));
        assert_eq!(tag.get_string(ItemKey::Publisher), Some("Carpenter Brut"));
        assert_eq!(
            tag.get_string(ItemKey::Description),
            Some("https://www.youtube.com/watch?v=example")
        );
        assert_eq!(
            tag.comment().as_deref(),
            Some("Saved via Vibes · Station: Nightride FM")
        );
        let ts = tag.date().expect("release date timestamp must be saved");
        assert_eq!(ts.year, 2016);
        assert_eq!(ts.month, Some(1));
        assert_eq!(ts.day, Some(15));
        assert!(
            tag.pictures()
                .iter()
                .any(|p| p.pic_type() == PictureType::CoverFront
                    && p.mime_type() == Some(&MimeType::Jpeg)
                    && !p.data().is_empty())
        );

        let lrc = "[00:01.00] Line one\n[00:02.00] Line two";
        let lyric_spec = TrackTagSpec {
            title: "Turbo Killer".into(),
            artist: Some("Carpenter Brut".into()),
            lyrics: Some(lrc.to_string()),
            ..Default::default()
        };
        write_tags_and_cover(&opus_path, &lyric_spec, Some(&cover_bytes)).unwrap();
        assert_eq!(crate::lyrics::read_embedded_lyrics(&opus_path).as_deref(), Some(lrc));
    }
}
