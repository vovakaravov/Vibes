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

struct FrameAwareLru<K> {
    entries: VecDeque<(K, u64)>,
    max_entries: usize,
    frame_gen: u64,
}

impl<K: PartialEq> FrameAwareLru<K> {
    fn new(max_entries: usize) -> Self {
        let cap = max_entries.max(1);
        Self {
            entries: VecDeque::with_capacity(cap + 1),
            max_entries: cap,
            frame_gen: 0,
        }
    }

    fn begin_frame(&mut self) -> Vec<K> {
        let mut evicted = Vec::new();
        while self.entries.len() > self.max_entries
            && self
                .entries
                .front()
                .is_some_and(|(_, last_gen)| *last_gen < self.frame_gen)
        {
            if let Some((old, _)) = self.entries.pop_front() {
                evicted.push(old);
            }
        }
        self.frame_gen = self.frame_gen.saturating_add(1);
        evicted
    }

    fn touch(&mut self, key: K) -> Vec<K> {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos);
        }
        self.entries.push_back((key, self.frame_gen));

        // Protect entries touched in the current or immediately preceding frame so
        // over-capacity screens never self-evict between layout and paint passes.
        let cutoff = self.frame_gen.saturating_sub(1);
        let mut evicted = Vec::new();
        while self.entries.len() > self.max_entries
            && self
                .entries
                .front()
                .is_some_and(|(_, last_gen)| *last_gen < cutoff)
        {
            if let Some((old, _)) = self.entries.pop_front() {
                evicted.push(old);
            }
        }
        evicted
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

pub struct LruImageCache {
    inner: Entity<RetainAllImageCache>,
    lru: FrameAwareLru<Resource>,
}

impl LruImageCache {
    pub fn new(max_entries: usize, cx: &mut App) -> Entity<Self> {
        let inner = RetainAllImageCache::new(cx);
        cx.new(|_| Self {
            inner,
            lru: FrameAwareLru::new(max_entries),
        })
    }

    fn evict(&self, evicted: Vec<Resource>, window: &mut Window, cx: &mut App) {
        if evicted.is_empty() {
            return;
        }
        self.inner.update(cx, |cache, cx| {
            for oldest in evicted {
                cache.remove(&oldest, window, cx);
            }
        });
    }

    pub fn begin_frame(&mut self, window: &mut Window, cx: &mut App) {
        let evicted = self.lru.begin_frame();
        self.evict(evicted, window, cx);
    }
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let evicted = self.lru.touch(resource.clone());
        let result = self
            .inner
            .update(cx, |cache, cx| cache.load(resource, window, cx));
        self.evict(evicted, window, cx);
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

#[cfg(test)]
mod tests {
    use super::FrameAwareLru;

    #[test]
    fn same_frame_over_capacity_never_evicts_between_layout_and_paint() {
        let mut lru = FrameAwareLru::new(48);
        assert!(lru.begin_frame().is_empty());

        // Simulate request_layout touching 86 unique album covers in one frame.
        for id in 0..86 {
            assert!(
                lru.touch(id).is_empty(),
                "must not evict during request_layout for id {id}"
            );
        }
        assert_eq!(lru.len(), 86);

        // Simulate paint touching the same 86 covers in the same frame.
        for id in 0..86 {
            assert!(
                lru.touch(id).is_empty(),
                "must not evict during paint for id {id}"
            );
        }
        assert_eq!(lru.len(), 86);
    }

    #[test]
    fn repeated_frames_over_capacity_never_thrash() {
        let mut lru = FrameAwareLru::new(48);
        for frame in 0..5 {
            let evicted_begin = lru.begin_frame();
            assert!(
                evicted_begin.is_empty(),
                "unexpected eviction at start of frame {frame}: {evicted_begin:?}"
            );
            // Layout + paint passes
            for _pass in 0..2 {
                for id in 0..86 {
                    let evicted_touch = lru.touch(id);
                    assert!(
                        evicted_touch.is_empty(),
                        "unexpected eviction during frame {frame} touch {id}: {evicted_touch:?}"
                    );
                }
            }
        }
        assert_eq!(lru.len(), 86);
    }

    #[test]
    fn smaller_working_set_evicts_stale_lru_entries_on_subsequent_frames() {
        let mut lru = FrameAwareLru::new(3);

        // Frame 1 touches 0..5 (over capacity 3).
        assert!(lru.begin_frame().is_empty());
        for id in 0..5 {
            assert!(lru.touch(id).is_empty());
        }
        assert_eq!(lru.len(), 5);

        // Frame 2 touches only [4]; 0..3 still match `frame_gen` of Frame 1 at begin_frame
        // (since Frame 2 hasn't finished yet), then cutoff=frame_gen-1 evicts < frame 1 on touch?
        // After Frame 2 skips 0..3, Frame 3 begin_frame evicts stale entries [0, 1] down to 3.
        assert!(lru.begin_frame().is_empty());
        assert!(lru.touch(4).is_empty());

        let evicted = lru.begin_frame();
        assert_eq!(evicted, vec![0, 1]);
        assert_eq!(lru.len(), 3);
    }

    #[test]
    fn re_touching_entry_promotes_to_mru_and_preserves_it() {
        let mut lru = FrameAwareLru::new(2);

        // Frame 1 touches 0, 1, 2, then re-touches 0 before frame ends -> LRU order [1, 2, 0].
        assert!(lru.begin_frame().is_empty());
        assert!(lru.touch(0).is_empty());
        assert!(lru.touch(1).is_empty());
        assert!(lru.touch(2).is_empty());
        assert!(lru.touch(0).is_empty());

        // Frame 2 touches 3; frame_gen=2, cutoff=1 protects frame 1 & frame 2 entries.
        assert!(lru.begin_frame().is_empty());
        assert!(lru.touch(3).is_empty());

        // Frame 3 begin_frame evicts oldest untouched frame 1 entries [1, 2], preserving [0, 3].
        let evicted_f3 = lru.begin_frame();
        assert_eq!(evicted_f3, vec![1, 2]);
        assert_eq!(lru.len(), 2);

        // Touching 4 during Frame 3 (frame_gen=3, cutoff=2) evicts 0 (gen 1 < cutoff 2).
        let evicted_touch = lru.touch(4);
        assert_eq!(evicted_touch, vec![0]);
        assert_eq!(lru.len(), 2);
    }
}
