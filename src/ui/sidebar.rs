use super::VibesWindow;
use crate::core::{NavSection, VibesCore};
use crate::theme::hsla;
use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px,
};

pub fn render_sidebar(win: &mut VibesWindow, cx: &mut Context<VibesWindow>) -> impl IntoElement {
    let core = win.core.read(cx);
    let pal = core.palette.clone();
    let active_sec = core.section;
    let fav_count = core.state.favorites.len();
    let log_count = core.state.song_log.len();
    let queue_count = core.queue.len();
    let active_dl = core.active_saving_count();
    let dl_total = core.downloads.len();
    let presets: Vec<(usize, String, crate::radio::RadioStation)> = core
        .state
        .favorites
        .iter()
        .take(6)
        .enumerate()
        .map(|(i, s)| (i + 1, s.name.clone(), s.clone()))
        .collect();
    let current_stream = match &core.current_target {
        Some(crate::config::PlayTarget::Radio(s)) => Some(s.stream_url().to_lowercase()),
        _ => None,
    };

    let nav_items: Vec<(NavSection, IconName, &str, Option<String>)> = vec![
        (NavSection::Radio, IconName::Radio, "Radio", None),
        (NavSection::Youtube, IconName::Headphones, "YouTube Audio", None),
        (
            NavSection::Favorites,
            IconName::Star,
            "Favorites",
            (fav_count > 0).then(|| fav_count.to_string()),
        ),
        (
            NavSection::SongLog,
            IconName::Clock,
            "Song Log",
            (log_count > 0).then(|| log_count.to_string()),
        ),
        (
            NavSection::Queue,
            IconName::ListMusic,
            "Queue",
            (queue_count > 0).then(|| queue_count.to_string()),
        ),
        (
            NavSection::Downloads,
            IconName::Download,
            "Saved Songs",
            if active_dl > 0 {
                Some(format!("{active_dl}…"))
            } else if dl_total > 0 {
                Some(dl_total.to_string())
            } else {
                None
            },
        ),
        (NavSection::Settings, IconName::Settings, "Settings", None),
    ];

    v_flex()
        .w(px(210.))
        .h_full()
        .flex_shrink_0()
        .bg(hsla(pal.bg_sidebar))
        .border_r_1()
        .border_color(hsla(pal.border_subtle))
        .p_2p5()
        .gap_1()
        .child(
            div()
                .px_2()
                .py_1()
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hsla(pal.fg_muted))
                .child("CHANNELS"),
        )
        .children(nav_items.into_iter().map(|(sec, icon, label, badge)| {
            let is_active = sec == active_sec;
            let core_ent = win.core.clone();
            div()
                .id(SharedString::from(format!("nav-{label}")))
                .w_full()
                .h(px(32.))
                .px_2p5()
                .rounded(px(6.))
                .cursor_pointer()
                .bg(hsla(if is_active { pal.bg_panel } else { pal.bg_sidebar }))
                .hover(|s| s.bg(hsla(pal.bg_panel)))
                .on_click(cx.listener(move |_, _, _, cx| {
                    core_ent.update(cx, |c: &mut VibesCore, cx| {
                        c.section = sec;
                        cx.notify();
                    });
                }))
                .child(
                    h_flex()
                        .size_full()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(icon).small().text_color(hsla(if is_active {
                                    pal.accent
                                } else {
                                    pal.fg_muted
                                })))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(if is_active {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::MEDIUM
                                        })
                                        .text_color(hsla(if is_active { pal.fg } else { pal.fg_subtle }))
                                        .child(label),
                                ),
                        )
                        .when_some(badge, |row, b| {
                            row.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(99.))
                                    .bg(hsla(pal.bg_btn))
                                    .text_size(px(10.))
                                    .text_color(hsla(pal.fg_muted))
                                    .child(b),
                            )
                        }),
                )
        }))
        .when(!presets.is_empty(), |rail| {
            rail.child(
                div()
                    .mt_3()
                    .px_2()
                    .py_1()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(pal.fg_muted))
                    .child("PRESETS (1-6)"),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_0p5()
                    .overflow_y_scrollbar()
                    .children(presets.into_iter().map(|(slot, name, station)| {
                        let is_playing_preset = current_stream
                            .as_deref()
                            .is_some_and(|u| u == station.stream_url().to_lowercase());
                        let core_ent = win.core.clone();
                        div()
                            .id(SharedString::from(format!("preset-{slot}")))
                            .w_full()
                            .h(px(28.))
                            .px_2()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(hsla(pal.bg_panel)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let st = station.clone();
                                core_ent.update(cx, |c, cx| c.play_station(st, cx));
                            }))
                            .child(
                                h_flex()
                                    .size_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .h(px(16.))
                                            .rounded(px(4.))
                                            .bg(hsla(if is_playing_preset {
                                                pal.accent
                                            } else {
                                                pal.bg_btn
                                            }))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(9.5))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(hsla(if is_playing_preset {
                                                pal.bg_deep
                                            } else {
                                                pal.fg_subtle
                                            }))
                                            .child(format!("{slot}")),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .truncate()
                                            .text_color(hsla(if is_playing_preset {
                                                pal.accent
                                            } else {
                                                pal.fg_subtle
                                            }))
                                            .child(name),
                                    ),
                            )
                    })),
            )
        })
}
