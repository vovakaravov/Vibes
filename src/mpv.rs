use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
}

/// Parses one JSON line emitted by `mpv`'s `--input-ipc-server` socket.
pub fn parse_mpv_ipc_line(line: &str) -> Vec<MpvEvent> {
    let Ok(val) = serde_json::from_str::<Value>(line.trim()) else {
        return Vec::new();
    };
    let Some(ev_name) = val.get("event").and_then(|v| v.as_str()) else {
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

impl MpvController {
    pub fn spawn(
        initial_volume: f64,
        event_tx: async_channel::Sender<MpvEvent>,
    ) -> Result<Self> {
        let uid_or_tmp = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        let socket_path = uid_or_tmp.join(format!("vibes-mpv-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket_path);

        let child = Command::new("mpv")
            .args([
                "--no-video",
                "--idle=yes",
                "--no-terminal",
                "--force-window=no",
                "--keep-open=no",
                "--ytdl=yes",
                "--ytdl-format=bestaudio[ext=webm]/bestaudio/best",
                "--audio-client-name=Vibes",
                &format!("--volume={:.0}", initial_volume.clamp(0.0, 100.0)),
                &format!("--input-ipc-server={}", socket_path.display()),
            ])
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
}
