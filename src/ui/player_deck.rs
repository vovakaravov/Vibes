use super::{VibesWindow, art};
use crate::config::PlayTarget;
use crate::theme::{hsla, rgb_with_alpha};
use crate::youtube::format_duration;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Bounds, Context, FontWeight, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, ScrollDelta, ScrollEvent, Styled, canvas, div, fill, point,
    prelude::FluentBuilder as _, px, size,
};

fn paint_slider_track(
    bounds: Bounds<gpui_kit::Pixels>,
    frac: f32,
    track_col: u32,
    fill_col: u32,
    thumb_col: u32,
    window: &mut gpui_kit::Window,
) {
    let ox = f32::from(bounds.origin.x);
    let cy = f32::from(bounds.origin.y) + f32::from(bounds.size.height) * 0.5;
    let w = f32::from(bounds.size.width);
    if w <= 4.0 {
        return;
    }
    let frac = frac.clamp(0.0, 1.0);
    window.paint_quad(fill(
        Bounds {
            origin: point(px(ox), px(cy - 2.0)),
            size: size(px(w), px(4.)),
        },
        hsla(track_col),
    ));
    window.paint_quad(fill(
        Bounds {
            origin: point(px(ox), px(cy - 2.0)),
            size: size(px(w * frac), px(4.)),
        },
        hsla(fill_col),
    ));
    let knob_x = (ox + w * frac - 5.0).clamp(ox, ox + (w - 10.0).max(0.0));
    window.paint_quad(fill(
        Bounds {
            origin: point(px(knob_x), px(cy - 5.0)),
            size: size(px(10.), px(10.)),
        },
        hsla(thumb_col),
    ));
}

fn render_spectrum_canvas(
    win: &VibesWindow,
    cx: &Context<VibesWindow>,
    compact: bool,
) -> impl IntoElement {
    let core = win.core.read(cx);
    let bars = if core.visualizer.is_active() {
        core.visualizer.snapshot()
    } else {
        [0.05f32; 24]
    };
    let accent = core.palette.accent;
    let green = core.palette.green;
    let yellow = core.palette.yellow;
    let is_live_anim = core.playing && !core.paused;

    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let ox = f32::from(bounds.origin.x);
            let oy = f32::from(bounds.origin.y);
            let w = f32::from(bounds.size.width);
            let h = f32::from(bounds.size.height);
            if w <= 8.0 || h <= 6.0 {
                return;
            }
            let count = bars.len() as f32;
            let slot_w = w / count;
            let bar_w = (slot_w * 0.65).clamp(2.0, 5.5);

            for (i, &val) in bars.iter().enumerate() {
                let norm = if is_live_anim { val } else { 0.08 };
                let bh = (norm * (h - 2.0)).clamp(3.0, h - 2.0);
                let bx = ox + (i as f32) * slot_w + (slot_w - bar_w) * 0.5;
                let by = oy + h - bh;
                let col = if norm > 0.78 {
                    hsla(yellow)
                } else if norm > 0.45 {
                    hsla(green)
                } else if is_live_anim {
                    hsla(accent)
                } else {
                    rgb_with_alpha(accent, 0x55)
                };
                window.paint_quad(fill(
                    Bounds {
                        origin: point(px(bx), px(by)),
                        size: size(px(bar_w), px(bh)),
                    },
                    col,
                ));
            }
        },
    )
    .w(px(if compact { 82. } else { 116. }))
    .h(px(if compact { 24. } else { 32. }))
}

pub fn render_player_deck(win: &mut VibesWindow, cx: &mut Context<VibesWindow>) -> impl IntoElement {
    let core = win.core.read(cx);
    let pal = core.palette.clone();
    let compact = core.compact_mode;
    let playing = core.playing;
    let paused = core.paused;
    let buffering = core.buffering;
    let is_local = matches!(core.current_target, Some(PlayTarget::Local(_)));
    let is_seekable = matches!(
        core.current_target,
        Some(PlayTarget::Youtube(_) | PlayTarget::Local(_))
    );
    let pos = core.position_secs;
    let dur = core.duration_secs;
    let vol = core.volume;
    let muted = core.muted;
    let speed = core.speed;
    let sleep_mins = core.sleep_remaining_mins();
    let can_save = core.can_save_current_song();
    let saving = core.active_saving_count() > 0;
    let vis_enabled = core.config.visualizer_enabled;
    let lyrics_open = core.lyrics_open;

    let (primary_title, sub_title, fav_station, is_fav) = match &core.current_target {
        Some(PlayTarget::Radio(s)) => {
            let track = core
                .current_track_text()
                .unwrap_or_else(|| "Listening live · Waiting for track tag…".to_string());
            (track, s.name.clone(), Some(s.clone()), core.is_favorite(s))
        }
        Some(PlayTarget::Youtube(y)) => (y.title.clone(), y.channel.clone(), None, false),
        Some(PlayTarget::Local(t)) => {
            let sub = if t.album.is_empty() {
                t.artist.clone()
            } else {
                format!("{} · {}", t.artist, t.album)
            };
            (t.title.clone(), sub, None, false)
        }
        None => (
            "Select Radio, Local Library, or YouTube Audio".to_string(),
            "Ready".to_string(),
            None,
            false,
        ),
    };

    let (now_art, now_icon, now_seed) = match &core.current_target {
        Some(PlayTarget::Local(t)) => {
            let seed = if t.album.is_empty() {
                t.title.clone()
            } else {
                t.album.clone()
            };
            (t.cover_path.clone(), IconName::Music, seed)
        }
        Some(PlayTarget::Youtube(y)) => (
            core.art_path_for_youtube(&y.id).cloned(),
            IconName::Headphones,
            y.title.clone(),
        ),
        Some(PlayTarget::Radio(s)) => (
            core.art_path_for_url(&s.favicon).cloned(),
            IconName::Radio,
            s.name.clone(),
        ),
        None => (None, IconName::Radio, "Vibes".to_string()),
    };

    let mut telemetry = Vec::new();
    if is_local && playing {
        telemetry.push("LOCAL".to_string());
        if paused {
            telemetry.push("PAUSED".to_string());
        }
    } else if buffering && playing && !paused {
        telemetry.push("BUFFERING".to_string());
    } else if !is_seekable && playing {
        telemetry.push(if paused { "PAUSED" } else { "LIVE" }.to_string());
    }
    if let Some(ref c) = core.codec {
        telemetry.push(c.clone());
    }
    if let Some(br) = core.bitrate_kbps {
        telemetry.push(format!("{br}k"));
    }
    if let Some(sr) = core.sample_rate {
        telemetry.push(format!("{:.1}kHz", sr as f64 / 1000.0));
    }
    let telemetry_str = telemetry.join(" · ");

    let progress = match dur {
        Some(d) if d > 0.0 => (pos / d).clamp(0.0, 1.0) as f32,
        _ => 0.0,
    };

    let core_prev = win.core.clone();
    let core_rw = win.core.clone();
    let core_play = win.core.clone();
    let core_ff = win.core.clone();
    let core_next = win.core.clone();
    let core_stop = win.core.clone();
    let core_rec = win.core.clone();
    let core_save = win.core.clone();
    let core_fav = win.core.clone();
    let core_spd = win.core.clone();
    let core_slp = win.core.clone();
    let core_lyr = win.core.clone();
    let core_mute = win.core.clone();
    let core_vol_scroll = win.core.clone();
    let core_cmp = win.core.clone();

    let seek_geo = win.seek_geo.clone();
    let vol_geo = win.vol_geo.clone();

    let vol_icon = if muted || vol <= 0.5 {
        IconName::VolumeX
    } else if vol < 40.0 {
        IconName::Volume
    } else if vol < 75.0 {
        IconName::Volume1
    } else {
        IconName::Volume2
    };

    v_flex()
        .w_full()
        .flex_shrink_0()
        .bg(hsla(pal.bg_deep))
        .border_t_1()
        .border_color(hsla(pal.border))
        .px_4()
        .py(px(if compact { 8. } else { 10. }))
        .gap_2()
        .when(is_seekable, |deck| {
            deck.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(hsla(pal.fg_muted))
                            .child(format_duration(pos)),
                    )
                    .child(
                        div()
                            .id("deck-seek-bar")
                            .flex_1()
                            .h(px(16.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    this.dragging_seek = true;
                                    this.update_seek_from_x(f32::from(ev.position.x), cx);
                                }),
                            )
                            .child(
                                canvas(
                                    move |bounds, _, _| {
                                        seek_geo.set((
                                            f32::from(bounds.origin.x),
                                            f32::from(bounds.size.width),
                                        ));
                                        bounds
                                    },
                                    move |bounds, _, window, _| {
                                        paint_slider_track(
                                            bounds,
                                            progress,
                                            pal.bg_btn,
                                            pal.accent,
                                            pal.fg,
                                            window,
                                        );
                                    },
                                )
                                .size_full(),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(hsla(pal.fg_muted))
                            .child(dur.map(format_duration).unwrap_or_else(|| "--:--".to_string())),
                    ),
            )
        })
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .gap_3()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_2p5()
                        .child(art::render_art_tile(
                            now_art.as_ref(),
                            if compact { 36. } else { 52. },
                            7.,
                            now_icon,
                            &now_seed,
                            playing,
                            &pal,
                        ))
                        .when(vis_enabled, |r| r.child(render_spectrum_canvas(win, cx, compact)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(hsla(pal.fg))
                                        .truncate()
                                        .child(primary_title),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(hsla(pal.fg_muted))
                                                .truncate()
                                                .child(sub_title),
                                        )
                                        .when(!telemetry_str.is_empty(), |r| {
                                            r.child(
                                                div()
                                                    .px_1p5()
                                                    .rounded(px(4.))
                                                    .bg(hsla(pal.bg_panel))
                                                    .text_size(px(9.5))
                                                    .text_color(hsla(if is_local {
                                                        pal.green
                                                    } else if buffering {
                                                        pal.yellow
                                                    } else {
                                                        pal.aqua
                                                    }))
                                                    .child(telemetry_str),
                                            )
                                        }),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .flex_shrink_0()
                        .when(!compact, |r| {
                            r.child(
                                Button::new("deck-prev")
                                    .ghost()
                                    .small()
                                    .icon(IconName::SkipBack)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_prev.update(cx, |c, cx| c.play_previous_action(cx));
                                    })),
                            )
                        })
                        .when(is_seekable, |r| {
                            r.child(
                                Button::new("deck-rw15")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Rewind)
                                    .label("-15s")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_rw.update(cx, |c, cx| c.seek_relative(-15.0, cx));
                                    })),
                            )
                        })
                        .child(
                            Button::new("deck-play-pause")
                                .primary()
                                .small()
                                .icon(if playing && !paused {
                                    IconName::Pause
                                } else {
                                    IconName::Play
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_play.update(cx, |c, cx| c.toggle_play(cx));
                                })),
                        )
                        .when(is_seekable, |r| {
                            r.child(
                                Button::new("deck-ff30")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::FastForward)
                                    .label("+30s")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_ff.update(cx, |c, cx| c.seek_relative(30.0, cx));
                                    })),
                            )
                        })
                        .when(!compact, |r| {
                            r.child(
                                Button::new("deck-next")
                                    .ghost()
                                    .small()
                                    .icon(IconName::SkipForward)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_next.update(cx, |c, cx| c.play_next_in_queue(cx));
                                    })),
                            )
                            .child(
                                Button::new("deck-stop")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Square)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_stop.update(cx, |c, cx| c.stop_playback(cx));
                                    })),
                            )
                            .when(!is_seekable && playing, |row| {
                                row.child(
                                    Button::new("deck-reconnect")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::RefreshCw)
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            core_rec.update(cx, |c, cx| c.reconnect_stream(cx));
                                        })),
                                )
                            })
                        }),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .flex_shrink_0()
                        .when(can_save || saving, |r| {
                            r.child(
                                Button::new("deck-save-song")
                                    .with_variant(if saving {
                                        ButtonVariant::Secondary
                                    } else {
                                        ButtonVariant::Success
                                    })
                                    .small()
                                    .icon(IconName::Download)
                                    .when(!compact, |b| {
                                        b.label(if saving { "Saving…" } else { "Save Song" })
                                    })
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_save.update(cx, |c, cx| c.save_current_song(cx));
                                    })),
                            )
                        })
                        .when_some(fav_station, |r, st| {
                            r.child(
                                Button::new("deck-star")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Star).xsmall().text_color(hsla(
                                        if is_fav { pal.yellow } else { pal.fg_muted },
                                    )))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let s = st.clone();
                                        core_fav.update(cx, |c, cx| c.toggle_favorite_station(s, cx));
                                    })),
                            )
                        })
                        .when(is_seekable && !compact, |r| {
                            r.child(
                                Button::new("deck-speed")
                                    .ghost()
                                    .xsmall()
                                    .label(format!("{speed:.2}x"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_spd.update(cx, |c, cx| c.cycle_speed(cx));
                                    })),
                            )
                        })
                        .when(!compact, |r| {
                            r.child(
                                Button::new("deck-lyrics")
                                    .with_variant(if lyrics_open {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Ghost
                                    })
                                    .xsmall()
                                    .icon(IconName::Mic)
                                    .label("Lyrics")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_lyr.update(cx, |c, cx| c.toggle_lyrics_panel(cx));
                                    })),
                            )
                            .child(
                                Button::new("deck-sleep")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Moon)
                                    .when_some(sleep_mins, |b, m| b.label(format!("{m}m")))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_slp.update(cx, |c, cx| c.cycle_sleep_timer(cx));
                                    })),
                            )
                        })
                        .child(
                            h_flex()
                                .id("deck-vol-group")
                                .gap_1p5()
                                .items_center()
                                .on_scroll(cx.listener(move |_, ev: &ScrollEvent, _, cx| {
                                    let dy = match ev.delta {
                                        ScrollDelta::Lines(p) => p.y,
                                        ScrollDelta::Pixels(p) => f32::from(p.y),
                                    };
                                    if dy.abs() > 0.001 {
                                        cx.stop_propagation();
                                        core_vol_scroll.update(cx, |c, cx| {
                                            c.step_volume(dy > 0.0, cx);
                                        });
                                    }
                                }))
                                .child(
                                    Button::new("deck-mute")
                                        .ghost()
                                        .xsmall()
                                        .icon(vol_icon)
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            core_mute.update(cx, |c, cx| c.toggle_mute(cx));
                                        })),
                                )
                                .when(!compact, |r| {
                                    r.child(
                                        div()
                                            .id("deck-vol-slider")
                                            .w(px(88.))
                                            .h(px(20.))
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                                    this.dragging_vol = true;
                                                    this.update_volume_from_x(
                                                        f32::from(ev.position.x),
                                                        false,
                                                        cx,
                                                    );
                                                }),
                                            )
                                            .child(
                                                canvas(
                                                    move |bounds, _, _| {
                                                        vol_geo.set((
                                                            f32::from(bounds.origin.x),
                                                            f32::from(bounds.size.width),
                                                        ));
                                                        bounds
                                                    },
                                                    move |bounds, _, window, _| {
                                                        let frac = if muted {
                                                            0.0
                                                        } else {
                                                            (vol / 100.0).clamp(0.0, 1.0) as f32
                                                        };
                                                        paint_slider_track(
                                                            bounds,
                                                            frac,
                                                            pal.bg_btn,
                                                            pal.accent,
                                                            pal.fg,
                                                            window,
                                                        );
                                                    },
                                                )
                                                .size_full(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(30.))
                                            .text_size(px(10.5))
                                            .text_color(hsla(pal.fg_muted))
                                            .child(if muted {
                                                "Mute".to_string()
                                            } else {
                                                format!("{:.0}%", vol.clamp(0.0, 100.0))
                                            }),
                                    )
                                }),
                        )
                        .when(compact, |r| {
                            r.child(
                                Button::new("deck-expand")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Maximize2)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_cmp.update(cx, |c, cx| c.toggle_compact_mode(cx));
                                    })),
                            )
                        }),
                ),
        )
}
