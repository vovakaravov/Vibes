use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub const SPECTRUM_BANDS: usize = 24;
const FFT_SIZE: usize = 256;
const SAMPLE_RATE: f32 = 22050.0;

#[derive(Clone, Copy)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    fn mag(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

/// In-place 256-point radix-2 Cooley-Tukey FFT.
fn fft_radix2(buf: &mut [Complex; FFT_SIZE]) {
    let n = FFT_SIZE;
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let angle = -std::f32::consts::TAU / (len as f32);
        let wlen = Complex::new(angle.cos(), angle.sin());
        let half = len / 2;
        for start in (0..n).step_by(len) {
            let mut w = Complex::new(1.0, 0.0);
            for k in 0..half {
                let u = buf[start + k];
                let v = buf[start + k + half];
                let tv = Complex::new(v.re * w.re - v.im * w.im, v.re * w.im + v.im * w.re);
                buf[start + k] = Complex::new(u.re + tv.re, u.im + tv.im);
                buf[start + k + half] = Complex::new(u.re - tv.re, u.im - tv.im);
                w = Complex::new(w.re * wlen.re - w.im * wlen.im, w.re * wlen.im + w.im * wlen.re);
            }
        }
        len *= 2;
    }
}

/// Computes 24 log-spaced spectrum bars from a 256-sample mono frame with smooth decay.
pub fn compute_spectrum_bars(
    samples: &[f32; FFT_SIZE],
    prev: &mut [f32; SPECTRUM_BANDS],
) -> [f32; SPECTRUM_BANDS] {
    let mut fft_buf = [Complex::new(0.0, 0.0); FFT_SIZE];
    for i in 0..FFT_SIZE {
        let hann = 0.5 * (1.0 - ((std::f32::consts::TAU * i as f32) / (FFT_SIZE as f32 - 1.0)).cos());
        fft_buf[i] = Complex::new(samples[i] * hann, 0.0);
    }
    fft_radix2(&mut fft_buf);

    let bin_hz = SAMPLE_RATE / (FFT_SIZE as f32);
    let min_hz = 60.0f32;
    let max_hz = 8500.0f32;
    let ratio = max_hz / min_hz;
    let mut next = [0.0f32; SPECTRUM_BANDS];

    for (band, slot) in next.iter_mut().enumerate() {
        let f_lo = min_hz * ratio.powf((band as f32) / (SPECTRUM_BANDS as f32));
        let f_hi = min_hz * ratio.powf(((band + 1) as f32) / (SPECTRUM_BANDS as f32));
        let b_lo = ((f_lo / bin_hz).floor() as usize).clamp(1, FFT_SIZE / 2 - 1);
        let b_hi = ((f_hi / bin_hz).ceil() as usize).clamp(b_lo + 1, FFT_SIZE / 2);

        let mut peak = 0.0f32;
        for bin in &fft_buf[b_lo..b_hi] {
            peak = peak.max(bin.mag() / (FFT_SIZE as f32 * 0.25));
        }

        let db_scaled = ((1.0 + peak * 8.5).ln() / 2.25).clamp(0.0, 1.0);
        let smoothed = if db_scaled > prev[band] {
            prev[band] * 0.35 + db_scaled * 0.65
        } else {
            (prev[band] * 0.80).max(db_scaled)
        };
        *slot = smoothed;
    }

    *prev = next;
    next
}

pub struct VisualizerController {
    bars: Arc<Mutex<[f32; SPECTRUM_BANDS]>>,
    running: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    worker: Option<JoinHandle<()>>,
    active: bool,
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
            child: Arc::new(Mutex::new(None)),
            worker: None,
            active: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn snapshot(&self) -> [f32; SPECTRUM_BANDS] {
        *self.bars.lock().unwrap()
    }

    /// Starts or tears down the `parec` monitor tap so tray-only mode uses 0% CPU.
    pub fn set_active(&mut self, should_run: bool) {
        if self.active == should_run {
            return;
        }
        self.active = should_run;
        if should_run {
            self.start();
        } else {
            self.stop();
        }
    }

    fn start(&mut self) {
        self.stop_inner();
        let child = match Command::new("parec")
            .args([
                "--raw",
                "--format=float32le",
                "--rate=22050",
                "--channels=1",
                "--latency-msec=30",
                "--device=@DEFAULT_MONITOR@",
                "--client-name=Vibes-Visualizer",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let mut c = child;
        let Some(mut stdout) = c.stdout.take() else {
            let _ = c.kill();
            let _ = c.wait();
            return;
        };
        *self.child.lock().unwrap() = Some(c);
        self.running.store(true, Ordering::Release);

        let running = self.running.clone();
        let bars_lock = self.bars.clone();
        let handle = thread::Builder::new()
            .name("vibes-visualizer".into())
            .spawn(move || {
                let mut raw = [0u8; FFT_SIZE * 4];
                let mut samples = [0.0f32; FFT_SIZE];
                let mut prev = [0.04f32; SPECTRUM_BANDS];
                while running.load(Ordering::Acquire) {
                    if stdout.read_exact(&mut raw).is_err() {
                        break;
                    }
                    for (idx, chunk) in raw.chunks_exact(4).enumerate() {
                        samples[idx] =
                            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    }
                    let next = compute_spectrum_bars(&samples, &mut prev);
                    *bars_lock.lock().unwrap() = next;
                }
            })
            .ok();
        self.worker = handle;
    }

    fn stop_inner(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.stop_inner();
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

    #[test]
    fn spectrum_detects_sine_wave_and_decays_on_silence() {
        let mut sine = [0.0f32; FFT_SIZE];
        for (i, s) in sine.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE;
            *s = (std::f32::consts::TAU * 440.0 * t).sin() * 0.7;
        }
        let mut prev = [0.0f32; SPECTRUM_BANDS];
        let bars = compute_spectrum_bars(&sine, &mut prev);
        let peak = bars.iter().copied().fold(0.0f32, f32::max);
        assert!(peak > 0.35, "expected clear FFT peak, got {peak}");

        let silence = [0.0f32; FFT_SIZE];
        let decayed = compute_spectrum_bars(&silence, &mut prev);
        let decayed_peak = decayed.iter().copied().fold(0.0f32, f32::max);
        assert!(decayed_peak < peak);
    }
}
