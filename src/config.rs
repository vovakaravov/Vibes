use crate::local_library::LocalTrack;
use crate::radio::RadioStation;
use crate::theme::ThemePreset;
use crate::youtube::YoutubeItem;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SaveFolderMode {
    #[default]
    Flat,
    ByArtist,
}

impl SaveFolderMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat (music_dir/Artist - Title.ext)",
            Self::ByArtist => "By Artist (music_dir/Artist/Artist - Title.ext)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SaveAudioFormat {
    #[default]
    Opus,
    M4a,
    Mp3,
    Ogg,
    Flac,
}

impl SaveAudioFormat {
    pub const ALL: [Self; 5] = [Self::Opus, Self::M4a, Self::Mp3, Self::Ogg, Self::Flac];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Ogg => "ogg",
            Self::Opus => "opus",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Flac => "flac",
        }
    }

    pub fn ytdlp_format(self) -> &'static str {
        match self {
            Self::Ogg => "vorbis",
            Self::Opus => "opus",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Flac => "flac",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ogg => "OGG Vorbis (.ogg)",
            Self::Opus => "Opus (.opus)",
            Self::Mp3 => "MP3 (.mp3)",
            Self::M4a => "M4A / AAC (.m4a)",
            Self::Flac => "FLAC (.flac)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AlbumFolderMode {
    #[default]
    ArtistAndAlbum,
    AlbumPrefix,
    ArtistDashAlbum,
}

impl AlbumFolderMode {
    pub const ALL: [Self; 3] = [Self::ArtistAndAlbum, Self::AlbumPrefix, Self::ArtistDashAlbum];

    pub fn label(self) -> &'static str {
        match self {
            Self::ArtistAndAlbum => "Artist / Album",
            Self::AlbumPrefix => "Album - Name",
            Self::ArtistDashAlbum => "Artist - Album",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VibesConfig {
    #[serde(default)]
    pub theme: ThemePreset,
    #[serde(default = "default_country_code")]
    pub country_code: String,
    #[serde(default = "default_true")]
    pub local_blending: bool,
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default = "default_true")]
    pub visualizer_enabled: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub lyrics_overlay_enabled: bool,
    #[serde(default = "default_music_dir")]
    pub music_dir: PathBuf,
    #[serde(default)]
    pub save_folder_mode: SaveFolderMode,
    #[serde(default)]
    pub save_audio_format: SaveAudioFormat,
    #[serde(default = "default_albums_dir")]
    pub albums_dir: PathBuf,
    #[serde(default)]
    pub album_folder_mode: AlbumFolderMode,
}

fn default_true() -> bool {
    true
}

fn default_volume() -> f64 {
    80.0
}

fn default_albums_dir() -> PathBuf {
    dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_music_dir() -> PathBuf {
    default_albums_dir().join("Radio")
}

fn default_country_code() -> String {
    if let Ok(lang) = std::env::var("LANG") {
        let code_part = lang.split('.').next().unwrap_or("");
        if let Some((_, country)) = code_part.split_once('_') {
            let trimmed = country.trim();
            if trimmed.len() == 2 && trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
                return trimmed.to_uppercase();
            }
        }
    }
    String::new()
}

impl Default for VibesConfig {
    fn default() -> Self {
        Self {
            theme: ThemePreset::VibesCrimson,
            country_code: default_country_code(),
            local_blending: true,
            volume: 80.0,
            visualizer_enabled: true,
            close_to_tray: true,
            lyrics_overlay_enabled: false,
            music_dir: default_music_dir(),
            save_folder_mode: SaveFolderMode::Flat,
            save_audio_format: SaveAudioFormat::Opus,
            albums_dir: default_albums_dir(),
            album_folder_mode: AlbumFolderMode::ArtistAndAlbum,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SongLogEntry {
    pub timestamp: String,
    pub track: String,
    pub station: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlayTarget {
    Radio(RadioStation),
    Youtube(YoutubeItem),
    Local(LocalTrack),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct VibesState {
    #[serde(default)]
    pub favorites: Vec<RadioStation>,
    #[serde(default)]
    pub custom_stations: Vec<RadioStation>,
    #[serde(default)]
    pub song_log: Vec<SongLogEntry>,
    #[serde(default)]
    pub resume_positions: HashMap<String, f64>,
    #[serde(default)]
    pub recent_yt_searches: Vec<String>,
    #[serde(default)]
    pub last_played: Option<PlayTarget>,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("vibes")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn state_path() -> PathBuf {
    config_dir().join("state.json")
}

pub fn atomic_write_json<T: Serialize>(path: &Path, val: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(val)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

impl VibesConfig {
    pub fn load() -> Self {
        fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        atomic_write_json(&config_path(), self)
    }
}

impl VibesState {
    pub fn load() -> Self {
        fs::read_to_string(state_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        atomic_write_json(&state_path(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_and_state_roundtrip_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg_file = tmp.path().join("config.json");
        let state_file = tmp.path().join("state.json");

        let cfg = VibesConfig {
            theme: ThemePreset::BreezeDark,
            country_code: "RU".to_string(),
            local_blending: true,
            volume: 65.0,
            visualizer_enabled: false,
            close_to_tray: true,
            lyrics_overlay_enabled: false,
            music_dir: tmp.path().join("Music"),
            save_folder_mode: SaveFolderMode::ByArtist,
            save_audio_format: SaveAudioFormat::Opus,
            albums_dir: tmp.path().join("Albums"),
            album_folder_mode: AlbumFolderMode::AlbumPrefix,
        };
        atomic_write_json(&cfg_file, &cfg).unwrap();
        let parsed_cfg: VibesConfig =
            serde_json::from_str(&fs::read_to_string(&cfg_file).unwrap()).unwrap();
        assert_eq!(cfg, parsed_cfg);

        let state = VibesState {
            favorites: vec![RadioStation::custom("SomaFM", "https://ice1.somafm.com/groovesalad-128-mp3")],
            custom_stations: Vec::new(),
            song_log: vec![SongLogEntry {
                timestamp: "03:45".to_string(),
                track: "Artist - Song".to_string(),
                station: "SomaFM".to_string(),
                source_url: "https://ice1.somafm.com/groovesalad-128-mp3".to_string(),
            }],
            resume_positions: HashMap::from([("vid1".to_string(), 1450.5)]),
            recent_yt_searches: vec!["ambient mix".to_string()],
            last_played: None,
        };
        atomic_write_json(&state_file, &state).unwrap();
        let parsed_state: VibesState =
            serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
        assert_eq!(state, parsed_state);
    }
}
