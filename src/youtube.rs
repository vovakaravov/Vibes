use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct YoutubeItem {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration_secs: Option<f64>,
    pub url: String,
    pub is_live: bool,
    pub view_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YoutubeFilter {
    #[default]
    All,
    Music,
    Podcasts,
    Live,
}

impl YoutubeFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Music, Self::Podcasts, Self::Live];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Music => "Music",
            Self::Podcasts => "Podcasts (20m+)",
            Self::Live => "Live",
        }
    }

    pub fn matches(self, item: &YoutubeItem) -> bool {
        match self {
            Self::All => true,
            Self::Music => {
                !item.is_live
                    && item
                        .duration_secs
                        .is_none_or(|d| d >= 30.0 && d <= 900.0)
            }
            Self::Podcasts => !item.is_live && item.duration_secs.is_some_and(|d| d >= 1200.0),
            Self::Live => item.is_live,
        }
    }
}

pub fn is_youtube_url(input: &str) -> bool {
    let lower = input.trim().to_lowercase();
    lower.starts_with("https://youtube.com/")
        || lower.starts_with("https://www.youtube.com/")
        || lower.starts_with("https://music.youtube.com/")
        || lower.starts_with("https://youtu.be/")
        || lower.starts_with("https://m.youtube.com/")
        || lower.starts_with("http://youtube.com/")
        || lower.starts_with("http://www.youtube.com/")
}

pub fn format_duration(secs: f64) -> String {
    let tot = secs.max(0.0).round() as u64;
    let h = tot / 3600;
    let m = (tot % 3600) / 60;
    let s = tot % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[derive(Deserialize)]
struct RawYtdlpEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    webpage_url: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    is_live: Option<bool>,
    #[serde(default)]
    live_status: Option<String>,
    #[serde(default)]
    view_count: Option<u64>,
}

pub fn parse_ytdlp_ndjson(ndjson: &str) -> Vec<YoutubeItem> {
    let mut out = Vec::new();
    for line in ndjson.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<RawYtdlpEntry>(trimmed) else {
            continue;
        };
        let Some(title) = raw.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) else {
            continue;
        };
        let id = raw.id.unwrap_or_default();
        let url = raw
            .webpage_url
            .filter(|u| u.starts_with("http"))
            .or_else(|| {
                raw.url.map(|u| {
                    if u.starts_with("http") {
                        u
                    } else {
                        format!("https://www.youtube.com/watch?v={u}")
                    }
                })
            })
            .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));

        let is_live = raw.is_live.unwrap_or(false)
            || raw.live_status.as_deref() == Some("is_live");
        let channel = raw
            .channel
            .or(raw.uploader)
            .unwrap_or_else(|| "YouTube".to_string());

        out.push(YoutubeItem {
            id,
            title,
            channel,
            duration_secs: raw.duration,
            url,
            is_live,
            view_count: raw.view_count,
        });
    }
    out
}

pub fn search_youtube(query: &str, limit: usize) -> Result<Vec<YoutubeItem>> {
    let trimmed = query.trim();
    anyhow::ensure!(!trimmed.is_empty(), "Empty search query");

    let target = if is_youtube_url(trimmed) {
        trimmed.to_string()
    } else {
        format!("ytsearch{}:{trimmed}", limit.clamp(1, 30))
    };

    let output = Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--dump-json",
            "--no-warnings",
            "--ignore-errors",
            &target,
        ])
        .output()
        .context("Failed to execute `yt-dlp`")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let items = parse_ytdlp_ndjson(&stdout);
    if items.is_empty() && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        anyhow::bail!(
            "{}",
            if msg.is_empty() {
                "yt-dlp returned no results"
            } else {
                msg
            }
        );
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_youtube_urls_and_formats_durations() {
        assert!(is_youtube_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://music.youtube.com/watch?v=abc"));
        assert!(!is_youtube_url("synthwave mix"));
        assert_eq!(format_duration(75.0), "01:15");
        assert_eq!(format_duration(3725.0), "1:02:05");
    }

    #[test]
    fn parses_ytdlp_ndjson_and_applies_filters() {
        let ndjson = r#"{"id":"abc123","title":"Daft Punk - Voyager","channel":"Daft Punk","duration":227.0,"url":"abc123","live_status":"not_live","view_count":500000}
{"id":"pod999","title":"Linux Kernel Deep Dive","channel":"TechPod","duration":3600.0,"webpage_url":"https://www.youtube.com/watch?v=pod999","is_live":false}
{"id":"live1","title":"Lofi Hip Hop Radio","uploader":"Lofi Girl","is_live":true}"#;

        let items = parse_ytdlp_ndjson(ndjson);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].url, "https://www.youtube.com/watch?v=abc123");
        assert!(YoutubeFilter::Music.matches(&items[0]));
        assert!(!YoutubeFilter::Podcasts.matches(&items[0]));
        assert!(YoutubeFilter::Podcasts.matches(&items[1]));
        assert!(!YoutubeFilter::Music.matches(&items[1]));
        assert!(YoutubeFilter::Live.matches(&items[2]));
    }
}
