use ksni::Tray;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
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
    pub lyrics_overlay_open: bool,
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
            lyrics_overlay_open: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TrayAction {
    TogglePlay,
    ToggleWindow,
    ToggleLyricsOverlay,
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

/// Maps `0..=100` volume + mute state to a normalized `0.0..=1.0` clockwise arc fraction.
pub fn volume_arc_fraction(volume: f64, muted: bool) -> f32 {
    if muted || volume <= 0.5 {
        0.0
    } else {
        (volume.clamp(0.0, 100.0) / 100.0) as f32
    }
}

fn alpha_byte(cov: f32, max_alpha: u8) -> u8 {
    (cov * max_alpha as f32).round().clamp(0.0, 255.0) as u8
}

fn blend_argb(data: &mut [u8], width: i32, x: i32, y: i32, argb: [u8; 4]) {
    if argb[0] == 0 || x < 0 || y < 0 || x >= width || y >= width {
        return;
    }
    let idx = ((y * width + x) * 4) as usize;
    let sa = argb[0] as f32 / 255.0;
    let da = data[idx] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    let inv_sa = da * (1.0 - sa);
    for c in 0..3 {
        let src = argb[c + 1] as f32;
        let dst = data[idx + c + 1] as f32;
        data[idx + c + 1] = ((src * sa + dst * inv_sa) / out_a)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    data[idx] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn disc_coverage(dist: f32, radius: f32) -> f32 {
    (radius + 0.65 - dist).clamp(0.0, 1.0)
}

fn ring_coverage(dist: f32, radius: f32, half_width: f32) -> f32 {
    (half_width + 0.65 - (dist - radius).abs()).clamp(0.0, 1.0)
}

fn arc_coverage(dx: f32, dy: f32, dist: f32, radius: f32, half_width: f32, sweep: f32) -> f32 {
    if sweep <= 0.001 {
        return 0.0;
    }
    let d_center = if sweep >= std::f32::consts::TAU - 0.01 {
        (dist - radius).abs()
    } else {
        let angle_cw = dx.atan2(-dy).rem_euclid(std::f32::consts::TAU);
        if angle_cw <= sweep {
            (dist - radius).abs()
        } else {
            let sx = 0.0;
            let sy = -radius;
            let ex = radius * sweep.sin();
            let ey = -radius * sweep.cos();
            let d_start = ((dx - sx) * (dx - sx) + (dy - sy) * (dy - sy)).sqrt();
            let d_end = ((dx - ex) * (dx - ex) + (dy - ey) * (dy - ey)).sqrt();
            d_start.min(d_end)
        }
    };
    (half_width + 0.65 - d_center).clamp(0.0, 1.0)
}

fn rounded_rect_coverage(
    px: f32,
    py: f32,
    cx: f32,
    cy: f32,
    half_w: f32,
    half_h: f32,
    radius: f32,
) -> f32 {
    let qx = (px - cx).abs() - (half_w - radius);
    let qy = (py - cy).abs() - (half_h - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    let sdf = outside + inside - radius;
    (0.65 - sdf).clamp(0.0, 1.0)
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

/// Generates a 32x32 ARGB32 (network byte order) circular vinyl disc + volume ring tray icon.
pub fn render_tray_icon(state: TrayPlaybackState, volume: f64, muted: bool) -> ksni::Icon {
    const W: i32 = 32;
    const CX: f32 = 15.5;
    const CY: f32 = 15.5;
    const R_RING: f32 = 12.8;
    const RING_HALF: f32 = 1.65;
    const R_DISC: f32 = 8.6;

    let mut data = vec![0u8; (W * W * 4) as usize];
    let vol_frac = volume_arc_fraction(volume, muted);

    for y in 0..W {
        for x in 0..W {
            let xf = x as f32;
            let yf = y as f32;
            let dx = xf - CX;
            let dy = yf - CY;
            let dist = (dx * dx + dy * dy).sqrt();
            let track_cov = ring_coverage(dist, R_RING, RING_HALF);

            match state {
                TrayPlaybackState::Idle => {
                    if track_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(track_cov, 145), 0x8a, 0x90, 0x9e],
                        );
                    }
                    let disc_cov = disc_coverage(dist, R_DISC);
                    if disc_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(disc_cov, 155), 0x2a, 0x2d, 0x36],
                        );
                    }
                    let rim_cov = ring_coverage(dist, 8.1, 0.55);
                    if rim_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(rim_cov, 115), 0x58, 0x5e, 0x6e],
                        );
                    }
                    let hub_cov = disc_coverage(dist, 2.9);
                    if hub_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(hub_cov, 190), 0x9e, 0x6b, 0x6b],
                        );
                    }
                    let hole_cov = disc_coverage(dist, 1.2);
                    if hole_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(hole_cov, 210), 0x1c, 0x1e, 0x24],
                        );
                    }
                }
                TrayPlaybackState::Paused => {
                    if track_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(track_cov, 130), 0x84, 0x8b, 0x99],
                        );
                    }
                    if vol_frac > 0.0 {
                        let sweep = vol_frac * std::f32::consts::TAU;
                        let arc_cov = arc_coverage(dx, dy, dist, R_RING, RING_HALF, sweep);
                        if arc_cov > 0.0 {
                            blend_argb(
                                &mut data,
                                W,
                                x,
                                y,
                                [alpha_byte(arc_cov, 200), 0xb8, 0xbf, 0xc9],
                            );
                        }
                    }
                    let disc_cov = disc_coverage(dist, 8.8);
                    if disc_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(disc_cov, 242), 0x23, 0x26, 0x2e],
                        );
                    }
                    let rim_cov = ring_coverage(dist, 8.2, 0.55);
                    if rim_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(rim_cov, 135), 0x55, 0x5b, 0x6b],
                        );
                    }
                    let left_bar = rounded_rect_coverage(xf, yf, 12.55, 15.5, 1.35, 4.9, 0.9);
                    let right_bar = rounded_rect_coverage(xf, yf, 18.45, 15.5, 1.35, 4.9, 0.9);
                    let bar_cov = left_bar.max(right_bar);
                    if bar_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(bar_cov, 255), 0xf2, 0xf4, 0xf8],
                        );
                    }
                }
                TrayPlaybackState::Buffering => {
                    if track_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(track_cov, 110), 0x7f, 0x87, 0x96],
                        );
                    }
                    let spinner_cov = arc_coverage(
                        dx,
                        dy,
                        dist,
                        R_RING,
                        RING_HALF,
                        250.0f32.to_radians(),
                    );
                    if spinner_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(spinner_cov, 255), 0xf6, 0xc4, 0x53],
                        );
                    }
                    let disc_cov = disc_coverage(dist, R_DISC);
                    if disc_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(disc_cov, 248), 0x24, 0x26, 0x2e],
                        );
                    }
                    let groove_cov = ring_coverage(dist, 5.6, 0.45);
                    if groove_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(groove_cov, 115), 0x5c, 0x62, 0x73],
                        );
                    }
                    let label_cov = disc_coverage(dist, 3.1);
                    if label_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(label_cov, 255), 0xf6, 0xc4, 0x53],
                        );
                    }
                    let hole_cov = disc_coverage(dist, 1.2);
                    if hole_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(hole_cov, 255), 0x1a, 0x1c, 0x22],
                        );
                    }
                }
                TrayPlaybackState::Playing => {
                    if track_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(track_cov, 115), 0x7f, 0x87, 0x96],
                        );
                    }
                    if vol_frac > 0.0 {
                        let sweep = vol_frac * std::f32::consts::TAU;
                        let arc_cov = arc_coverage(dx, dy, dist, R_RING, RING_HALF, sweep);
                        if arc_cov > 0.0 {
                            blend_argb(
                                &mut data,
                                W,
                                x,
                                y,
                                [alpha_byte(arc_cov, 255), 0xff, 0x52, 0x52],
                            );
                        }
                    }
                    let disc_cov = disc_coverage(dist, R_DISC);
                    if disc_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(disc_cov, 248), 0x24, 0x26, 0x2e],
                        );
                    }
                    let rim_cov = ring_coverage(dist, 8.1, 0.5);
                    if rim_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(rim_cov, 145), 0x4e, 0x54, 0x64],
                        );
                    }
                    let groove_cov = ring_coverage(dist, 5.6, 0.45);
                    if groove_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(groove_cov, 115), 0x63, 0x69, 0x7a],
                        );
                    }
                    let label_cov = disc_coverage(dist, 3.1);
                    if label_cov > 0.0 {
                        let (max_a, rgb) = if vol_frac > 0.0 {
                            (255u8, [0xe5, 0x39, 0x35])
                        } else {
                            (225u8, [0x72, 0x77, 0x82])
                        };
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(label_cov, max_a), rgb[0], rgb[1], rgb[2]],
                        );
                    }
                    let hole_cov = disc_coverage(dist, 1.2);
                    if hole_cov > 0.0 {
                        blend_argb(
                            &mut data,
                            W,
                            x,
                            y,
                            [alpha_byte(hole_cov, 255), 0x1a, 0x1c, 0x22],
                        );
                    }

                    // Bold mute slash when muted / 0% volume
                    if vol_frac == 0.0 {
                        let slash_d = dist_to_segment(xf, yf, 5.2, 5.2, 25.8, 25.8);
                        if slash_d < 2.4 {
                            let halo_cov = (2.4 - slash_d).clamp(0.0, 1.0);
                            blend_argb(
                                &mut data,
                                W,
                                x,
                                y,
                                [alpha_byte(halo_cov, 200), 0x18, 0x1a, 0x20],
                            );
                        }
                        if slash_d < 1.55 {
                            let slash_cov = (1.55 - slash_d).clamp(0.0, 1.0);
                            blend_argb(
                                &mut data,
                                W,
                                x,
                                y,
                                [alpha_byte(slash_cov, 255), 0xff, 0x52, 0x52],
                            );
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
        items.push(
            CheckmarkItem {
                label: "Lyrics Overlay".to_string(),
                checked: self.snap.lyrics_overlay_open,
                activate: Box::new(|this: &mut Self| this.send(TrayAction::ToggleLyricsOverlay)),
                ..Default::default()
            }
            .into(),
        );

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

    fn crimson_ring_pixels(icon: &ksni::Icon) -> usize {
        let mut count = 0;
        for y in 0..32i32 {
            for x in 0..32i32 {
                let dx = (x as f32) - 15.5;
                let dy = (y as f32) - 15.5;
                let dist = (dx * dx + dy * dy).sqrt();
                if !(10.5..=15.0).contains(&dist) {
                    continue;
                }
                let idx = ((y * 32 + x) * 4) as usize;
                let [a, r, g, _b] = [
                    icon.data[idx],
                    icon.data[idx + 1],
                    icon.data[idx + 2],
                    icon.data[idx + 3],
                ];
                if a > 160 && r > 200 && r > g.saturating_add(80) {
                    count += 1;
                }
            }
        }
        count
    }

    fn pixel_at(icon: &ksni::Icon, x: i32, y: i32) -> [u8; 4] {
        let idx = ((y * 32 + x) * 4) as usize;
        [
            icon.data[idx],
            icon.data[idx + 1],
            icon.data[idx + 2],
            icon.data[idx + 3],
        ]
    }

    fn maybe_write_preview(dir: &str, name: &str, icon: &ksni::Icon) {
        let _ = std::fs::create_dir_all(dir);
        let mut rgba = Vec::with_capacity(icon.data.len());
        for argb in icon.data.chunks_exact(4) {
            rgba.extend_from_slice(&[argb[1], argb[2], argb[3], argb[0]]);
        }
        if let Some(img) =
            image::RgbaImage::from_raw(icon.width as u32, icon.height as u32, rgba)
        {
            let _ = img.save(std::path::Path::new(dir).join(format!("{name}.png")));
        }
    }

    #[test]
    fn escapes_menu_underscores_and_renders_distinct_circular_volume_disc_icons() {
        assert_eq!(
            escape_menu_label("Lofi_Girl - Track_01"),
            "Lofi__Girl - Track__01"
        );
        assert_eq!(volume_arc_fraction(0.0, false), 0.0);
        assert_eq!(volume_arc_fraction(50.0, true), 0.0);
        assert!((volume_arc_fraction(25.0, false) - 0.25).abs() < 1e-6);
        assert!((volume_arc_fraction(50.0, false) - 0.50).abs() < 1e-6);
        assert!((volume_arc_fraction(100.0, false) - 1.0).abs() < 1e-6);
        assert!((volume_arc_fraction(150.0, false) - 1.0).abs() < 1e-6);

        let idle = render_tray_icon(TrayPlaybackState::Idle, 80.0, false);
        let paused = render_tray_icon(TrayPlaybackState::Paused, 80.0, false);
        let buffering = render_tray_icon(TrayPlaybackState::Buffering, 80.0, false);
        let v25 = render_tray_icon(TrayPlaybackState::Playing, 25.0, false);
        let v50 = render_tray_icon(TrayPlaybackState::Playing, 50.0, false);
        let v75 = render_tray_icon(TrayPlaybackState::Playing, 75.0, false);
        let v100 = render_tray_icon(TrayPlaybackState::Playing, 100.0, false);
        let muted = render_tray_icon(TrayPlaybackState::Playing, 80.0, true);

        let samples = [
            ("idle", &idle),
            ("paused", &paused),
            ("buffering", &buffering),
            ("playing_25", &v25),
            ("playing_50", &v50),
            ("playing_75", &v75),
            ("playing_100", &v100),
            ("muted", &muted),
        ];

        for (_, icon) in &samples {
            assert_eq!(icon.width, 32);
            assert_eq!(icon.height, 32);
            assert_eq!(icon.data.len(), 32 * 32 * 4);
        }

        if let Ok(dir) = std::env::var("VIBES_TRAY_PREVIEW_DIR") {
            for (name, icon) in &samples {
                maybe_write_preview(&dir, name, icon);
            }
        }

        assert_ne!(idle.data, paused.data);
        assert_ne!(paused.data, v100.data);
        assert_ne!(v100.data, muted.data);
        assert_ne!(buffering.data, v100.data);
        assert_ne!(idle.data, muted.data);

        assert!(crimson_ring_pixels(&muted) < crimson_ring_pixels(&v25));
        assert!(crimson_ring_pixels(&v25) < crimson_ring_pixels(&v50));
        assert!(crimson_ring_pixels(&v50) < crimson_ring_pixels(&v75));
        assert!(crimson_ring_pixels(&v75) < crimson_ring_pixels(&v100));

        // Center pixel: muted draws the red diagonal mute slash through the center; playing shows dark spindle hole.
        let center_muted = pixel_at(&muted, 15, 15);
        assert!(center_muted[0] > 200 && center_muted[1] > 180);
        let center_playing = pixel_at(&v100, 15, 15);
        assert!(center_playing[0] > 200 && center_playing[1] < 60);
    }
}
