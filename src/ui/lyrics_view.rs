use super::VibesWindow;
use crate::config::PlayTarget;
use crate::lyrics::{
    LyricsKind, LyricsOrigin, LyricsState, active_synced_line_index,
};
use crate::theme::{hsla, rgb_with_alpha};
use crate::youtube::format_duration;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, div, point, prelude::FluentBuilder as _, px,
};

pub fn render_lyrics_panel(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (
        pal,
        lyrics_state,
        lyrics_track_key,
        pos_secs,
        track_title,
        track_sub,
        is_local,
        is_seekable,
    ) = {
        let core = win.core.read(cx);
        let (title, sub, local, seekable) = match &core.current_target {
            Some(PlayTarget::Local(t)) => {
                let s = if t.album.trim().is_empty() {
                    t.artist.clone()
                } else {
                    format!("{} · {}", t.artist, t.album)
                };
                (t.title.clone(), s, true, true)
            }
            Some(PlayTarget::Youtube(y)) => (y.title.clone(), y.channel.clone(), false, true),
            Some(PlayTarget::Radio(s)) => {
                let tr = core
                    .current_track_text()
                    .unwrap_or_else(|| "Listening live · Waiting for track tag…".to_string());
                (tr, s.name.clone(), false, false)
            }
            None => (
                "No track playing".to_string(),
                "Select a station or track".to_string(),
                false,
                false,
            ),
        };
        (
            core.palette.clone(),
            core.lyrics_state.clone(),
            core.lyrics_track_key.clone(),
            core.position_secs,
            title,
            sub,
            local,
            seekable,
        )
    };

    if win.last_lyrics_key != lyrics_track_key {
        win.lyrics_scroll.set_offset(point(px(0.), px(0.)));
        win.last_lyrics_key = lyrics_track_key;
        win.last_scrolled_lyric_idx = None;
    }

    let badge_text: Option<&'static str> = match &lyrics_state {
        LyricsState::Ready(doc) => Some(match (doc.origin, &doc.kind) {
            (_, LyricsKind::Instrumental) => "Instrumental",
            (LyricsOrigin::Embedded, LyricsKind::Synced(_)) => "Embedded · Synced",
            (LyricsOrigin::Embedded, LyricsKind::Plain(_)) => "Embedded · Plain",
            (LyricsOrigin::Lrclib, LyricsKind::Synced(_)) => "LRCLIB · Synced",
            (LyricsOrigin::Lrclib, LyricsKind::Plain(_)) => "LRCLIB · Plain",
        }),
        _ => None,
    };

    let show_embed_btn = matches!(
        &lyrics_state,
        LyricsState::Ready(doc)
            if is_local
                && doc.origin == LyricsOrigin::Lrclib
                && !matches!(doc.kind, LyricsKind::Instrumental)
    );

    let core_embed = win.core.clone();
    let core_reload = win.core.clone();
    let core_close = win.core.clone();
    let core_retry = win.core.clone();

    let header = v_flex()
        .w_full()
        .px_3p5()
        .py_3()
        .gap_1p5()
        .border_b_1()
        .border_color(hsla(pal.border_subtle))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .min_w_0()
                        .child(Icon::new(IconName::Mic).small().text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Lyrics"),
                        )
                        .when_some(badge_text, |row, b| {
                            row.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(4.))
                                    .bg(hsla(pal.bg_panel))
                                    .text_size(px(9.5))
                                    .text_color(hsla(if b.starts_with("Embedded") {
                                        pal.green
                                    } else {
                                        pal.aqua
                                    }))
                                    .child(b),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .flex_shrink_0()
                        .when(show_embed_btn, |r| {
                            r.child(
                                Button::new("lyr-embed-btn")
                                    .with_variant(ButtonVariant::Primary)
                                    .xsmall()
                                    .label("Embed")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_embed
                                            .update(cx, |c, cx| c.embed_current_lyrics_to_file(cx));
                                    })),
                            )
                        })
                        .child(
                            Button::new("lyr-refresh-btn")
                                .ghost()
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_reload.update(cx, |c, cx| c.force_reload_lyrics(cx));
                                })),
                        )
                        .child(
                            Button::new("lyr-close-btn")
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_close.update(cx, |c, cx| c.toggle_lyrics_panel(cx));
                                })),
                        ),
                ),
        )
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(pal.fg))
                        .truncate()
                        .child(track_title),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(hsla(pal.fg_muted))
                        .truncate()
                        .child(track_sub),
                ),
        );

    let body = match lyrics_state {
        LyricsState::Empty => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .p_5()
            .gap_2()
            .child(Icon::new(IconName::Mic).large().text_color(hsla(pal.fg_muted)))
            .child(
                div()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child("Play a track to view synced lyrics"),
            )
            .into_any_element(),
        LyricsState::Loading => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .p_5()
            .gap_2()
            .child(Icon::new(IconName::Clock).small().text_color(hsla(pal.accent)))
            .child(
                div()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child("Searching lyrics…"),
            )
            .into_any_element(),
        LyricsState::NotFound => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .p_5()
            .gap_2p5()
            .child(Icon::new(IconName::Music).large().text_color(hsla(pal.fg_muted)))
            .child(
                div()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child("No lyrics found for this track"),
            )
            .child(
                Button::new("lyr-retry-btn")
                    .secondary()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .label("Retry LRCLIB")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        core_retry.update(cx, |c, cx| c.force_reload_lyrics(cx));
                    })),
            )
            .into_any_element(),
        LyricsState::Ready(doc) => match doc.kind {
            LyricsKind::Instrumental => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .p_5()
                .gap_2()
                .child(Icon::new(IconName::Music).large().text_color(hsla(pal.aqua)))
                .child(
                    div()
                        .text_xs()
                        .text_color(hsla(pal.fg_subtle))
                        .child("Instrumental track"),
                )
                .into_any_element(),
            LyricsKind::Synced(lines) => {
                let active_idx = active_synced_line_index(&lines, pos_secs);
                if let Some(idx) = active_idx
                    && win.last_scrolled_lyric_idx != Some(idx)
                {
                    win.lyrics_scroll.scroll_to_item(idx);
                    win.last_scrolled_lyric_idx = Some(idx);
                }
                let transparent = rgb_with_alpha(pal.accent, 0x00);

                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("lyrics-synced-scroll")
                            .track_scroll(&win.lyrics_scroll)
                            .overflow_y_scroll()
                            .size_full()
                            .py_2()
                            .px_2p5()
                            .gap_1()
                            .children(lines.into_iter().enumerate().map(|(idx, line)| {
                                let is_active = active_idx == Some(idx);
                                let is_past = active_idx.is_some_and(|a| idx < a);
                                let seek_secs = line.time_secs;
                                let ts_label = format_duration(seek_secs);
                                let core_seek = win.core.clone();

                                h_flex()
                                    .id(SharedString::from(format!("lyr-line-{idx}")))
                                    .w_full()
                                    .items_start()
                                    .gap_2()
                                    .px_2p5()
                                    .py_1p5()
                                    .rounded(px(6.))
                                    .border_l_2()
                                    .border_color(if is_active {
                                        hsla(pal.accent)
                                    } else {
                                        transparent
                                    })
                                    .bg(if is_active {
                                        rgb_with_alpha(pal.accent, 0x24)
                                    } else {
                                        transparent
                                    })
                                    .when(is_seekable, |row| {
                                        row.cursor_pointer()
                                            .hover(|s| s.bg(hsla(pal.bg_panel)))
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                core_seek.update(cx, |c, cx| {
                                                    c.seek_absolute(seek_secs, cx)
                                                });
                                            }))
                                    })
                                    .child(
                                        div()
                                            .w(px(34.))
                                            .flex_shrink_0()
                                            .pt(px(1.))
                                            .text_size(px(10.))
                                            .text_color(hsla(if is_active {
                                                pal.accent
                                            } else {
                                                pal.fg_muted
                                            }))
                                            .child(ts_label),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(if is_active { 13.5 } else { 12.5 }))
                                            .font_weight(if is_active {
                                                FontWeight::BOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .text_color(hsla(if is_active {
                                                pal.fg
                                            } else if is_past {
                                                pal.fg_muted
                                            } else {
                                                pal.fg_subtle
                                            }))
                                            .child(line.text),
                                    )
                            })),
                    )
                    .child(Scrollbar::vertical(&win.lyrics_scroll))
                    .into_any_element()
            }
            LyricsKind::Plain(lines) => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .p_3p5()
                .gap_1p5()
                .children(lines.into_iter().map(|line| {
                    if line.is_empty() {
                        div().h(px(6.)).into_any_element()
                    } else {
                        div()
                            .text_size(px(12.5))
                            .text_color(hsla(pal.fg_subtle))
                            .child(line)
                            .into_any_element()
                    }
                }))
                .into_any_element(),
        },
    };

    v_flex()
        .w(px(320.))
        .h_full()
        .flex_shrink_0()
        .bg(hsla(pal.bg_sidebar))
        .border_l_1()
        .border_color(hsla(pal.border_subtle))
        .child(header)
        .child(body)
}
