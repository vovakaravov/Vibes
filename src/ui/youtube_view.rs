use super::{VibesWindow, art};
use crate::config::PlayTarget;
use crate::theme::{hsla, rgb_with_alpha};
use crate::youtube::{YoutubeFilter, format_duration};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};

pub fn render_youtube_view(
    win: &mut VibesWindow,
    _window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, filtered, filter, loading, err_msg, active_url, recent, resume_map) = {
        let core = win.core.read(cx);
        let active_url = match &core.current_target {
            Some(PlayTarget::Youtube(y)) => Some(y.url.clone()),
            _ => None,
        };
        let filter = core.yt_filter;
        let filtered: Vec<(crate::youtube::YoutubeItem, Option<std::path::PathBuf>)> = core
            .yt_results
            .iter()
            .filter(|i| filter.matches(i))
            .map(|i| (i.clone(), core.art_path_for_youtube(&i.id).cloned()))
            .collect();
        (
            core.palette.clone(),
            filtered,
            filter,
            core.yt_loading,
            core.yt_error.clone(),
            active_url,
            core.state.recent_yt_searches.clone(),
            core.state.resume_positions.clone(),
        )
    };
    let core_go = win.core.clone();

    v_flex()
        .size_full()
        .p_4()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Headphones).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("YouTube Audio-Only"),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(hsla(pal.bg_panel))
                                .text_size(px(10.))
                                .text_color(hsla(pal.green))
                                .child("No Video · Zero Ads · Opus"),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div().flex_1().child(
                        Input::new(&win.yt_search_input)
                            .small()
                            .cleanable(true)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .xsmall()
                                    .text_color(hsla(pal.fg_muted)),
                            ),
                    ),
                )
                .child(
                    Button::new("yt-search-btn")
                        .primary()
                        .small()
                        .label(if loading { "Searching…" } else { "Search / URL" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let q = this.yt_search_input.read(cx).value().to_string();
                            core_go.update(cx, |c, cx| c.search_youtube_query(q, cx));
                        })),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .flex_wrap()
                .gap_2()
                .child(
                    h_flex()
                        .gap_1p5()
                        .children(YoutubeFilter::ALL.into_iter().map(|f| {
                            let active = f == filter;
                            let core_ent = win.core.clone();
                            Button::new(SharedString::from(format!("ytf-{}", f.label())))
                                .with_variant(if active {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(f.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_ent.update(cx, |c, cx| {
                                        c.yt_filter = f;
                                        cx.notify();
                                    });
                                }))
                        })),
                )
                .when(!recent.is_empty(), |row| {
                    row.child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .children(recent.into_iter().take(4).enumerate().map(|(ix, q)| {
                                let core_ent = win.core.clone();
                                let q_click = q.clone();
                                Button::new(SharedString::from(format!("yt-rec-{ix}")))
                                    .ghost()
                                    .xsmall()
                                    .label(q)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let query = q_click.clone();
                                        this.yt_search_input.update(cx, |s, cx| {
                                            s.set_value(&query, window, cx)
                                        });
                                        core_ent
                                            .update(cx, |c, cx| c.search_youtube_query(query, cx));
                                    }))
                            })),
                    )
                }),
        )
        .when_some(err_msg, |col, err| {
            col.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(hsla(pal.bg_panel))
                    .text_xs()
                    .text_color(hsla(pal.red))
                    .child(err),
            )
        })
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .overflow_y_scrollbar()
                .when(filtered.is_empty() && !loading, |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Search for music mixes, podcasts, or paste any YouTube link to stream audio-only via mpv."),
                    )
                })
                .children(filtered.into_iter().enumerate().map(|(ix, (item, thumb_path))| {
                    let is_active = active_url.as_deref() == Some(item.url.as_str());
                    let saved_pos = resume_map.get(&item.id).copied();
                    let core_play = win.core.clone();
                    let core_queue = win.core.clone();
                    let core_save = win.core.clone();
                    let item_play = item.clone();
                    let item_queue = item.clone();
                    let item_save = item.clone();

                    let dur_badge = if item.is_live {
                        "LIVE".to_string()
                    } else if let Some(d) = item.duration_secs {
                        format_duration(d)
                    } else {
                        "AUDIO".to_string()
                    };

                    div()
                        .id(SharedString::from(format!("yt-row-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .cursor_pointer()
                        .bg(if is_active {
                            rgb_with_alpha(pal.accent, 0x26)
                        } else {
                            hsla(pal.bg_sidebar)
                        })
                        .border_1()
                        .border_color(hsla(if is_active {
                            pal.accent
                        } else {
                            pal.border_subtle
                        }))
                        .hover(|s| s.bg(hsla(pal.bg_panel)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let it = item_play.clone();
                            core_play.update(cx, |c, cx| c.play_youtube(it, cx));
                        }))
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .gap_3()
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .items_center()
                                        .gap_2p5()
                                        .child(art::render_art_rect(
                                            thumb_path.as_ref(),
                                            64.,
                                            36.,
                                            5.,
                                            IconName::Headphones,
                                            &item.title,
                                            is_active,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_0p5()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(hsla(if is_active {
                                                            pal.accent
                                                        } else {
                                                            pal.fg
                                                        }))
                                                        .truncate()
                                                        .child(item.title.clone()),
                                                )
                                                .child(
                                                    h_flex()
                                                        .gap_2()
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(hsla(pal.fg_muted))
                                                                .truncate()
                                                                .child(item.channel.clone()),
                                                        )
                                                        .when_some(saved_pos, |r, pos| {
                                                            r.child(
                                                                div()
                                                                    .px_1p5()
                                                                    .rounded(px(4.))
                                                                    .bg(hsla(pal.bg_panel))
                                                                    .text_size(px(10.))
                                                                    .text_color(hsla(pal.yellow))
                                                                    .child(format!(
                                                                        "Resume {}",
                                                                        format_duration(pos)
                                                                    )),
                                                            )
                                                        }),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .items_center()
                                        .flex_shrink_0()
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(px(4.))
                                                .bg(hsla(pal.bg_btn))
                                                .text_size(px(10.5))
                                                .text_color(hsla(if item.is_live {
                                                    pal.red
                                                } else {
                                                    pal.aqua
                                                }))
                                                .child(dur_badge),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("yt-q-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::ListMusic)
                                                .label("+Queue")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let it = item_queue.clone();
                                                    core_queue.update(cx, |c, cx| {
                                                        c.queue_youtube_item(it, cx)
                                                    });
                                                })),
                                        )
                                        .when(!item.is_live, |row| {
                                            row.child(
                                                Button::new(SharedString::from(format!(
                                                    "yt-save-{ix}"
                                                )))
                                                .secondary()
                                                .xsmall()
                                                .icon(IconName::Download)
                                                .label("Save")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let it = item_save.clone();
                                                    core_save.update(cx, |c, cx| {
                                                        c.save_youtube_item(&it, cx)
                                                    });
                                                })),
                                            )
                                        }),
                                ),
                        )
                })),
        )
}
