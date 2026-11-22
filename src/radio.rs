use anyhow::{Context, Result};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const MIRRORS: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://fi1.api.radio-browser.info",
    "https://at1.api.radio-browser.info",
];

pub const GENRE_CHIPS: &[&str] = &[
    "Top",
    "Lo-Fi",
    "Synthwave",
    "Electronic",
    "Jazz",
    "Ambient",
    "DnB",
    "Rock",
    "Classical",
    "Phonk",
    "Local",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RadioStation {
    #[serde(default)]
    pub stationuuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub url_resolved: String,
    #[serde(default)]
    pub favicon: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub countrycode: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub clickcount: i64,
    #[serde(default)]
    pub hls: u8,
}

impl RadioStation {
    pub fn stream_url(&self) -> &str {
        if !self.url_resolved.trim().is_empty() {
            self.url_resolved.trim()
        } else {
            self.url.trim()
        }
    }

    pub fn display_codec(&self) -> String {
        let codec = self.codec.trim().to_uppercase();
        match (codec.is_empty(), self.bitrate > 0) {
            (false, true) => format!("{codec} {}k", self.bitrate),
            (false, false) => codec,
            (true, true) => format!("{}k", self.bitrate),
            (true, false) => "STREAM".to_string(),
        }
    }

    pub fn custom(name: impl Into<String>, url: impl Into<String>) -> Self {
        let u = url.into().trim().to_string();
        let n = name.into().trim().to_string();
        let clean_name = if n.is_empty() { u.clone() } else { n };
        let uuid = format!("custom-{clean_name}-{u}");
        Self {
            stationuuid: uuid,
            name: clean_name,
            url: u.clone(),
            url_resolved: u,
            favicon: String::new(),
            tags: "custom".to_string(),
            country: String::new(),
            countrycode: String::new(),
            codec: "CUSTOM".to_string(),
            bitrate: 0,
            votes: 0,
            clickcount: 0,
            hls: 0,
        }
    }
}

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("vibes").join("stations_cache.json"))
}

pub fn load_cached_stations() -> Vec<RadioStation> {
    cache_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<Vec<RadioStation>>(&raw).ok())
        .unwrap_or_default()
}

pub fn save_cached_stations(stations: &[RadioStation]) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(stations) {
        let _ = fs::write(path, json);
    }
}

/// Filters out empty/duplicate streams and prefers high-signal entries.
pub fn filter_and_dedupe(stations: Vec<RadioStation>) -> Vec<RadioStation> {
    let mut seen_urls = HashSet::new();
    let mut out = Vec::with_capacity(stations.len());
    for mut s in stations {
        s.name = s.name.trim().to_string();
        let stream = s.stream_url().to_string();
        if s.name.is_empty()
            || stream.is_empty()
            || (!stream.starts_with("http://") && !stream.starts_with("https://"))
        {
            continue;
        }
        if seen_urls.insert(stream.to_lowercase()) {
            out.push(s);
        }
    }
    out
}

/// Interleaves local stations into global results every 4th slot without duplicates.
pub fn blend_local_stations(
    global: Vec<RadioStation>,
    local: Vec<RadioStation>,
    limit: usize,
) -> Vec<RadioStation> {
    if local.is_empty() {
        return global.into_iter().take(limit).collect();
    }
    let mut out = Vec::with_capacity(limit);
    let mut seen_ids = HashSet::new();
    let mut g_iter = global.into_iter();
    let mut l_iter = local.into_iter();
    let mut slot = 0usize;

    while out.len() < limit {
        let pick_local = slot % 4 == 3;
        let next = if pick_local {
            l_iter.next().or_else(|| g_iter.next())
        } else {
            g_iter.next().or_else(|| l_iter.next())
        };
        let Some(station) = next else {
            break;
        };
        let key = if station.stationuuid.is_empty() {
            station.stream_url().to_lowercase()
        } else {
            station.stationuuid.clone()
        };
        if seen_ids.insert(key) {
            out.push(station);
        }
        slot += 1;
    }
    out
}

#[derive(Clone)]
pub struct RadioBrowserClient {
    agent: ureq::Agent,
}

impl Default for RadioBrowserClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RadioBrowserClient {
    pub fn new() -> Self {
        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(7)))
            .build();
        Self {
            agent: cfg.into(),
        }
    }

    fn get_json(&self, path_and_query: &str) -> Result<String> {
        let mut last_err = None;
        for mirror in MIRRORS {
            let url = format!("{mirror}{path_and_query}");
            match self
                .agent
                .get(&url)
                .header("User-Agent", "Vibes/0.1")
                .call()
            {
                Ok(mut resp) => match resp.body_mut().read_to_string() {
                    Ok(body) => return Ok(body),
                    Err(e) => last_err = Some(anyhow::anyhow!("{mirror}: {e}")),
                },
                Err(e) => last_err = Some(anyhow::anyhow!("{mirror}: {e}")),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("All RadioBrowser mirrors failed")))
    }

    fn search_endpoint(
        &self,
        query: &str,
        tag: &str,
        country_code: &str,
        limit: usize,
    ) -> Result<Vec<RadioStation>> {
        let mut params = vec![
            "hidebroken=true".to_string(),
            "order=clickcount".to_string(),
            "reverse=true".to_string(),
            format!("limit={limit}"),
        ];
        if !query.trim().is_empty() {
            let enc = utf8_percent_encode(query.trim(), NON_ALPHANUMERIC);
            params.push(format!("name={enc}"));
        }
        if !tag.trim().is_empty() {
            let normalized = tag.trim().to_lowercase();
            let mapped = match normalized.as_str() {
                "dnb" => "drum and bass",
                "lo-fi" => "lofi",
                other => other,
            };
            let enc = utf8_percent_encode(mapped, NON_ALPHANUMERIC);
            params.push(format!("tag={enc}"));
        }
        if !country_code.trim().is_empty() {
            let cc = country_code.trim().to_uppercase();
            let enc = utf8_percent_encode(&cc, NON_ALPHANUMERIC);
            params.push(format!("countrycode={enc}"));
        }

        let path = format!("/json/stations/search?{}", params.join("&"));
        let body = self.get_json(&path)?;
        let parsed: Vec<RadioStation> =
            serde_json::from_str(&body).context("Invalid JSON from RadioBrowser")?;
        Ok(filter_and_dedupe(parsed))
    }

    pub fn fetch_stations(
        &self,
        query: &str,
        genre: &str,
        country_code: &str,
        local_blending: bool,
        limit: usize,
    ) -> Result<Vec<RadioStation>> {
        let cc = country_code.trim().to_uppercase();
        if genre.eq_ignore_ascii_case("Local") {
            let effective_cc = if cc.is_empty() { "US" } else { cc.as_str() };
            return self.search_endpoint(query, "", effective_cc, limit);
        }

        let tag = if genre.eq_ignore_ascii_case("Top") {
            ""
        } else {
            genre
        };
        let global = self.search_endpoint(query, tag, "", limit)?;
        if local_blending && !cc.is_empty() && query.trim().is_empty() {
            let local = self
                .search_endpoint("", tag, &cc, (limit / 3).max(8))
                .unwrap_or_default();
            return Ok(blend_local_stations(global, local, limit));
        }
        Ok(global.into_iter().take(limit).collect())
    }

    /// Picks a high-bitrate working station for "Spin the Dial".
    pub fn fetch_random_station(
        &self,
        genre: &str,
        country_code: &str,
        exclude_uuid: Option<&str>,
    ) -> Result<RadioStation> {
        let pool = self.fetch_stations("", genre, country_code, false, 60)?;
        let quality: Vec<RadioStation> = pool
            .into_iter()
            .filter(|s| exclude_uuid.is_none_or(|ex| s.stationuuid != ex))
            .collect();
        anyhow::ensure!(!quality.is_empty(), "No matching stations available");
        let high_bitrate: Vec<&RadioStation> =
            quality.iter().filter(|s| s.bitrate >= 96).collect();
        let chosen_slice = if !high_bitrate.is_empty() {
            high_bitrate
        } else {
            quality.iter().collect()
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        Ok(chosen_slice[nanos % chosen_slice.len()].clone())
    }

    pub fn register_click(&self, uuid: &str) {
        if uuid.trim().is_empty() || uuid.starts_with("custom-") {
            return;
        }
        let path = format!("/json/url/{}", uuid.trim());
        let _ = self.get_json(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, url: &str) -> RadioStation {
        RadioStation {
            stationuuid: id.to_string(),
            name: format!("Station {id}"),
            url: url.to_string(),
            url_resolved: url.to_string(),
            favicon: String::new(),
            tags: String::new(),
            country: String::new(),
            countrycode: String::new(),
            codec: "AAC".to_string(),
            bitrate: 128,
            votes: 10,
            clickcount: 100,
            hls: 0,
        }
    }

    #[test]
    fn filters_broken_schemes_and_duplicates() {
        let input = vec![
            mk("1", "https://stream.example/a"),
            mk("2", "ftp://bad.example/b"),
            mk("3", "https://stream.example/a"),
            RadioStation {
                name: "   ".to_string(),
                ..mk("4", "https://stream.example/c")
            },
        ];
        let cleaned = filter_and_dedupe(input);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].stationuuid, "1");
    }

    #[test]
    fn blends_local_stations_every_fourth_slot_without_dupes() {
        let global = vec![
            mk("g1", "https://a/1"),
            mk("g2", "https://a/2"),
            mk("g3", "https://a/3"),
            mk("g4", "https://a/4"),
            mk("g5", "https://a/5"),
        ];
        let local = vec![
            mk("l1", "https://l/1"),
            mk("g2", "https://a/2"), // duplicate should be skipped
            mk("l2", "https://l/2"),
        ];
        let blended = blend_local_stations(global, local, 6);
        let ids: Vec<&str> = blended.iter().map(|s| s.stationuuid.as_str()).collect();
        assert_eq!(ids, vec!["g1", "g2", "g3", "l1", "g4", "g5"]);
    }
}
