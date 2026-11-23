use crate::mpv::MpvController;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub const SPECTRUM_BANDS: usize = 24;
pub const VIS_FILTER_LABEL: &str = "@vibes_vis";
const CROSSOVER_BANDS: usize = 16;
const CROSSOVER_SPLITS_HZ: [u32; 15] = [
    82, 111, 152, 207, 282, 384, 524, 714, 973, 1326, 1808, 2464, 3358, 4576, 6237,
];
const HEXADECAGONAL_CHANNELS: [&str; CROSSOVER_BANDS] = [
    "FL", "FR", "FC", "BL", "BR", "BC", "SL", "SR", "TFL", "TFC", "TFR", "TBL", "TBC", "TBR",
    "WL", "WR",
];
static NEXT_FIFO_SEQ: AtomicU64 = AtomicU64::new(1);

fn fifo_base_dir() -> PathBuf {
    if let Some(dir) = dirs::runtime_dir() {
        let s = dir.to_string_lossy();
        if !s.contains([':', ',', ';', '\'', '[', ']']) {
            return dir;
        }
    }
    std::env::temp_dir()
}

fn next_fifo_path() -> PathBuf {
    let seq = NEXT_FIFO_SEQ.fetch_add(1, Ordering::Relaxed);
    fifo_base_dir().join(format!("vibes-vis-{}-{seq}.fifo", std::process::id()))
}

/// Builds the labeled `mpv` `lavfi` filter passing audio untouched to `[out]` while tapping a 16-band crossover into a FIFO.
pub fn build_mpv_af_spec(fifo_path: &Path) -> String {
    let splits = CROSSOVER_SPLITS_HZ
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let outputs: String = (0..CROSSOVER_BANDS).map(|i| format!("[b{i}]")).collect();
    let map = HEXADECAGONAL_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, ch)| format!("{i}.0-{ch}"))
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "{VIS_FILTER_LABEL}:lavfi=[asplit=2[out][tap];\
         [tap]aresample=22050,aformat=sample_fmts=fltp:channel_layouts=mono,asetnsamples=n=512:p=0,\
         acrossover=split={splits}:order=4th{outputs};\
         {outputs}join=inputs={CROSSOVER_BANDS}:channel_layout=hexadecagonal:map={map},\
         astats=metadata=1:reset=1:measure_overall=none:measure_perchannel=RMS_level,\
         ametadata=mode=print:direct=1:file={},anullsink]",
        fifo_path.display()
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AmetadataLine {
    FrameStart,
    BandRms(usize, f32),
}

/// Parses one line emitted by FFmpeg `ametadata=mode=print` for per-channel `astats` RMS levels.
pub fn parse_ametadata_line(line: &str) -> Option<AmetadataLine> {
    let trimmed = line.trim();
    if trimmed.starts_with("frame:") {
        return Some(AmetadataLine::FrameStart);
    }
    let rest = trimmed.strip_prefix("lavfi.astats.")?;
    let (ch_str, val_str) = rest.split_once(".RMS_level=")?;
    let ch_one_based: usize = ch_str.parse().ok()?;
    if !(1..=CROSSOVER_BANDS).contains(&ch_one_based) {
        return None;
    }
    let db =
        if val_str.eq_ignore_ascii_case("-inf") || val_str.eq_ignore_ascii_case("-infinity") {
            f32::NEG_INFINITY
        } else {
            val_str.parse::<f32>().ok()?
        };
    Some(AmetadataLine::BandRms(ch_one_based - 1, db))
}

pub fn rms_db_to_bar(db: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    ((db + 54.0) / 40.0).clamp(0.0, 1.0).powf(1.25)
}

/// Maps 16 crossover RMS dB levels into 24 smoothly decaying visual spectrum bars.
pub fn update_bars_from_db(
    band_db: &[f32; CROSSOVER_BANDS],
    prev: &mut [f32; SPECTRUM_BANDS],
) -> [f32; SPECTRUM_BANDS] {
    let mut src = [0.0f32; CROSSOVER_BANDS];
    for (slot, &db) in src.iter_mut().zip(band_db.iter()) {
        *slot = rms_db_to_bar(db);
    }

    let mut next = [0.0f32; SPECTRUM_BANDS];
    let scale = (CROSSOVER_BANDS - 1) as f32 / (SPECTRUM_BANDS - 1) as f32;
    for (i, slot) in next.iter_mut().enumerate() {
        let pos = (i as f32) * scale;
        let lo = (pos.floor() as usize).min(CROSSOVER_BANDS - 1);
        let hi = (lo + 1).min(CROSSOVER_BANDS - 1);
        let frac = pos - (lo as f32);
        let target = src[lo] + (src[hi] - src[lo]) * frac;
        *slot = if target > prev[i] {
            prev[i] * 0.30 + target * 0.70
        } else {
            (prev[i] * 0.80).max(target)
        };
    }

    *prev = next;
    next
}

pub struct VisualizerController {
    bars: Arc<Mutex<[f32; SPECTRUM_BANDS]>>,
    running: Arc<AtomicBool>,
    fifo_path: Option<PathBuf>,
    wake_writer: Option<File>,
    worker: Option<JoinHandle<()>>,
    filter_attached: bool,
    animating: bool,
}

impl Default for VisualizerController {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualizerController {
    pub fn new() -> Self {
        Self {
            bars: Arc::new(Mutex::new([0.04; SPECTRUM_BANDS])),
            running: Arc::new(AtomicBool::new(false)),
            fifo_path: None,
            wake_writer: None,
            worker: None,
            filter_attached: false,
            animating: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.animating
    }

    pub fn snapshot(&self) -> [f32; SPECTRUM_BANDS] {
        *self.bars.lock().unwrap()
    }

    fn ensure_reader(&mut self) -> Option<PathBuf> {
        if let Some(ref path) = self.fifo_path
            && self.worker.is_some()
        {
            return Some(path.clone());
        }

        let path = next_fifo_path();
        let _ = fs::remove_file(&path);
        let ok = Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = fs::remove_file(&path);
            return None;
        }

        // Open O_RDWR so opening never blocks on mpv and mpv closing/reopening doesn't send EOF.
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
            let _ = fs::remove_file(&path);
            return None;
        };
        let Ok(wake) = file.try_clone() else {
            let _ = fs::remove_file(&path);
            return None;
        };

        self.fifo_path = Some(path.clone());
        self.wake_writer = Some(wake);
        self.running.store(true, Ordering::Release);

        let running = self.running.clone();
        let bars_lock = self.bars.clone();
        let handle = thread::Builder::new()
            .name("vibes-visualizer".into())
            .spawn(move || {
                let mut reader = BufReader::new(file);
                let mut line = String::new();
                let mut frame_db = [f32::NEG_INFINITY; CROSSOVER_BANDS];
                let mut frame_dirty = false;
                let mut prev = [0.04f32; SPECTRUM_BANDS];

                while running.load(Ordering::Acquire) {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if !running.load(Ordering::Acquire) {
                                break;
                            }
                            match parse_ametadata_line(&line) {
                                Some(AmetadataLine::FrameStart) => {
                                    if frame_dirty {
                                        let next = update_bars_from_db(&frame_db, &mut prev);
                                        *bars_lock.lock().unwrap() = next;
                                        frame_db = [f32::NEG_INFINITY; CROSSOVER_BANDS];
                                        frame_dirty = false;
                                    }
                                }
                                Some(AmetadataLine::BandRms(idx, db)) => {
                                    frame_db[idx] = db;
                                    frame_dirty = true;
                                    if idx == CROSSOVER_BANDS - 1 {
                                        let next = update_bars_from_db(&frame_db, &mut prev);
                                        *bars_lock.lock().unwrap() = next;
                                        frame_db = [f32::NEG_INFINITY; CROSSOVER_BANDS];
                                        frame_dirty = false;
                                    }
                                }
                                None => {}
                            }
                        }
                    }
                }
            })
            .ok();
        self.worker = handle;
        Some(path)
    }

    /// Synchronizes the in-process `mpv` `lavfi` visualizer filter and UI animation state without touching PulseAudio/PipeWire capture.
    pub fn sync(&mut self, want_filter: bool, want_anim: bool, mpv: Option<&MpvController>) {
        self.animating = want_filter && want_anim;
        if !self.animating {
            *self.bars.lock().unwrap() = [0.04; SPECTRUM_BANDS];
        }

        if want_filter && !self.filter_attached {
            if let Some(mpv) = mpv
                && let Some(fifo) = self.ensure_reader()
                && mpv
                    .set_audio_filter_label(&build_mpv_af_spec(&fifo))
                    .is_ok()
            {
                self.filter_attached = true;
            }
        } else if !want_filter && self.filter_attached {
            if let Some(mpv) = mpv {
                let _ = mpv.remove_audio_filter_label(VIS_FILTER_LABEL);
            }
            self.filter_attached = false;
        }
    }

    pub fn stop(&mut self) {
        self.animating = false;
        self.filter_attached = false;
        self.running.store(false, Ordering::Release);
        if let Some(mut w) = self.wake_writer.take() {
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some(path) = self.fifo_path.take() {
            let _ = fs::remove_file(path);
        }
        *self.bars.lock().unwrap() = [0.04; SPECTRUM_BANDS];
    }
}

impl Drop for VisualizerController {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn parses_ametadata_lines_and_interpolates_24_bars_with_decay() {
        assert_eq!(
            parse_ametadata_line("frame:23   pts:12288   pts_time:0.557279\n"),
            Some(AmetadataLine::FrameStart)
        );
        assert_eq!(
            parse_ametadata_line("lavfi.astats.7.RMS_level=-30.408648\n"),
            Some(AmetadataLine::BandRms(6, -30.408648))
        );
        assert_eq!(
            parse_ametadata_line("lavfi.astats.1.RMS_level=-inf\n"),
            Some(AmetadataLine::BandRms(0, f32::NEG_INFINITY))
        );
        assert_eq!(parse_ametadata_line("lavfi.astats.17.RMS_level=-10"), None);
        assert_eq!(parse_ametadata_line("unrelated=1"), None);

        assert_eq!(rms_db_to_bar(f32::NEG_INFINITY), 0.0);
        assert!(rms_db_to_bar(-22.0) > rms_db_to_bar(-40.0));

        let mut prev = [0.0f32; SPECTRUM_BANDS];
        let mut bass = [f32::NEG_INFINITY; CROSSOVER_BANDS];
        bass[1] = -20.0;
        let bars = update_bars_from_db(&bass, &mut prev);
        assert!(bars[1] > 0.30 && bars[20] < 0.05);

        let silence = [f32::NEG_INFINITY; CROSSOVER_BANDS];
        let decayed = update_bars_from_db(&silence, &mut prev);
        assert!(decayed[1] < bars[1] && decayed[1] > 0.1);
    }

    #[test]
    fn mpv_lavfi_visualizer_reads_live_spectrum_via_fifo_without_parec() {
        let mut vis = VisualizerController::new();
        let fifo = vis.ensure_reader().expect("FIFO reader should initialize");
        assert!(fifo.exists());
        let af = build_mpv_af_spec(&fifo);

        let status = Command::new("mpv")
            .args([
                "--no-config",
                "--no-video",
                "--ao=null",
                "--length=0.45",
                &format!("--af={af}"),
                "av://lavfi:sine=frequency=440:sample_rate=48000:duration=0.45",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("mpv should execute");
        assert!(status.success());

        let deadline = Instant::now() + Duration::from_millis(500);
        let mut bars = vis.snapshot();
        while Instant::now() < deadline {
            bars = vis.snapshot();
            if bars[8..=11].iter().copied().fold(0.0f32, f32::max) > 0.25 {
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }

        let mid_peak = bars[8..=11].iter().copied().fold(0.0f32, f32::max);
        let high_peak = bars[20..SPECTRUM_BANDS].iter().copied().fold(0.0f32, f32::max);
        assert!(
            mid_peak > 0.25,
            "expected clear 440Hz mid-band peak, got {mid_peak} (bars={bars:?})"
        );
        assert!(
            high_peak < 0.10,
            "expected silent upper bands, got {high_peak} (bars={bars:?})"
        );

        vis.stop();
        assert!(!fifo.exists(), "stop() should remove temporary FIFO");
    }
}
