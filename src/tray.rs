use ksni::Tray;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayPlaybackState {
    #[default]
    Idle,
    Playing,
    Paused,
    Buffering,
}

#[derive(Debug, Clone)]
pub struct TraySnapshot {
    pub state: TrayPlaybackState,
    pub volume: f64,
    pub muted: bool,
    pub title: String,
    pub subtitle: String,
    pub window_open: bool,
    pub can_save_song: bool,
    pub can_copy_track: bool,
    pub favorites: Vec<(String, String)>,
    pub sleep_remaining_mins: Option<u64>,
    pub saving_active: bool,
}

impl Default for TraySnapshot {
    fn default() -> Self {
        Self {
            state: TrayPlaybackState::Idle,
            volume: 80.0,
            muted: false,
            title: "Vibes".to_string(),
            subtitle: "Ready to tune in".to_string(),
            window_open: true,
            can_save_song: false,
            can_copy_track: false,
            favorites: Vec::new(),
            sleep_remaining_mins: None,
            saving_active: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TrayAction {
    TogglePlay,
    ToggleWindow,
    PlayFavorite(String),
    SaveCurrentSong,
    CopyTrackName,
    AdjustVolume(f64),
    SetSleepTimer(Option<u64>),
    Quit,
}

/// Escapes `_` so DBusMenu doesn't interpret track/station underscores as mnemonic accelerators.
pub fn escape_menu_label(s: &str) -> String {
    s.replace('_', "__")
}

/// Maps `0..=100` volume + mute state to `0..=3` active radio wave rings.
pub fn volume_wave_level(volume: f64, muted: bool) -> u8 {
    if muted || volume <= 0.5 {
        0
    } else if volume <= 33.0 {
        1
    } else if volume <= 66.0 {
        2
    } else {
        3
    }
}

fn put_argb(data: &mut [u8], width: i32, x: i32, y: i32, argb: [u8; 4]) {
    if x < 0 || y < 0 || x >= width || y >= width {
        return;
    }
    let idx = ((y * width + x) * 4) as usize;
    if argb[0] >= data[idx] {
        data[idx] = argb[0];
        data[idx + 1] = argb[1];
        data[idx + 2] = argb[2];
        data[idx + 3] = argb[3];
    }
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let len_sq = vx * vx + vy * vy;
    let t = if len_sq > 0.0 {
        ((wx * vx + wy * vy) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = px - (ax + t * vx);
    let dy = py - (ay + t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// Generates a 32x32 ARGB32 (network byte order) tray icon reflecting playback state and volume.
pub fn render_tray_icon(state: TrayPlaybackState, volume: f64, muted: bool) -> ksni::Icon {
    const W: i32 = 32;
    let mut data = vec![0u8; (W * W * 4) as usize];

    match state {
        TrayPlaybackState::Idle => {
            let cx = 15.5f32;
            let cy = 15.5f32;
            for y in 0..W {
                for x in 0..W {
                    let dx = (x as f32) - cx;
                    let dy = (y as f32) - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist <= 12.2 {
                        // Subtle translucent badge fill
                        put_argb(&mut data, W, x, y, [75, 0x28, 0x27, 0x27]);
                    }
                    let ring_d = (dist - 11.4).abs();
                    if ring_d < 1.35 {
                        let a = ((1.0 - ring_d / 1.35) * 200.0).round() as u8;
                        put_argb(&mut data, W, x, y, [a, 0x72, 0x71, 0x69]);
                    }
                    if dist <= 3.3 {
                        let a = ((3.3 - dist).clamp(0.0, 1.0) * 235.0).round() as u8;
                        put_argb(&mut data, W, x, y, [a, 0x8b, 0xa4, 0xb0]);
                    }
                }
            }
        }
        TrayPlaybackState::Paused => {
            let cx = 15.5f32;
            let cy = 15.5f32;
            for y in 0..W {
                for x in 0..W {
                    let xf = x as f32;
                    let yf = y as f32;
                    let dx = xf - cx;
                    let dy = yf - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist <= 13.5 {
                        put_argb(&mut data, W, x, y, [185, 0x24, 0x22, 0x20]);
                    }
                    let ring_d = (dist - 13.0).abs();
                    if ring_d < 1.3 {
                        let a = ((1.0 - ring_d / 1.3) * 255.0).round() as u8;
                        put_argb(&mut data, W, x, y, [a, 0xc4, 0xb2, 0x8a]);
                    }
                    // Two bold vertical pause bars
                    let in_y = (9.5..=21.5).contains(&yf);
                    let in_left_bar = (10.2..=13.8).contains(&xf) && in_y;
                    let in_right_bar = (17.2..=20.8).contains(&xf) && in_y;
                    if in_left_bar || in_right_bar {
                        put_argb(&mut data, W, x, y, [255, 0xdc, 0xd7, 0xba]);
                    }
                }
            }
        }
        TrayPlaybackState::Buffering | TrayPlaybackState::Playing => {
            let is_buffering = state == TrayPlaybackState::Buffering;
            let level = volume_wave_level(volume, muted);
            let vol_norm = (volume.clamp(0.0, 100.0) / 100.0) as f32;
            let cx = 15.5f32;
            let cy = 24.5f32;

            for y in 0..W {
                for x in 0..W {
                    let xf = x as f32;
                    let yf = y as f32;
                    let dx = xf - cx;
                    let dy = yf - cy;
                    let dist = (dx * dx + dy * dy).sqrt();

                    // Center emitter dot
                    if dist <= 3.4 {
                        let fade = (3.4 - dist).clamp(0.0, 1.0);
                        let (r, g, b, a_scale) = if is_buffering {
                            (0xc4, 0xb2, 0x8a, 1.0)
                        } else if level == 0 {
                            (0x72, 0x71, 0x69, 0.7)
                        } else {
                            (0x87, 0xa9, 0x87, 1.0)
                        };
                        let a = (fade * a_scale * 255.0).round() as u8;
                        put_argb(&mut data, W, x, y, [a, r, g, b]);
                    } else if dy <= -0.5 && dx.abs() <= (-dy) * 1.48 {
                        let bands: [(u8, f32, [u8; 3]); 3] = [
                            (1, 7.2, [0x87, 0xa9, 0x87]),
                            (2, 12.4, [0x8e, 0xa4, 0xa2]),
                            (3, 17.6, [0xa3, 0xc4, 0xd4]),
                        ];
                        for (req_lvl, radius, lit_rgb) in bands {
                            let ring_d = (dist - radius).abs();
                            if ring_d < 1.35 {
                                let aa = (1.0 - ring_d / 1.35).clamp(0.0, 1.0);
                                let dash_skip =
                                    is_buffering && ((xf + yf).round() as i32).rem_euclid(5) < 2;
                                if is_buffering {
                                    if !dash_skip {
                                        let a = (aa * 235.0).round() as u8;
                                        put_argb(&mut data, W, x, y, [a, 0xc4, 0xb2, 0x8a]);
                                    }
                                } else if level >= req_lvl {
                                    let emphasis = if req_lvl == 3 {
                                        0.78 + 0.22 * vol_norm
                                    } else {
                                        1.0
                                    };
                                    let a = (aa * emphasis * 255.0).round() as u8;
                                    put_argb(
                                        &mut data,
                                        W,
                                        x,
                                        y,
                                        [a, lit_rgb[0], lit_rgb[1], lit_rgb[2]],
                                    );
                                } else {
                                    // Unlit wave ghost so full arc silhouette remains visible
                                    let a = (aa * 56.0).round() as u8;
                                    put_argb(&mut data, W, x, y, [a, 0x72, 0x71, 0x69]);
                                }
                                break;
                            }
                        }
                    }

                    // Bold mute slash when muted / 0% volume
                    if !is_buffering && level == 0 {
                        let slash_d = dist_to_segment(xf, yf, 6.0, 6.0, 25.0, 25.0);
                        if slash_d < 1.55 {
                            let a = ((1.0 - slash_d / 1.55) * 250.0).round() as u8;
                            put_argb(&mut data, W, x, y, [a, 0xc4, 0x74, 0x6e]);
                        }
                    }
                }
            }
        }
    }

    ksni::Icon {
        width: W,
        height: W,
        data,
    }
}

struct VibesTray {
    snap: TraySnapshot,
    tx: async_channel::Sender<TrayAction>,
}

impl VibesTray {
    fn send(&self, action: TrayAction) {
        let _ = self.tx.try_send(action);
    }

    fn action_item(label: impl Into<String>, icon_name: &str, action: TrayAction) -> MenuItem<Self> {
        StandardItem {
            label: label.into(),
            icon_name: icon_name.to_string(),
            activate: Box::new(move |this: &mut Self| this.send(action.clone())),
            ..Default::default()
        }
        .into()
    }
}

impl Tray for VibesTray {
    fn id(&self) -> String {
        "vibes".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn title(&self) -> String {
        if self.snap.subtitle.is_empty() {
            self.snap.title.clone()
        } else {
            format!("{} — {}", self.snap.title, self.snap.subtitle)
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let q_vol = ((self.snap.volume / 5.0).round() * 5.0).clamp(0.0, 100.0);
        vec![render_tray_icon(self.snap.state, q_vol, self.snap.muted)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let status_prefix = match self.snap.state {
            TrayPlaybackState::Playing => "Playing",
            TrayPlaybackState::Paused => "Paused",
            TrayPlaybackState::Buffering => "Buffering",
            TrayPlaybackState::Idle => "Idle",
        };
        let vol_str = if self.snap.muted {
            "Muted".to_string()
        } else {
            format!("Vol {:.0}%", self.snap.volume.clamp(0.0, 100.0))
        };
        ksni::ToolTip {
            title: format!("Vibes · {status_prefix} · {vol_str}"),
            description: if self.snap.subtitle.is_empty() {
                format!(
                    "{}\nLeft-click: Play/Pause · Middle-click: Window · Scroll: Volume",
                    self.snap.title
                )
            } else {
                format!(
                    "{}\n{}\nLeft-click: Play/Pause · Middle-click: Window · Scroll: Volume",
                    self.snap.title, self.snap.subtitle
                )
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::TogglePlay);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::ToggleWindow);
    }

    fn scroll(&mut self, delta: i32, orientation: ksni::Orientation) {
        if delta == 0 {
            return;
        }
        let step = if delta > 0 { 5.0 } else { -5.0 };
        let signed = match orientation {
            ksni::Orientation::Vertical => step,
            ksni::Orientation::Horizontal => step,
        };
        self.send(TrayAction::AdjustVolume(signed));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = Vec::new();

        let header_text = if self.snap.subtitle.is_empty() {
            self.snap.title.clone()
        } else {
            format!("{} · {}", self.snap.title, self.snap.subtitle)
        };
        items.push(
            StandardItem {
                label: escape_menu_label(&header_text),
                enabled: false,
                icon_name: "audio-headphones".to_string(),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);

        let play_label = match self.snap.state {
            TrayPlaybackState::Playing | TrayPlaybackState::Buffering => "Pause",
            TrayPlaybackState::Paused | TrayPlaybackState::Idle => "Play",
        };
        let play_icon = match self.snap.state {
            TrayPlaybackState::Playing | TrayPlaybackState::Buffering => {
                "media-playback-pause"
            }
            TrayPlaybackState::Paused | TrayPlaybackState::Idle => "media-playback-start",
        };
        items.push(Self::action_item(
            play_label,
            play_icon,
            TrayAction::TogglePlay,
        ));

        let win_label = if self.snap.window_open {
            "Hide Window to Tray"
        } else {
            "Show Window"
        };
        items.push(Self::action_item(win_label, "window", TrayAction::ToggleWindow));

        items.push(MenuItem::Separator);

        if self.snap.can_save_song {
            let save_label = if self.snap.saving_active {
                "Saving Song…".to_string()
            } else {
                "Save Current Song to Music".to_string()
            };
            items.push(
                StandardItem {
                    label: save_label,
                    enabled: !self.snap.saving_active,
                    icon_name: "document-save".to_string(),
                    activate: Box::new(|this: &mut Self| this.send(TrayAction::SaveCurrentSong)),
                    ..Default::default()
                }
                .into(),
            );
        }

        if self.snap.can_copy_track {
            items.push(Self::action_item(
                "Copy Track Title",
                "edit-copy",
                TrayAction::CopyTrackName,
            ));
        }

        if !self.snap.favorites.is_empty() {
            let fav_items: Vec<MenuItem<Self>> = self
                .snap
                .favorites
                .iter()
                .take(15)
                .enumerate()
                .map(|(idx, (uuid, name))| {
                    let numbered = if idx < 9 {
                        format!("[{}] {}", idx + 1, escape_menu_label(name))
                    } else {
                        escape_menu_label(name)
                    };
                    Self::action_item(
                        numbered,
                        "emblem-favorite",
                        TrayAction::PlayFavorite(uuid.clone()),
                    )
                })
                .collect();

            items.push(
                SubMenu {
                    label: "Favorites".to_string(),
                    icon_name: "starred".to_string(),
                    submenu: fav_items,
                    ..Default::default()
                }
                .into(),
            );
        }

        let sleep_label = match self.snap.sleep_remaining_mins {
            Some(m) => format!("Sleep Timer ({m}m left)"),
            None => "Sleep Timer".to_string(),
        };
        items.push(
            SubMenu {
                label: sleep_label,
                icon_name: "appointment-soon".to_string(),
                submenu: vec![
                    Self::action_item("Off", "", TrayAction::SetSleepTimer(None)),
                    Self::action_item("15 minutes", "", TrayAction::SetSleepTimer(Some(15))),
                    Self::action_item("30 minutes", "", TrayAction::SetSleepTimer(Some(30))),
                    Self::action_item("60 minutes", "", TrayAction::SetSleepTimer(Some(60))),
                ],
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);
        items.push(Self::action_item(
            "Quit Vibes",
            "application-exit",
            TrayAction::Quit,
        ));

        items
    }
}

pub struct TrayController {
    handle: Mutex<Option<Handle<VibesTray>>>,
}

impl TrayController {
    pub fn spawn(tx: async_channel::Sender<TrayAction>) -> Self {
        let tray = VibesTray {
            snap: TraySnapshot::default(),
            tx,
        };
        let handle = match tray.assume_sni_available(true).spawn() {
            Ok(h) => Some(h),
            Err(err) => {
                eprintln!("Vibes tray warning: {err}");
                None
            }
        };
        Self {
            handle: Mutex::new(handle),
        }
    }

    pub fn sync(&self, snap: TraySnapshot) {
        if let Some(ref handle) = *self.handle.lock().unwrap() {
            handle.update(move |tray| {
                tray.snap = snap;
            });
        }
    }

    pub fn shutdown(&self) {
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.shutdown().wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit_pixel_count(icon: &ksni::Icon) -> usize {
        icon.data.chunks_exact(4).filter(|px| px[0] > 140).count()
    }

    #[test]
    fn escapes_menu_underscores_and_renders_distinct_volume_modulated_icons() {
        assert_eq!(
            escape_menu_label("Lofi_Girl - Track_01"),
            "Lofi__Girl - Track__01"
        );
        assert_eq!(volume_wave_level(0.0, false), 0);
        assert_eq!(volume_wave_level(50.0, true), 0);
        assert_eq!(volume_wave_level(20.0, false), 1);
        assert_eq!(volume_wave_level(55.0, false), 2);
        assert_eq!(volume_wave_level(90.0, false), 3);

        let idle = render_tray_icon(TrayPlaybackState::Idle, 80.0, false);
        let paused = render_tray_icon(TrayPlaybackState::Paused, 80.0, false);
        let buffering = render_tray_icon(TrayPlaybackState::Buffering, 80.0, false);
        let low = render_tray_icon(TrayPlaybackState::Playing, 20.0, false);
        let mid = render_tray_icon(TrayPlaybackState::Playing, 55.0, false);
        let high = render_tray_icon(TrayPlaybackState::Playing, 100.0, false);
        let muted = render_tray_icon(TrayPlaybackState::Playing, 80.0, true);

        for icon in [&idle, &paused, &buffering, &low, &mid, &high, &muted] {
            assert_eq!(icon.width, 32);
            assert_eq!(icon.height, 32);
            assert_eq!(icon.data.len(), 32 * 32 * 4);
        }

        assert_ne!(idle.data, paused.data);
        assert_ne!(paused.data, high.data);
        assert_ne!(high.data, muted.data);
        assert!(lit_pixel_count(&low) < lit_pixel_count(&mid));
        assert!(lit_pixel_count(&mid) < lit_pixel_count(&high));
    }
}
