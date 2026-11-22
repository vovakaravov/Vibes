use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;

fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("vibes-instance.sock")
}

/// Returns `true` if an existing Vibes instance accepted the "SHOW" signal.
pub fn notify_existing_instance() -> bool {
    let path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return false;
    };
    let _ = stream.write_all(b"SHOW\n");
    let _ = stream.flush();
    true
}

pub struct InstanceGuard {
    path: PathBuf,
}

impl InstanceGuard {
    pub fn start(show_tx: async_channel::Sender<()>) -> Option<Self> {
        let path = socket_path();
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).ok()?;

        thread::Builder::new()
            .name("vibes-single-instance".into())
            .spawn(move || {
                for stream_res in listener.incoming() {
                    let Ok(mut stream) = stream_res else {
                        break;
                    };
                    let mut buf = [0u8; 32];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let msg = String::from_utf8_lossy(&buf[..n]);
                    if msg.contains("SHOW") {
                        let _ = show_tx.try_send(());
                    }
                }
            })
            .ok();

        Some(Self { path })
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
