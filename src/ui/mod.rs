pub mod library_view;
pub mod player_deck;
pub mod radio_view;
pub mod sidebar;
pub mod youtube_view;

use crate::core::{NavSection, VibesCore};
use crate::theme::hsla;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, Render, Styled, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub fn register_keybindings(_cx: &mut App) {}

pub struct VibesWindow {
    pub core: Entity<VibesCore>,
    pub radio_search_input: Entity<InputState>,
    pub yt_search_input: Entity<InputState>,
    pub custom_name_input: Entity<InputState>,
    pub custom_url_input: Entity<InputState>,
    pub country_input: Entity<InputState>,
    pub music_dir_input: Entity<InputState>,
    pub show_custom_station: bool,
    pub seek_geo: Rc<Cell<(f32, f32)>>,
    pub vol_geo: Rc<Cell<(f32, f32)>>,
    pub dragging_seek: bool,
    pub dragging_vol: bool,
    focus_handle: FocusHandle,
    _subs: Vec<Subscription>,
}

impl Focusable for VibesWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl VibesWindow {
    pub fn new(
        core: Entity<VibesCore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let cfg = core.read(cx).config.clone();
        let initial_radio_q = core.read(cx).radio_query.clone();
        let initial_yt_q = core.read(cx).yt_query.clone();

        let radio_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search 50,000+ global radio stations (Enter)…")
                .default_value(initial_radio_q)
        });
        let yt_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search YouTube Music, mixes, podcasts, or paste a URL (Enter)…")
                .default_value(initial_yt_q)
        });
        let custom_name_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Station name (e.g. SomaFM Groove Salad)")
        });
        let custom_url_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Direct stream URL (https://…)")
        });
        let country_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("US")
                .default_value(cfg.country_code)
        });
        let music_dir_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("~/Music/Radio")
                .default_value(cfg.music_dir.display().to_string())
        });

        let mut subs = Vec::new();
        subs.push(cx.observe(&core, |_, _, cx| cx.notify()));

        let core_r = core.clone();
        subs.push(
            cx.subscribe_in(&radio_search_input, window, move |_, inp, ev, _, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    let q = inp.read(cx).value().to_string();
                    core_r.update(cx, |c, cx| c.set_radio_query(q, cx));
                }
            }),
        );

        let core_y = core.clone();
        subs.push(
            cx.subscribe_in(&yt_search_input, window, move |_, inp, ev, _, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    let q = inp.read(cx).value().to_string();
                    core_y.update(cx, |c, cx| c.search_youtube_query(q, cx));
                }
            }),
        );

        // Smooth 30fps visualizer refresh loop ONLY while this window lives & spectrum is active.
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let keep = weak
                    .update(cx, |win, cx| {
                        let (vis_active, open) = {
                            let core = win.core.read(cx);
                            (core.visualizer.is_active(), core.window_open)
                        };
                        if vis_active {
                            cx.notify();
                        }
                        open
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();

        let fh = cx.focus_handle();
        fh.focus(window, cx);

        Self {
            core,
            radio_search_input,
            yt_search_input,
            custom_name_input,
            custom_url_input,
            country_input,
            music_dir_input,
            show_custom_station: false,
            seek_geo: Rc::new(Cell::new((0.0, 200.0))),
            vol_geo: Rc::new(Cell::new((0.0, 88.0))),
            dragging_seek: false,
            dragging_vol: false,
            focus_handle: fh,
            _subs: subs,
        }
    }

    pub fn update_volume_from_x(&mut self, mouse_x: f32, persist: bool, cx: &mut Context<Self>) {
        let (ox, w) = self.vol_geo.get();
        if w > 8.0 {
            let frac = ((mouse_x - ox) / w).clamp(0.0, 1.0);
            self.core
                .update(cx, |c, cx| c.set_volume_live(frac as f64 * 100.0, persist, cx));
        }
    }

    pub fn update_seek_from_x(&mut self, mouse_x: f32, cx: &mut Context<Self>) {
        let (ox, w) = self.seek_geo.get();
        let dur = self.core.read(cx).duration_secs;
        if w > 8.0
            && let Some(d) = dur
        {
            let frac = ((mouse_x - ox) / w).clamp(0.0, 1.0);
            self.core
                .update(cx, |c, cx| c.seek_absolute(frac as f64 * d, cx));
        }
    }

    fn handle_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging_vol && !self.dragging_seek {
            return;
        }
        if !ev.dragging() {
            self.finish_drag(cx);
            return;
        }
        let mx = f32::from(ev.position.x);
        if self.dragging_vol {
            self.update_volume_from_x(mx, false, cx);
        } else if self.dragging_seek {
            self.update_seek_from_x(mx, cx);
        }
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        let was_vol = self.dragging_vol;
        self.dragging_vol = false;
        self.dragging_seek = false;
        if was_vol {
            self.core.read(cx).persist_config_now();
        }
    }

    fn any_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.radio_search_input.focus_handle(cx).is_focused(window)
            || self.yt_search_input.focus_handle(cx).is_focused(window)
            || self.custom_name_input.focus_handle(cx).is_focused(window)
            || self.custom_url_input.focus_handle(cx).is_focused(window)
            || self.country_input.focus_handle(cx).is_focused(window)
            || self.music_dir_input.focus_handle(cx).is_focused(window)
    }

    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control;

        if ctrl {
            match key {
                "s" => {
                    cx.stop_propagation();
                    self.core.update(cx, |c, cx| c.save_current_song(cx));
                    return;
                }
                "m" => {
                    cx.stop_propagation();
                    self.core.update(cx, |c, cx| c.toggle_compact_mode(cx));
                    return;
                }
                "q" => {
                    cx.stop_propagation();
                    self.core.update(cx, |c, cx| c.shutdown(cx));
                    return;
                }
                _ => {}
            }
        }

        if self.any_input_focused(window, cx) {
            return;
        }

        match key {
            "space" => {
                cx.stop_propagation();
                self.core.update(cx, |c, cx| c.toggle_play(cx));
            }
            "m" => {
                cx.stop_propagation();
                self.core.update(cx, |c, cx| c.toggle_mute(cx));
            }
            "left" => {
                cx.stop_propagation();
                self.core.update(cx, |c, cx| c.seek_relative(-5.0, cx));
            }
            "right" => {
                cx.stop_propagation();
                self.core.update(cx, |c, cx| c.seek_relative(5.0, cx));
            }
            k if k.len() == 1 && k.chars().all(|c| ('1'..='9').contains(&c)) => {
                let slot = k.parse::<usize>().unwrap_or(1);
                cx.stop_propagation();
                self.core.update(cx, |c, cx| c.play_favorite_preset(slot, cx));
            }
            _ => {}
        }
    }
}

impl Render for VibesWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (pal, compact, section, toast, mpv_err) = {
            let core = self.core.read(cx);
            (
                core.palette.clone(),
                core.compact_mode,
                core.section,
                core.toast.clone(),
                core.mpv_error.clone(),
            )
        };

        if compact {
            return v_flex()
                .id("vibes-compact-root")
                .track_focus(&self.focus_handle)
                .size_full()
                .justify_between()
                .bg(hsla(pal.bg_main))
                .text_color(hsla(pal.fg))
                .on_key_down(cx.listener(Self::handle_key_down))
                .on_mouse_move(cx.listener(Self::handle_mouse_move))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
                )
                .child(
                    h_flex()
                        .h(px(30.))
                        .px_3()
                        .justify_between()
                        .items_center()
                        .bg(hsla(pal.bg_deep))
                        .border_b_1()
                        .border_color(hsla(pal.border_subtle))
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(Icon::new(IconName::Radio).xsmall().text_color(hsla(pal.accent)))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Vibes · Mini Deck"),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("cmp-tray-btn")
                                        .ghost()
                                        .xsmall()
                                        .label("Tray")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.core.update(cx, |c, _| c.hide_to_tray());
                                        })),
                                ),
                        ),
                )
                .child(player_deck::render_player_deck(self, cx))
                .into_any_element();
        }

        let core_cmp = self.core.clone();
        let core_tray = self.core.clone();

        let header = h_flex()
            .h(px(38.))
            .w_full()
            .px_4()
            .justify_between()
            .items_center()
            .bg(hsla(pal.bg_deep))
            .border_b_1()
            .border_color(hsla(pal.border_subtle))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(IconName::Radio).small().text_color(hsla(pal.accent)))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(hsla(pal.fg))
                            .child("Vibes"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Hi-Fi Internet Radio & Audio Deck"),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when_some(mpv_err, |r, err| {
                        r.child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(hsla(pal.bg_panel))
                                .text_xs()
                                .text_color(hsla(pal.red))
                                .child(err),
                        )
                    })
                    .when_some(toast, |r, t| {
                        r.child(
                            div()
                                .px_2p5()
                                .py_0p5()
                                .rounded(px(5.))
                                .bg(hsla(pal.bg_panel))
                                .border_1()
                                .border_color(hsla(pal.border))
                                .text_xs()
                                .text_color(hsla(pal.green))
                                .child(t),
                        )
                    })
                    .child(
                        Button::new("hdr-compact-btn")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Minimize2)
                            .label("Mini Deck")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                core_cmp.update(cx, |c, cx| c.toggle_compact_mode(cx));
                            })),
                    )
                    .child(
                        Button::new("hdr-tray-btn")
                            .secondary()
                            .xsmall()
                            .label("Minimize to Tray")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                core_tray.update(cx, |c, _| c.hide_to_tray());
                            })),
                    ),
            );

        let main_stage = match section {
            NavSection::Radio => radio_view::render_radio_view(self, window, cx).into_any_element(),
            NavSection::Youtube => youtube_view::render_youtube_view(self, window, cx).into_any_element(),
            NavSection::Favorites => library_view::render_favorites_view(self, cx).into_any_element(),
            NavSection::SongLog => library_view::render_song_log_view(self, cx).into_any_element(),
            NavSection::Queue => library_view::render_queue_view(self, cx).into_any_element(),
            NavSection::Downloads => library_view::render_downloads_view(self, cx).into_any_element(),
            NavSection::Settings => library_view::render_settings_view(self, cx).into_any_element(),
        };

        v_flex()
            .id("vibes-root")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(hsla(pal.bg_main))
            .text_color(hsla(pal.fg))
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
            )
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(sidebar::render_sidebar(self, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(hsla(pal.bg_main))
                            .child(main_stage),
                    ),
            )
            .child(player_deck::render_player_deck(self, cx))
            .into_any_element()
    }
}
