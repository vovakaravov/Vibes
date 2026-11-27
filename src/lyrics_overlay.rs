use crate::artwork::trim_allocator;
use cosmic_text::{
    Attrs, Buffer, Color as CtColor, Ellipsize, EllipsizeHeightLimit, Family, FontSystem,
    Metrics, Shaping, SwashCache, Weight, Wrap,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry,
    wl_shm::Format as ShmFormat, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

static NEXT_SHM_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayFrame {
    pub header: String,
    pub primary: String,
    pub secondary: Option<String>,
    pub bg_rgb: u32,
    pub border_rgb: u32,
    pub accent_rgb: u32,
    pub fg_rgb: u32,
    pub muted_rgb: u32,
}

#[derive(Debug, Clone)]
pub struct RenderedOverlay {
    pub logical_w: i32,
    pub logical_h: i32,
    pub scale: i32,
    pub phys_w: i32,
    pub phys_h: i32,
    /// Premultiplied BGRA (`wl_shm::Format::Argb8888`) pixel buffer.
    pub bgra_premul: Vec<u8>,
}

enum OverlayCmd {
    Show(OverlayFrame),
    Hide,
    Disable,
    Quit,
}

pub struct LyricsOverlayController {
    tx: mpsc::Sender<OverlayCmd>,
    worker: Option<JoinHandle<()>>,
    last_state: Mutex<Option<Option<OverlayFrame>>>,
}

impl Default for LyricsOverlayController {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricsOverlayController {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("vibes-lyrics-overlay".into())
            .spawn(move || run_overlay_worker(rx))
            .ok();
        Self {
            tx,
            worker,
            last_state: Mutex::new(None),
        }
    }

    pub fn show(&self, frame: OverlayFrame) {
        let mut last = self.last_state.lock().unwrap();
        if last.as_ref() == Some(&Some(frame.clone())) {
            return;
        }
        *last = Some(Some(frame.clone()));
        let _ = self.tx.send(OverlayCmd::Show(frame));
    }

    pub fn hide(&self) {
        let mut last = self.last_state.lock().unwrap();
        if matches!(last.as_ref(), Some(None)) {
            return;
        }
        *last = Some(None);
        let _ = self.tx.send(OverlayCmd::Hide);
    }

    pub fn set_disabled(&self) {
        let mut last = self.last_state.lock().unwrap();
        if last.is_none() {
            return;
        }
        *last = None;
        let _ = self.tx.send(OverlayCmd::Disable);
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(OverlayCmd::Quit);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for LyricsOverlayController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn unpack_rgb(rgb: u32) -> [f32; 3] {
    [
        ((rgb >> 16) & 0xFF) as f32 / 255.0,
        ((rgb >> 8) & 0xFF) as f32 / 255.0,
        (rgb & 0xFF) as f32 / 255.0,
    ]
}

fn rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (half_w - r).max(0.0);
    let qy = (py - cy).abs() - (half_h - r).max(0.0);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

fn blend_premul_bgra(buf: &mut [u8], width: i32, height: i32, x: i32, y: i32, rgb: [f32; 3], sa: f32) {
    if sa <= 0.0 || x < 0 || y < 0 || x >= width || y >= height {
        return;
    }
    let idx = ((y * width + x) * 4) as usize;
    let inv_sa = (1.0 - sa).clamp(0.0, 1.0);
    let db = buf[idx] as f32 / 255.0;
    let dg = buf[idx + 1] as f32 / 255.0;
    let dr = buf[idx + 2] as f32 / 255.0;
    let da = buf[idx + 3] as f32 / 255.0;

    let out_a = (sa + da * inv_sa).clamp(0.0, 1.0);
    let max_c = (out_a * 255.0).round().clamp(0.0, 255.0);
    let out_b = ((rgb[2] * sa + db * inv_sa) * 255.0).round().clamp(0.0, max_c) as u8;
    let out_g = ((rgb[1] * sa + dg * inv_sa) * 255.0).round().clamp(0.0, max_c) as u8;
    let out_r = ((rgb[0] * sa + dr * inv_sa) * 255.0).round().clamp(0.0, max_c) as u8;

    buf[idx] = out_b;
    buf[idx + 1] = out_g;
    buf[idx + 2] = out_r;
    buf[idx + 3] = max_c as u8;
}

fn ct_color_from_rgb(rgb: u32, a: u8) -> CtColor {
    CtColor::rgba(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
        a,
    )
}

struct MeasuredBlock {
    buf: Buffer,
    max_w: f32,
    height: f32,
    color: CtColor,
}

fn layout_text_block(
    font_system: &mut FontSystem,
    text: &str,
    font_px: f32,
    line_px: f32,
    max_width_px: f32,
    max_lines: usize,
    weight: Weight,
    wrap: Wrap,
    color: CtColor,
) -> MeasuredBlock {
    let mut buf = Buffer::new(font_system, Metrics::new(font_px, line_px));
    buf.set_wrap(wrap);
    buf.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(max_lines)));
    buf.set_size(Some(max_width_px), None);
    let attrs = Attrs::new().family(Family::SansSerif).weight(weight);
    buf.set_text(text, &attrs, Shaping::Advanced, None);
    buf.shape_until_scroll(font_system, false);

    let mut max_w = 0.0f32;
    let mut run_count = 0usize;
    for run in buf.layout_runs() {
        max_w = max_w.max(run.line_w);
        run_count += 1;
    }
    let lines = run_count.clamp(1, max_lines) as f32;
    MeasuredBlock {
        buf,
        max_w,
        height: lines * line_px,
        color,
    }
}

fn draw_text_block(
    block: &mut MeasuredBlock,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    buf: &mut [u8],
    phys_w: i32,
    phys_h: i32,
    origin_x: i32,
    origin_y: i32,
) {
    let base_color = block.color;
    block
        .buf
        .draw(font_system, swash_cache, base_color, |gx, gy, w, h, col| {
            let sa = col.a() as f32 / 255.0;
            if sa <= 0.0 {
                return;
            }
            let rgb = [
                col.r() as f32 / 255.0,
                col.g() as f32 / 255.0,
                col.b() as f32 / 255.0,
            ];
            for dy in 0..(h as i32) {
                for dx in 0..(w as i32) {
                    blend_premul_bgra(
                        buf,
                        phys_w,
                        phys_h,
                        origin_x + gx + dx,
                        origin_y + gy + dy,
                        rgb,
                        sa,
                    );
                }
            }
        });
}

/// Renders a frosted dark translucent lyrics card into premultiplied BGRA pixels for Wayland `wl_shm`.
pub fn render_overlay_bitmap(
    frame: &OverlayFrame,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    scale: i32,
) -> RenderedOverlay {
    let scale = scale.clamp(1, 3);
    let s = scale as f32;

    let left_pad = 18.0f32;
    let right_pad = 14.0f32;
    let top_pad = 10.0f32;
    let bottom_pad = 10.0f32;
    let max_text_logical_w = 396.0f32;

    let mut hdr_block = layout_text_block(
        font_system,
        &frame.header,
        10.5 * s,
        14.0 * s,
        max_text_logical_w * s,
        1,
        Weight::SEMIBOLD,
        Wrap::None,
        ct_color_from_rgb(frame.muted_rgb, 240),
    );
    let mut pri_block = layout_text_block(
        font_system,
        &frame.primary,
        15.0 * s,
        20.0 * s,
        max_text_logical_w * s,
        3,
        Weight::BOLD,
        Wrap::Word,
        ct_color_from_rgb(frame.fg_rgb, 255),
    );
    let mut sec_block = frame
        .secondary
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|text| {
            layout_text_block(
                font_system,
                text,
                12.0 * s,
                16.0 * s,
                max_text_logical_w * s,
                2,
                Weight::NORMAL,
                Wrap::Word,
                ct_color_from_rgb(frame.muted_rgb, 225),
            )
        });

    let measured_logical_w = hdr_block
        .max_w
        .max(pri_block.max_w)
        .max(sec_block.as_ref().map_or(0.0, |b| b.max_w))
        / s;
    let logical_w = (measured_logical_w + left_pad + right_pad)
        .ceil()
        .clamp(240.0, 432.0) as i32;

    let gap1 = 4.0f32 * s;
    let gap2 = if sec_block.is_some() { 3.5f32 * s } else { 0.0 };
    let phys_content_h = top_pad * s
        + hdr_block.height
        + gap1
        + pri_block.height
        + sec_block.as_ref().map_or(0.0, |b| gap2 + b.height)
        + bottom_pad * s;
    let logical_h = (phys_content_h / s).ceil().max(54.0) as i32;

    let phys_w = logical_w * scale;
    let phys_h = logical_h * scale;
    let mut bgra = vec![0u8; (phys_w * phys_h * 4) as usize];

    let bg_rgb = unpack_rgb(frame.bg_rgb);
    let border_rgb = unpack_rgb(frame.border_rgb);
    let accent_rgb = unpack_rgb(frame.accent_rgb);

    let card_cx = (phys_w as f32) * 0.5;
    let card_cy = (phys_h as f32) * 0.5;
    let card_hw = (phys_w as f32) * 0.5 - 1.0 * s;
    let card_hh = (phys_h as f32) * 0.5 - 1.0 * s;
    let corner_r = 10.5 * s;

    let bar_cx = 9.0 * s;
    let bar_cy = card_cy;
    let bar_hw = 1.6 * s;
    let bar_hh = (card_hh - 10.0 * s).max(12.0 * s);
    let bar_r = 1.6 * s;

    for y in 0..phys_h {
        for x in 0..phys_w {
            let px = (x as f32) + 0.5;
            let py = (y as f32) + 0.5;
            let sdf = rounded_rect_sdf(px, py, card_cx, card_cy, card_hw, card_hh, corner_r);

            let bg_cov = (0.65 - sdf).clamp(0.0, 1.0);
            if bg_cov > 0.0 {
                blend_premul_bgra(&mut bgra, phys_w, phys_h, x, y, bg_rgb, bg_cov * (222.0 / 255.0));
            }

            let border_cov = (0.75 * s - (sdf + 0.4 * s).abs()).clamp(0.0, 1.0);
            if border_cov > 0.0 {
                blend_premul_bgra(
                    &mut bgra,
                    phys_w,
                    phys_h,
                    x,
                    y,
                    border_rgb,
                    border_cov * (195.0 / 255.0),
                );
            }

            let bar_sdf = rounded_rect_sdf(px, py, bar_cx, bar_cy, bar_hw, bar_hh, bar_r);
            let bar_cov = (0.65 - bar_sdf).clamp(0.0, 1.0);
            if bar_cov > 0.0 {
                blend_premul_bgra(
                    &mut bgra,
                    phys_w,
                    phys_h,
                    x,
                    y,
                    accent_rgb,
                    bar_cov * (245.0 / 255.0),
                );
            }
        }
    }

    let text_x = (left_pad * s).round() as i32;
    let hdr_y = (top_pad * s).round() as i32;
    let pri_y = hdr_y + hdr_block.height.round() as i32 + gap1.round() as i32;
    let sec_y = pri_y + pri_block.height.round() as i32 + gap2.round() as i32;

    draw_text_block(
        &mut hdr_block,
        font_system,
        swash_cache,
        &mut bgra,
        phys_w,
        phys_h,
        text_x,
        hdr_y,
    );
    draw_text_block(
        &mut pri_block,
        font_system,
        swash_cache,
        &mut bgra,
        phys_w,
        phys_h,
        text_x,
        pri_y,
    );
    if let Some(ref mut sec) = sec_block {
        draw_text_block(
            sec,
            font_system,
            swash_cache,
            &mut bgra,
            phys_w,
            phys_h,
            text_x,
            sec_y,
        );
    }

    RenderedOverlay {
        logical_w,
        logical_h,
        scale,
        phys_w,
        phys_h,
        bgra_premul: bgra,
    }
}

fn create_shm_buffer(
    shm: &WlShm,
    qh: &QueueHandle<WlState>,
    bitmap: &RenderedOverlay,
) -> Option<WlBuffer> {
    let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    let seq = NEXT_SHM_SEQ.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = dir.join(format!("vibes-overlay-{}-{seq}.shm", std::process::id()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .ok()?;
    let _ = fs::remove_file(&path);
    file.write_all(&bitmap.bgra_premul).ok()?;
    file.flush().ok()?;

    let pool = shm.create_pool(file.as_fd(), bitmap.bgra_premul.len() as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        bitmap.phys_w,
        bitmap.phys_h,
        bitmap.phys_w * 4,
        ShmFormat::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Some(buffer)
}

struct WlState {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    scale: i32,
    configured: bool,
    closed: bool,
    scale_dirty: bool,
}

impl WlState {
    fn new() -> Self {
        Self {
            compositor: None,
            shm: None,
            layer_shell: None,
            scale: 1,
            configured: false,
            closed: false,
            scale_dirty: false,
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for WlState {
    fn event(
        state: &mut Self,
        reg: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(reg.bind::<WlCompositor, _, _>(name, version.min(6), qh, ()));
                }
                "wl_shm" => {
                    state.shm = Some(reg.bind::<WlShm, _, _>(name, version.min(1), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell =
                        Some(reg.bind::<ZwlrLayerShellV1, _, _>(name, version.min(4), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_surface::Event::PreferredBufferScale { factor } = event
            && factor >= 1
            && state.scale != factor
        {
            state.scale = factor.clamp(1, 3);
            state.scale_dirty = true;
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for WlState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                layer_surface.ack_configure(serial);
                state.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.closed = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(WlState: ignore WlCompositor);
delegate_noop!(WlState: ignore WlShm);
delegate_noop!(WlState: ignore WlShmPool);
delegate_noop!(WlState: ignore WlBuffer);
delegate_noop!(WlState: ignore WlRegion);
delegate_noop!(WlState: ignore ZwlrLayerShellV1);

struct MappedSurface {
    surface: wl_surface::WlSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    active_buffer: Option<WlBuffer>,
    logical_size: (i32, i32),
}

impl MappedSurface {
    fn destroy(mut self, conn: &Connection) {
        if let Some(buf) = self.active_buffer.take() {
            buf.destroy();
        }
        self.layer_surface.destroy();
        self.surface.destroy();
        let _ = conn.flush();
    }
}

struct WaylandSession {
    conn: Connection,
    event_queue: EventQueue<WlState>,
    state: WlState,
    mapped: Option<MappedSurface>,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl WaylandSession {
    fn connect() -> Option<Self> {
        let conn = Connection::connect_to_env().ok()?;
        let display = conn.display();
        let mut event_queue: EventQueue<WlState> = conn.new_event_queue();
        let qh = event_queue.handle();
        let mut state = WlState::new();
        let _registry = display.get_registry(&qh, ());
        event_queue.roundtrip(&mut state).ok()?;

        if state.compositor.is_none() || state.shm.is_none() || state.layer_shell.is_none() {
            let msg = "Compositor lacks zwlr_layer_shell_v1 or wl_shm";
            crate::log::log_warn("overlay", msg);
            eprintln!("Vibes overlay warning: {msg}");
            return None;
        }

        Some(Self {
            conn,
            event_queue,
            state,
            mapped: None,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        })
    }

    fn present(&mut self, frame: &OverlayFrame) {
        let _ = self.event_queue.dispatch_pending(&mut self.state);
        if self.state.closed {
            self.unmap();
            self.state.closed = false;
        }

        let bitmap = render_overlay_bitmap(
            frame,
            &mut self.font_system,
            &mut self.swash_cache,
            self.state.scale,
        );

        let qh = self.event_queue.handle();
        if self.mapped.is_none() {
            let (Some(compositor), Some(layer_shell)) =
                (self.state.compositor.as_ref(), self.state.layer_shell.as_ref())
            else {
                return;
            };
            let surface = compositor.create_surface(&qh, ());
            let empty_region = compositor.create_region(&qh, ());
            surface.set_input_region(Some(&empty_region));
            empty_region.destroy();

            let layer_surface = layer_shell.get_layer_surface(
                &surface,
                None,
                Layer::Top,
                "vibes-lyrics-overlay".to_string(),
                &qh,
                (),
            );
            layer_surface.set_anchor(Anchor::Bottom | Anchor::Right);
            layer_surface.set_exclusive_zone(0);
            layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer_surface.set_margin(0, 14, 10, 0);
            layer_surface.set_size(bitmap.logical_w as u32, bitmap.logical_h as u32);

            self.state.configured = false;
            self.state.closed = false;
            surface.commit();
            if self.event_queue.roundtrip(&mut self.state).is_err() || !self.state.configured {
                layer_surface.destroy();
                surface.destroy();
                return;
            }

            self.mapped = Some(MappedSurface {
                surface,
                layer_surface,
                active_buffer: None,
                logical_size: (bitmap.logical_w, bitmap.logical_h),
            });
        }

        // If PreferredBufferScale updated on initial configure, re-render at the compositor's scale.
        let bitmap = if self.state.scale != bitmap.scale {
            self.state.scale_dirty = false;
            render_overlay_bitmap(
                frame,
                &mut self.font_system,
                &mut self.swash_cache,
                self.state.scale,
            )
        } else {
            self.state.scale_dirty = false;
            bitmap
        };

        let Some(shm) = self.state.shm.as_ref() else {
            return;
        };
        let Some(ref mut mapped) = self.mapped else {
            return;
        };

        if mapped.logical_size != (bitmap.logical_w, bitmap.logical_h) {
            mapped
                .layer_surface
                .set_size(bitmap.logical_w as u32, bitmap.logical_h as u32);
            mapped.logical_size = (bitmap.logical_w, bitmap.logical_h);
        }

        let Some(new_buf) = create_shm_buffer(shm, &qh, &bitmap) else {
            return;
        };
        if mapped.surface.version() >= 3 {
            mapped.surface.set_buffer_scale(bitmap.scale);
        }
        mapped.surface.attach(Some(&new_buf), 0, 0);
        if mapped.surface.version() >= 4 {
            mapped
                .surface
                .damage_buffer(0, 0, bitmap.phys_w, bitmap.phys_h);
        } else {
            mapped
                .surface
                .damage(0, 0, bitmap.logical_w, bitmap.logical_h);
        }
        mapped.surface.commit();
        let _ = self.conn.flush();

        if let Some(old) = mapped.active_buffer.replace(new_buf) {
            old.destroy();
        }
    }

    fn unmap(&mut self) {
        if let Some(mapped) = self.mapped.take() {
            mapped.destroy(&self.conn);
        }
    }

    fn poll_scale_or_close(&mut self, current_frame: &OverlayFrame) {
        let _ = self.conn.prepare_read().map(|guard| guard.read());
        let _ = self.event_queue.dispatch_pending(&mut self.state);
        if self.state.closed {
            self.unmap();
            self.state.closed = false;
        } else if self.state.scale_dirty && self.mapped.is_some() {
            self.present(current_frame);
        }
    }
}

fn run_overlay_worker(rx: mpsc::Receiver<OverlayCmd>) {
    let mut session: Option<WaylandSession> = None;
    let mut current_frame: Option<OverlayFrame> = None;
    let mut connection_failed = false;

    loop {
        let cmd_res = if session.as_ref().is_some_and(|s| s.mapped.is_some()) {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(c) => Some(c),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let (Some(s), Some(frame)) = (session.as_mut(), current_frame.as_ref()) {
                        s.poll_scale_or_close(frame);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            rx.recv().ok()
        };

        let Some(mut cmd) = cmd_res else {
            break;
        };

        // Coalesce burst updates queued in the channel so only the newest command renders.
        while let Ok(newer) = rx.try_recv() {
            cmd = newer;
            if matches!(cmd, OverlayCmd::Quit) {
                break;
            }
        }

        match cmd {
            OverlayCmd::Show(frame) => {
                current_frame = Some(frame);
                if session.is_none() && !connection_failed {
                    session = WaylandSession::connect();
                    if session.is_none() {
                        connection_failed = true;
                    }
                }
                if let (Some(s), Some(f)) = (session.as_mut(), current_frame.as_ref()) {
                    s.present(f);
                }
            }
            OverlayCmd::Hide => {
                current_frame = None;
                if let Some(s) = session.as_mut() {
                    s.unmap();
                }
            }
            OverlayCmd::Disable => {
                current_frame = None;
                connection_failed = false;
                if let Some(mut s) = session.take() {
                    s.unmap();
                }
                trim_allocator();
            }
            OverlayCmd::Quit => {
                if let Some(mut s) = session.take() {
                    s.unmap();
                }
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_premultiplied_bgra_overlay_card_with_transparent_corners() {
        let mut fs = FontSystem::new();
        let mut cache = SwashCache::new();
        let frame = OverlayFrame {
            header: "♪ Linkin Park — Papercut".to_string(),
            primary: "Why does it feel like night today? Somethin' in here's not right today"
                .to_string(),
            secondary: Some("Why am I so uptight today?".to_string()),
            bg_rgb: 0x181616,
            border_rgb: 0x393836,
            accent_rgb: 0x8ba4b0,
            fg_rgb: 0xc5c9c5,
            muted_rgb: 0x8a8980,
        };

        let r1 = render_overlay_bitmap(&frame, &mut fs, &mut cache, 1);
        assert!((240..=432).contains(&r1.logical_w));
        assert!(r1.logical_h >= 54);
        assert_eq!(r1.phys_w, r1.logical_w);
        assert_eq!(r1.phys_h, r1.logical_h);
        assert_eq!(r1.bgra_premul.len(), (r1.phys_w * r1.phys_h * 4) as usize);

        for px in r1.bgra_premul.chunks_exact(4) {
            let [b, g, r, a] = [px[0], px[1], px[2], px[3]];
            assert!(
                b <= a && g <= a && r <= a,
                "invalid premultiplied BGRA pixel: [{b},{g},{r},{a}]"
            );
        }

        // Outer corner must be transparent; center interior must be opaque/translucent with bright glyphs present.
        assert_eq!(r1.bgra_premul[3], 0);
        let center_idx = (((r1.phys_h / 2) * r1.phys_w + (r1.phys_w / 2)) * 4) as usize;
        assert!(r1.bgra_premul[center_idx + 3] > 200);
        let bright_pixels = r1
            .bgra_premul
            .chunks_exact(4)
            .filter(|px| px[2] > 150 && px[3] > 200)
            .count();
        assert!(bright_pixels > 40, "expected crisp rendered text glyphs");

        let r2 = render_overlay_bitmap(&frame, &mut fs, &mut cache, 2);
        assert_eq!(r2.scale, 2);
        assert_eq!(r2.phys_w, r2.logical_w * 2);
        assert_eq!(r2.phys_h, r2.logical_h * 2);

        if let Ok(out_path) = std::env::var("VIBES_OVERLAY_PREVIEW_PATH") {
            let mut straight_rgba = Vec::with_capacity(r1.bgra_premul.len());
            for px in r1.bgra_premul.chunks_exact(4) {
                let [b, g, r, a] = [px[0], px[1], px[2], px[3]];
                if a == 0 {
                    straight_rgba.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    let inv = 255.0 / (a as f32);
                    straight_rgba.push(((r as f32 * inv).round().clamp(0.0, 255.0)) as u8);
                    straight_rgba.push(((g as f32 * inv).round().clamp(0.0, 255.0)) as u8);
                    straight_rgba.push(((b as f32 * inv).round().clamp(0.0, 255.0)) as u8);
                    straight_rgba.push(a);
                }
            }
            if let Some(img) =
                image::RgbaImage::from_raw(r1.phys_w as u32, r1.phys_h as u32, straight_rgba)
            {
                if let Some(parent) = std::path::Path::new(&out_path).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = img.save(&out_path);
            }
        }
    }
}
