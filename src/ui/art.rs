use crate::theme::{Palette, hsla, rgb_with_alpha};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Entity, ImageCache, ImageCacheError, IntoElement, ObjectFit,
    ParentElement, RenderImage, Resource, RetainAllImageCache, Styled, StyledImage as _, Window,
    div, img, px,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

pub struct LruImageCache {
    inner: Entity<RetainAllImageCache>,
    order: VecDeque<Resource>,
    max_entries: usize,
}

impl LruImageCache {
    pub fn new(max_entries: usize, cx: &mut App) -> Entity<Self> {
        let inner = RetainAllImageCache::new(cx);
        cx.new(|_| Self {
            inner,
            order: VecDeque::with_capacity(max_entries + 1),
            max_entries: max_entries.max(1),
        })
    }
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        if let Some(pos) = self.order.iter().position(|r| r == resource) {
            self.order.remove(pos);
        }
        self.order.push_back(resource.clone());

        let result = self
            .inner
            .update(cx, |cache, cx| cache.load(resource, window, cx));

        while self.order.len() > self.max_entries {
            if let Some(oldest) = self.order.pop_front() {
                self.inner.update(cx, |cache, cx| {
                    cache.remove(&oldest, window, cx);
                });
            }
        }

        result
    }
}

pub fn tint_for_seed(seed: &str, pal: &Palette) -> u32 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in seed.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let choices = [
        pal.accent,
        pal.aqua,
        pal.violet,
        pal.green,
        pal.orange,
        pal.yellow,
    ];
    choices[(hash as usize) % choices.len()]
}

fn render_fallback_inner(
    w: f32,
    h: f32,
    radius: f32,
    icon: IconName,
    tint: u32,
) -> AnyElement {
    let min_side = w.min(h);
    let styled_icon = if min_side <= 24.0 {
        Icon::new(icon).xsmall()
    } else if min_side <= 42.0 {
        Icon::new(icon).small()
    } else if min_side <= 64.0 {
        Icon::new(icon)
    } else {
        Icon::new(icon).size(px((min_side * 0.28).round().clamp(24.0, 44.0)))
    };

    div()
        .size_full()
        .rounded(px(radius))
        .bg(rgb_with_alpha(tint, 0x2a))
        .border_1()
        .border_color(rgb_with_alpha(tint, 0x60))
        .flex()
        .items_center()
        .justify_center()
        .child(styled_icon.text_color(hsla(tint)))
        .into_any_element()
}

pub fn render_art_rect(
    path: Option<&PathBuf>,
    w: f32,
    h: f32,
    radius: f32,
    fallback_icon: IconName,
    seed: &str,
    active: bool,
    pal: &Palette,
) -> impl IntoElement {
    let tint = tint_for_seed(seed, pal);
    let container = div()
        .w(px(w))
        .h(px(h))
        .flex_shrink_0()
        .rounded(px(radius))
        .overflow_hidden()
        .bg(hsla(pal.bg_btn))
        .border_1()
        .border_color(hsla(if active {
            pal.accent
        } else {
            pal.border_subtle
        }));

    if let Some(p) = path {
        container.child(
            img(p.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_loading(move || render_fallback_inner(w, h, radius, fallback_icon, tint))
                .with_fallback(move || render_fallback_inner(w, h, radius, fallback_icon, tint)),
        )
    } else {
        container.child(render_fallback_inner(w, h, radius, fallback_icon, tint))
    }
}

pub fn render_art_tile(
    path: Option<&PathBuf>,
    size: f32,
    radius: f32,
    fallback_icon: IconName,
    seed: &str,
    active: bool,
    pal: &Palette,
) -> impl IntoElement {
    render_art_rect(path, size, size, radius, fallback_icon, seed, active, pal)
}
