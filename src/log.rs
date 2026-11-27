use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 512 * 1024;
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn log_file_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("vibes")
        .join("vibes.log")
}

fn append_log_to_path(path: &Path, max_bytes: u64, level: &str, source: &str, msg: &str) {
    let clean = msg
        .replace('\r', " ")
        .replace('\n', " ")
        .trim()
        .to_string();
    if clean.is_empty() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(path).map(|m| m.len()).unwrap_or(0) >= max_bytes {
        let backup = path.with_extension("log.1");
        let _ = fs::rename(path, backup);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "[{ts}] [{level}] [{source}] {clean}");
    }
}

pub fn write_log_line(level: &str, source: &str, msg: &str) {
    let lock = LOG_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    append_log_to_path(&log_file_path(), MAX_LOG_BYTES, level, source, msg);
}

pub fn log_info(source: &str, msg: &str) {
    write_log_line("INFO", source, msg);
}

pub fn log_warn(source: &str, msg: &str) {
    write_log_line("WARN", source, msg);
}

pub fn log_error(source: &str, msg: &str) {
    write_log_line("ERROR", source, msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_rotates_rolling_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("vibes.log");
        let backup_path = log_path.with_extension("log.1");

        append_log_to_path(&log_path, 80, "INFO", "test", "first message line\nwith newline");
        let content1 = fs::read_to_string(&log_path).unwrap();
        assert!(content1.contains("[INFO] [test] first message line with newline"));

        append_log_to_path(
            &log_path,
            80,
            "WARN",
            "mpv",
            "second long warning line that triggers rotation threshold cleanly",
        );
        append_log_to_path(&log_path, 80, "ERROR", "mpv", "third message after rotate");

        assert!(backup_path.exists());
        let latest = fs::read_to_string(&log_path).unwrap();
        assert!(latest.contains("[ERROR] [mpv] third message after rotate"));
    }
}
