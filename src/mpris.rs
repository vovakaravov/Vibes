use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::blocking::Connection;
use zbus::fdo;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

#[derive(Debug, Clone)]
pub enum MprisAction {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekMicros(i64),
    Raise,
    Quit,
}

#[derive(Debug, Clone)]
pub struct MprisSnapshot {
    pub playback_status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length_us: Option<i64>,
    pub position_us: i64,
    pub volume: f64,
}

impl Default for MprisSnapshot {
    fn default() -> Self {
        Self {
            playback_status: "Stopped".to_string(),
            title: "Vibes".to_string(),
            artist: String::new(),
            album: "Vibes Radio & Audio".to_string(),
            length_us: None,
            position_us: 0,
            volume: 0.8,
        }
    }
}

struct MediaPlayer2 {
    tx: async_channel::Sender<MprisAction>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl MediaPlayer2 {
    fn raise(&self) {
        let _ = self.tx.try_send(MprisAction::Raise);
    }

    fn quit(&self) {
        let _ = self.tx.try_send(MprisAction::Quit);
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        "Vibes".to_string()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "vibes".to_string()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["http".into(), "https".into()]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec!["audio/mpeg".into(), "audio/aac".into(), "audio/ogg".into(), "audio/opus".into()]
    }
}

struct MediaPlayer2Player {
    snap: Arc<Mutex<MprisSnapshot>>,
    tx: async_channel::Sender<MprisAction>,
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MediaPlayer2Player {
    fn next(&self) {
        let _ = self.tx.try_send(MprisAction::Next);
    }

    fn previous(&self) {
        let _ = self.tx.try_send(MprisAction::Previous);
    }

    fn pause(&self) {
        let _ = self.tx.try_send(MprisAction::Pause);
    }

    fn play_pause(&self) {
        let _ = self.tx.try_send(MprisAction::PlayPause);
    }

    fn stop(&self) {
        let _ = self.tx.try_send(MprisAction::Stop);
    }

    fn play(&self) {
        let _ = self.tx.try_send(MprisAction::Play);
    }

    fn seek(&self, offset: i64) {
        let _ = self.tx.try_send(MprisAction::SeekMicros(offset));
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.snap.lock().unwrap().playback_status.clone()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let snap = self.snap.lock().unwrap().clone();
        let mut map: HashMap<String, OwnedValue> = HashMap::new();
        if let Ok(path) = ObjectPath::try_from("/org/mpris/MediaPlayer2/Track/1")
            && let Ok(val) = Value::from(path).try_to_owned()
        {
            map.insert("mpris:trackid".to_string(), val);
        }
        if let Ok(val) = Value::from(snap.title).try_to_owned() {
            map.insert("xesam:title".to_string(), val);
        }
        if !snap.artist.is_empty()
            && let Ok(val) = Value::from(vec![snap.artist]).try_to_owned()
        {
            map.insert("xesam:artist".to_string(), val);
        }
        if !snap.album.is_empty()
            && let Ok(val) = Value::from(snap.album).try_to_owned()
        {
            map.insert("xesam:album".to_string(), val);
        }
        if let Some(len) = snap.length_us
            && let Ok(val) = Value::from(len).try_to_owned()
        {
            map.insert("mpris:length".to_string(), val);
        }
        map
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.snap.lock().unwrap().volume.clamp(0.0, 1.0)
    }

    #[zbus(property)]
    fn set_volume(&self, _v: f64) -> fdo::Result<()> {
        Ok(())
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.snap.lock().unwrap().position_us
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        0.5
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        2.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

pub struct MprisController {
    conn: Option<Connection>,
    snap: Arc<Mutex<MprisSnapshot>>,
}

impl MprisController {
    pub fn spawn(tx: async_channel::Sender<MprisAction>) -> Self {
        let snap = Arc::new(Mutex::new(MprisSnapshot::default()));
        let conn = Self::try_connect(tx, snap.clone());
        Self { conn, snap }
    }

    fn try_connect(
        tx: async_channel::Sender<MprisAction>,
        snap: Arc<Mutex<MprisSnapshot>>,
    ) -> Option<Connection> {
        let conn = Connection::session().ok()?;
        let root = MediaPlayer2 { tx: tx.clone() };
        let player = MediaPlayer2Player { snap, tx };
        let path = "/org/mpris/MediaPlayer2";
        conn.object_server().at(path, root).ok()?;
        conn.object_server().at(path, player).ok()?;
        conn.request_name("org.mpris.MediaPlayer2.vibes").ok()?;
        Some(conn)
    }

    pub fn sync(&self, next: MprisSnapshot) {
        let prev = {
            let mut guard = self.snap.lock().unwrap();
            let old = guard.clone();
            *guard = next.clone();
            old
        };
        let Some(ref conn) = self.conn else {
            return;
        };
        let Ok(iface_ref) = conn
            .object_server()
            .interface::<_, MediaPlayer2Player>("/org/mpris/MediaPlayer2")
        else {
            return;
        };
        let emitter: &SignalEmitter<'static> = iface_ref.signal_emitter();
        let status_changed = prev.playback_status != next.playback_status;
        let meta_changed = prev.title != next.title
            || prev.artist != next.artist
            || prev.album != next.album
            || prev.length_us != next.length_us;
        let vol_changed = (prev.volume - next.volume).abs() > 0.01;

        if status_changed || meta_changed || vol_changed {
            let iface = iface_ref.get();
            async_io::block_on(async {
                if status_changed {
                    let _ = iface.playback_status_changed(emitter).await;
                }
                if meta_changed {
                    let _ = iface.metadata_changed(emitter).await;
                }
                if vol_changed {
                    let _ = iface.volume_changed(emitter).await;
                }
            });
        }
    }
}
