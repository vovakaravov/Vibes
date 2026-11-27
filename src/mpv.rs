use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    PauseChanged(bool),
    BufferingChanged(bool),
    FileLoaded,
    PlaybackRestart,
    MediaTitleChanged(String),
    IcyMetadata {
        title: Option<String>,
        name: Option<String>,
    },
    TimePosChanged(f64),
    DurationChanged(Option<f64>),
    AudioCodecChanged(String),
    AudioBitrateChanged(u64),
    SampleRateChanged(u32),
    VolumeChanged(f64),
    MuteChanged(bool),
    SpeedChanged(f64),
    EndFile {
        reason: String,
    },
    Error(String),
    Log {
        level: String,
        prefix: String,
        text: String,
    },
}

/// Parses one JSON line emitted by `mpv`'s `--input-ipc-server` socket.
pub fn parse_mpv_ipc_line(line: &str) -> Vec<MpvEvent> {
    let Ok(val) = serde_json::from_str::<Value>(line.trim()) else {
        return Vec::new();
    };
    let Some(ev_name) = val.get("event").and_then(|v| v.as_str()) else {
        if let Some(err) = val.get("error").and_then(|v| v.as_str())
            && err != "success"
        {
            let req_id = val.get("request_id").and_then(|v| v.as_u64());
            let text = match req_id {
                Some(id) => format!("IPC command #{id} failed: {err}"),
                None => format!("IPC command failed: {err}"),
            };
            return vec![MpvEvent::Log {
                level: "error".to_string(),
                prefix: "mpv-ipc".to_string(),
                text,
            }];
        }
        return Vec::new();
    };

    let mut events = Vec::new();
    match ev_name {
        "property-change" => {
            let prop = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let data = val.get("data");
            match prop {
                "pause" => {
                    if let Some(b) = data.and_then(|v| v.as_bool()) {
                        events.push(MpvEvent::PauseChanged(b));
                    }
                }
                "paused-for-cache" => {
                    if let Some(b) = data.and_then(|v| v.as_bool()) {
                        events.push(MpvEvent::BufferingChanged(b));
                    }
                }
                "media-title" => {
                    if let Some(s) = data.and_then(|v| v.as_str()) {
                        let t = s.trim();
                        if !t.is_empty() {
                            events.push(MpvEvent::MediaTitleChanged(t.to_string()));
                        }
                    }
                }
                "metadata" => {
                    if let Some(obj) = data.and_then(|v| v.as_object()) {
                        let find_key = |keys: &[&str]| -> Option<String> {
                            for (k, v) in obj {
                                if keys.iter().any(|want| k.eq_ignore_ascii_case(want))
                                    && let Some(s) = v.as_str()
                                {
                                    let t = s.trim();
                                    if !t.is_empty() {
                                        return Some(t.to_string());
                                    }
                                }
                            }
                            None
                        };
                        let title = find_key(&["icy-title", "title", "StreamTitle"]);
                        let name = find_key(&["icy-name", "artist", "StreamUrl"]);
                        if title.is_some() || name.is_some() {
                            events.push(MpvEvent::IcyMetadata { title, name });
                        }
                    }
                }
                "time-pos" => {
                    if let Some(pos) = data.and_then(|v| v.as_f64()) {
                        events.push(MpvEvent::TimePosChanged(pos.max(0.0)));
                    }
                }
                "duration" => {
                    let dur = data.and_then(|v| v.as_f64()).filter(|d| *d > 0.0);
                    events.push(MpvEvent::DurationChanged(dur));
                }
                "audio-codec-name" => {
                    if let Some(s) = data.and_then(|v| v.as_str()) {
                        let c = s.trim().to_uppercase();
                        if !c.is_empty() {
                            events.push(MpvEvent::AudioCodecChanged(c));
                        }
                    }
                }
                "audio-bitrate" => {
                    if let Some(br) = data.and_then(|v| v.as_f64()) {
                        let kbps = (br / 1000.0).round() as u64;
                        if kbps > 0 {
                            events.push(MpvEvent::AudioBitrateChanged(kbps));
                        }
                    }
                }
                "audio-params" => {
                    if let Some(sr) = data
                        .and_then(|v| v.get("samplerate"))
                        .and_then(|v| v.as_u64())
                        && sr > 0
                    {
                        events.push(MpvEvent::SampleRateChanged(sr as u32));
                    }
                }
                "volume" => {
                    if let Some(vol) = data.and_then(|v| v.as_f64()) {
                        events.push(MpvEvent::VolumeChanged(vol));
                    }
                }
                "mute" => {
                    if let Some(m) = data.and_then(|v| v.as_bool()) {
                        events.push(MpvEvent::MuteChanged(m));
                    }
                }
                "speed" => {
                    if let Some(spd) = data.and_then(|v| v.as_f64()) {
                        events.push(MpvEvent::SpeedChanged(spd));
                    }
                }
                _ => {}
            }
        }
        "file-loaded" => {
            events.push(MpvEvent::FileLoaded);
        }
        "playback-restart" => {
            events.push(MpvEvent::PlaybackRestart);
        }
        "end-file" => {
            let reason = val
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let err_msg = val
                .get("file_error")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if reason == "error"
                && let Some(msg) = err_msg
            {
                events.push(MpvEvent::Error(msg));
            }
            events.push(MpvEvent::EndFile { reason });
        }
        "log-message" => {
            let prefix = val.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
            let level = val.get("level").and_then(|v| v.as_str()).unwrap_or("");
            let text = val.get("text").and_then(|v| v.as_str()).unwrap_or("").trim();
            if !text.is_empty() {
                events.push(MpvEvent::Log {
                    level: level.to_string(),
                    prefix: prefix.to_string(),
                    text: text.to_string(),
                });
                if let Some(msg) = parse_mpv_log_message(prefix, level, text) {
                    events.push(MpvEvent::Error(msg));
                }
            }
        }
        _ => {}
    }
    events
}

pub struct MpvController {
    child: Mutex<Option<Child>>,
    writer: Arc<Mutex<Option<UnixStream>>>,
    socket_path: PathBuf,
    next_id: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedYoutubeStream {
    pub url: String,
    pub available_at: Option<SystemTime>,
}

pub const YTDL_AUDIO_FORMAT: &str =
    "bestaudio[protocol^=m3u8]/234/bestaudio[ext=webm]/bestaudio/best";

pub fn format_ytdl_raw_options(cookies_browser: Option<&str>) -> String {
    match cookies_browser.map(str::trim).filter(|b| !b.is_empty()) {
        Some(browser) => format!("no-playlist=,cookies-from-browser={browser}"),
        None => "no-playlist=".to_string(),
    }
}

pub fn build_mpv_args(
    initial_volume: f64,
    socket_path: &Path,
    cookies_browser: Option<&str>,
) -> Vec<String> {
    vec![
        "--no-config".to_string(),
        "--no-video".to_string(),
        "--audio-display=no".to_string(),
        "--no-osc".to_string(),
        "--no-osd-bar".to_string(),
        "--load-scripts=no".to_string(),
        "--idle=yes".to_string(),
        "--no-terminal".to_string(),
        "--force-window=no".to_string(),
        "--keep-open=no".to_string(),
        "--ytdl=yes".to_string(),
        format!("--ytdl-format={YTDL_AUDIO_FORMAT}"),
        format!("--ytdl-raw-options={}", format_ytdl_raw_options(cookies_browser)),
        "--script-opts=ytdl_hook-all_formats=no".to_string(),
        "--cache=yes".to_string(),
        "--cache-pause-initial=no".to_string(),
        "--cache-pause-wait=0.5".to_string(),
        "--demuxer-readahead-secs=10".to_string(),
        "--audio-buffer=0.15".to_string(),
        "--demuxer-max-bytes=16MiB".to_string(),
        "--demuxer-max-back-bytes=8MiB".to_string(),
        "--audio-client-name=Vibes".to_string(),
        format!("--volume={:.0}", initial_volume.clamp(0.0, 100.0)),
        format!("--input-ipc-server={}", socket_path.display()),
    ]
}

pub fn parse_ytdlp_stream_json(raw: &str) -> Result<ResolvedYoutubeStream> {
    let val: Value =
        serde_json::from_str(raw.trim()).context("Invalid JSON returned by yt-dlp -j")?;
    let url = val
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .ok_or_else(|| anyhow::anyhow!("Missing direct HTTP stream URL in yt-dlp output"))?
        .to_string();

    let available_at = val
        .get("available_at")
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.ceil() as u64)))
        .filter(|&ts| ts > 0)
        .map(|ts| UNIX_EPOCH + Duration::from_secs(ts + 1));

    Ok(ResolvedYoutubeStream { url, available_at })
}

pub fn parse_mpv_log_message(prefix: &str, level: &str, text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sign in to confirm") {
        return Some(
            "YouTube requires sign-in on this IP — pick your browser in Settings → YouTube Auth Cookies, or wait for rate limit to clear"
                .to_string(),
        );
    }
    if lower.contains("http error 403") {
        return Some("YouTube returned HTTP 403 for this stream".to_string());
    }
    if !matches!(level, "error" | "fatal") {
        return None;
    }
    if prefix == "ytdl_hook" {
        return Some(trimmed.to_string());
    }
    if prefix.starts_with("ao") {
        return Some(format!("Audio output error ({prefix}): {trimmed}"));
    }
    if matches!(prefix, "lavfi" | "af") {
        return Some(format!("Audio filter error: {trimmed}"));
    }
    if matches!(prefix, "ffmpeg" | "curl" | "stream" | "tls") {
        return Some(format!("Stream error ({prefix}): {trimmed}"));
    }
    None
}

/// Resolves a direct audio stream CDN URL and its `available_at` timestamp via `yt-dlp -j`.
pub fn resolve_youtube_audio_stream(
    url: &str,
    cookies_browser: Option<&str>,
) -> Result<ResolvedYoutubeStream> {
    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        "-f",
        YTDL_AUDIO_FORMAT,
        "-j",
        "--no-playlist",
        "--no-warnings",
    ]);
    if let Some(browser) = cookies_browser.map(str::trim).filter(|b| !b.is_empty()) {
        cmd.args(["--cookies-from-browser", browser]);
    }
    cmd.arg(url);
    let out = cmd.output().context("Failed to execute `yt-dlp`")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.lines().last().unwrap_or("yt-dlp failed to resolve URL");
        anyhow::bail!("{}", msg.trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_ytdlp_stream_json(&stdout)
}

impl MpvController {
    pub fn spawn(
        initial_volume: f64,
        cookies_browser: Option<&str>,
        event_tx: async_channel::Sender<MpvEvent>,
    ) -> Result<Self> {
        let uid_or_tmp = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        let socket_path = uid_or_tmp.join(format!("vibes-mpv-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket_path);

        let args = build_mpv_args(initial_volume, &socket_path, cookies_browser);
        let child = Command::new("mpv")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn `mpv` daemon")?;

        let deadline = Instant::now() + Duration::from_secs(3);
        let stream = loop {
            if let Ok(s) = UnixStream::connect(&socket_path) {
                break s;
            }
            if Instant::now() >= deadline {
                anyhow::bail!("Timed out waiting for mpv IPC socket");
            }
            thread::sleep(Duration::from_millis(25));
        };

        let reader_stream = stream.try_clone().context("Failed to clone mpv socket")?;
        let writer = Arc::new(Mutex::new(Some(stream)));

        thread::Builder::new()
            .name("vibes-mpv-ipc".into())
            .spawn(move || {
                let reader = BufReader::new(reader_stream);
                for line_res in reader.lines() {
                    let Ok(line) = line_res else {
                        break;
                    };
                    for ev in parse_mpv_ipc_line(&line) {
                        if event_tx.send_blocking(ev).is_err() {
                            return;
                        }
                    }
                }
            })
            .ok();

        let ctrl = Self {
            child: Mutex::new(Some(child)),
            writer,
            socket_path,
            next_id: AtomicU64::new(1),
        };
        ctrl.observe_all()?;
        Ok(ctrl)
    }

    fn observe_all(&self) -> Result<()> {
        const PROPS: &[&str] = &[
            "pause",
            "paused-for-cache",
            "media-title",
            "metadata",
            "time-pos",
            "duration",
            "audio-codec-name",
            "audio-bitrate",
            "audio-params",
            "volume",
            "mute",
            "speed",
        ];
        for (idx, prop) in PROPS.iter().enumerate() {
            let obs_id = (idx + 1) as u64;
            self.send_cmd(json!({
                "command": ["observe_property", obs_id, *prop]
            }))?;
        }
        self.send_cmd(json!({
            "command": ["request_log_messages", "warn"]
        }))?;
        Ok(())
    }

    fn send_cmd(&self, mut payload: Value) -> Result<()> {
        let req_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("request_id".to_string(), json!(req_id));
        }
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        let mut guard = self.writer.lock().unwrap();
        let Some(ref mut stream) = *guard else {
            anyhow::bail!("mpv IPC disconnected");
        };
        stream.write_all(line.as_bytes())?;
        stream.flush()?;
        Ok(())
    }

    pub fn load_url(&self, url: &str, start_secs: Option<f64>) -> Result<()> {
        self.send_cmd(json!({ "command": ["set_property", "pause", false] }))?;
        if let Some(secs) = start_secs.filter(|s| *s > 1.0) {
            let opt = format!("start=+{secs:.1}");
            self.send_cmd(json!({
                "command": ["loadfile", url, "replace", 0, { "start": format!("+{secs:.1}") }]
            }))
            .or_else(|_| self.send_cmd(json!({ "command": ["loadfile", url, "replace", &opt] })))
        } else {
            self.send_cmd(json!({ "command": ["loadfile", url, "replace"] }))
        }
    }

    pub fn set_pause(&self, paused: bool) -> Result<()> {
        self.send_cmd(json!({ "command": ["set_property", "pause", paused] }))
    }

    pub fn stop(&self) -> Result<()> {
        self.send_cmd(json!({ "command": ["stop"] }))
    }

    pub fn set_volume(&self, volume: f64) -> Result<()> {
        self.send_cmd(json!({
            "command": ["set_property", "volume", volume.clamp(0.0, 100.0)]
        }))
    }

    pub fn set_mute(&self, muted: bool) -> Result<()> {
        self.send_cmd(json!({ "command": ["set_property", "mute", muted] }))
    }

    pub fn seek_absolute(&self, secs: f64) -> Result<()> {
        self.send_cmd(json!({
            "command": ["seek", secs.max(0.0), "absolute"]
        }))
    }

    pub fn seek_relative(&self, delta: f64) -> Result<()> {
        self.send_cmd(json!({
            "command": ["seek", delta, "relative"]
        }))
    }

    pub fn set_speed(&self, speed: f64) -> Result<()> {
        self.send_cmd(json!({
            "command": ["set_property", "speed", speed.clamp(0.5, 2.5)]
        }))
    }

    pub fn set_audio_filter_label(&self, spec: &str) -> Result<()> {
        self.send_cmd(json!({ "command": ["af", "add", spec] }))
    }

    pub fn remove_audio_filter_label(&self, label: &str) -> Result<()> {
        self.send_cmd(json!({ "command": ["af", "remove", label] }))
    }

    pub fn set_ytdl_cookies(&self, cookies_browser: Option<&str>) -> Result<()> {
        self.send_cmd(json!({
            "command": ["set_property", "ytdl-raw-options", format_ytdl_raw_options(cookies_browser)]
        }))
    }

    pub fn shutdown(&self) {
        let _ = self.send_cmd(json!({ "command": ["quit"] }));
        *self.writer.lock().unwrap() = None;
        if let Some(mut child) = self.child.lock().unwrap().take() {
            thread::sleep(Duration::from_millis(30));
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

impl Drop for MpvController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mpv_ipc_property_changes_icy_metadata_and_end_file() {
        let icy = r#"{"event":"property-change","id":4,"name":"metadata","data":{"icy-title":"Carpenter Brut - Turbo Killer","icy-name":"Nightride FM"}}"#;
        assert_eq!(
            parse_mpv_ipc_line(icy),
            vec![MpvEvent::IcyMetadata {
                title: Some("Carpenter Brut - Turbo Killer".to_string()),
                name: Some("Nightride FM".to_string()),
            }]
        );

        let br = r#"{"event":"property-change","id":8,"name":"audio-bitrate","data":128000.0}"#;
        assert_eq!(
            parse_mpv_ipc_line(br),
            vec![MpvEvent::AudioBitrateChanged(128)]
        );

        let end = r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#;
        assert_eq!(
            parse_mpv_ipc_line(end),
            vec![
                MpvEvent::Error("loading failed".to_string()),
                MpvEvent::EndFile {
                    reason: "error".to_string()
                }
            ]
        );

        assert_eq!(
            parse_mpv_ipc_line(r#"{"event":"file-loaded"}"#),
            vec![MpvEvent::FileLoaded]
        );
        assert_eq!(
            parse_mpv_ipc_line(r#"{"event":"playback-restart"}"#),
            vec![MpvEvent::PlaybackRestart]
        );
        assert_eq!(
            parse_mpv_ipc_line(r#"{"event":"end-file","reason":"stop","playlist_entry_id":1}"#),
            vec![MpvEvent::EndFile {
                reason: "stop".to_string()
            }]
        );
    }

    #[test]
    fn mpv_args_cap_demuxer_cache_and_preserve_ytdl() {
        let args = build_mpv_args(72.0, Path::new("/tmp/vibes-test.sock"), None);
        assert!(args.iter().any(|a| a == "--demuxer-max-bytes=16MiB"));
        assert!(args.iter().any(|a| a == "--demuxer-max-back-bytes=8MiB"));
        assert!(args.iter().any(|a| a == "--cache-pause-initial=no"));
        assert!(args.iter().any(|a| a == "--demuxer-readahead-secs=10"));
        assert!(!args.iter().any(|a| a.contains("android_vr")));
        assert!(args.iter().any(|a| a == &format!("--ytdl-format={YTDL_AUDIO_FORMAT}")));
        assert!(args.iter().any(|a| a == "--ytdl-raw-options=no-playlist="));
        assert!(args.iter().any(|a| a == "--script-opts=ytdl_hook-all_formats=no"));
        assert!(args.iter().any(|a| a == "--audio-display=no"));
        assert!(args.iter().any(|a| a == "--ytdl=yes"));
        assert!(args.iter().any(|a| a == "--load-scripts=no"));
        assert!(args.iter().any(|a| a == "--volume=72"));
        assert!(args.iter().any(|a| a == "--input-ipc-server=/tmp/vibes-test.sock"));

        let args_cookie = build_mpv_args(50.0, Path::new("/tmp/vibes-test.sock"), Some("firefox"));
        assert!(
            args_cookie
                .iter()
                .any(|a| a == "--ytdl-raw-options=no-playlist=,cookies-from-browser=firefox")
        );
    }

    #[test]
    fn parses_ytdlp_stream_json_and_ipc_log_messages() {
        let json_str = r#"{"url":"https://example.com/audio.webm","available_at":1795752475}"#;
        let resolved = parse_ytdlp_stream_json(json_str).unwrap();
        assert_eq!(resolved.url, "https://example.com/audio.webm");
        assert_eq!(
            resolved.available_at,
            Some(UNIX_EPOCH + Duration::from_secs(1795752476))
        );

        let sign_in_ipc = r#"{"event":"log-message","prefix":"ytdl_hook","level":"error","text":"ERROR: [youtube] xyz: Sign in to confirm you're not a bot.\n"}"#;
        let evs = parse_mpv_ipc_line(sign_in_ipc);
        assert_eq!(evs.len(), 2);
        assert!(matches!(&evs[0], MpvEvent::Log { level, prefix, .. } if level == "error" && prefix == "ytdl_hook"));
        match &evs[1] {
            MpvEvent::Error(msg) => assert!(msg.contains("Settings")),
            other => panic!("Expected Error event, got {other:?}"),
        }

        let err_403_ipc =
            r#"{"event":"log-message","prefix":"curl","level":"error","text":"HTTP error 403"}"#;
        assert_eq!(
            parse_mpv_ipc_line(err_403_ipc),
            vec![
                MpvEvent::Log {
                    level: "error".to_string(),
                    prefix: "curl".to_string(),
                    text: "HTTP error 403".to_string(),
                },
                MpvEvent::Error("YouTube returned HTTP 403 for this stream".to_string()),
            ]
        );

        let ytdl_err_ipc = r#"{"event":"log-message","prefix":"ytdl_hook","level":"fatal","text":"Video unavailable"}"#;
        assert_eq!(
            parse_mpv_ipc_line(ytdl_err_ipc),
            vec![
                MpvEvent::Log {
                    level: "fatal".to_string(),
                    prefix: "ytdl_hook".to_string(),
                    text: "Video unavailable".to_string(),
                },
                MpvEvent::Error("Video unavailable".to_string()),
            ]
        );

        let ao_err_ipc = r#"{"event":"log-message","prefix":"ao/pipewire","level":"error","text":"device disconnected"}"#;
        assert!(
            parse_mpv_ipc_line(ao_err_ipc).contains(&MpvEvent::Error(
                "Audio output error (ao/pipewire): device disconnected".to_string()
            ))
        );

        let lavfi_err_ipc = r#"{"event":"log-message","prefix":"lavfi","level":"error","text":"filter init failed"}"#;
        assert!(
            parse_mpv_ipc_line(lavfi_err_ipc).contains(&MpvEvent::Error(
                "Audio filter error: filter init failed".to_string()
            ))
        );

        let ffmpeg_err_ipc = r#"{"event":"log-message","prefix":"ffmpeg","level":"error","text":"Connection timed out"}"#;
        assert!(
            parse_mpv_ipc_line(ffmpeg_err_ipc).contains(&MpvEvent::Error(
                "Stream error (ffmpeg): Connection timed out".to_string()
            ))
        );

        let ignored_info_ipc =
            r#"{"event":"log-message","prefix":"ao/pulse","level":"info","text":"48000Hz"}"#;
        assert_eq!(
            parse_mpv_ipc_line(ignored_info_ipc),
            vec![MpvEvent::Log {
                level: "info".to_string(),
                prefix: "ao/pulse".to_string(),
                text: "48000Hz".to_string(),
            }]
        );

        let ipc_reply_err = r#"{"request_id":9,"error":"invalid parameter"}"#;
        assert_eq!(
            parse_mpv_ipc_line(ipc_reply_err),
            vec![MpvEvent::Log {
                level: "error".to_string(),
                prefix: "mpv-ipc".to_string(),
                text: "IPC command #9 failed: invalid parameter".to_string(),
            }]
        );
        assert!(parse_mpv_ipc_line(r#"{"request_id":9,"error":"success"}"#).is_empty());
    }
}
