use super::{VibesWindow, art, library_view};
use crate::config::PlayTarget;
use crate::core::RadioTab;
use crate::radio::GENRE_CHIPS;
use crate::theme::{hsla, rgb_with_alpha};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};

pub fn render_radio_view(
    win: &mut VibesWindow,
    _window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (
        pal,
        selected_genre,
        stations_with_art,
        loading,
        err_msg,
        active_url,
        playing,
        favorites,
        country_code,
        local_blending,
        radio_tab,
        log_count,
    ) = {
        let core = win.core.read(cx);
        let active_url = match &core.current_target {
            Some(PlayTarget::Radio(s)) => Some(s.stream_url().to_lowercase()),
            _ => None,
        };
        let list: Vec<(crate::radio::RadioStation, Option<std::path::PathBuf>)> = core
            .stations
            .iter()
            .map(|s| (s.clone(), core.art_path_for_url(&s.favicon).cloned()))
            .collect();
        (
            core.palette.clone(),
            core.selected_genre.clone(),
            list,
            core.radio_loading,
            core.radio_error.clone(),
            active_url,
            core.playing && !core.paused,
            core.state.favorites.clone(),
            core.config.country_code.clone(),
            core.config.local_blending,
            core.radio_tab,
            core.state.song_log.len(),
        )
    };

    let core_search = win.core.clone();
    let core_dial = win.core.clone();
    let core_refresh = win.core.clone();
    let core_custom = win.core.clone();
    let show_custom = win.show_custom_station;

    if radio_tab != RadioTab::Directory {
        let fav_count = favorites.len();
        return v_flex()
            .size_full()
            .gap_0()
            .child(render_radio_tabs_bar(win, radio_tab, fav_count, log_count, &pal, cx))
            .child(
                div().flex_1().min_h_0().w_full().child(match radio_tab {
                    RadioTab::Directory => unreachable!(),
                    RadioTab::Favorites => library_view::render_favorites_view(win, cx).into_any_element(),
                    RadioTab::SongLog => library_view::render_song_log_view(win, cx).into_any_element(),
                }),
            )
            .into_any_element();
    }

    let fav_count = favorites.len();
    v_flex()
        .size_full()
        .p_4()
        .gap_3()
        .child(render_radio_tabs_bar(win, radio_tab, fav_count, log_count, &pal, cx))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Radio).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Radio Directory"),
                        )
                        .when(local_blending && !country_code.is_empty(), |row| {
                            row.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(4.))
                                    .bg(hsla(pal.bg_panel))
                                    .text_size(px(10.))
                                    .text_color(hsla(pal.aqua))
                                    .child(format!("Local Mix: {country_code}")),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(
                            Button::new("radio-spin-dial")
                                .primary()
                                .small()
                                .icon(IconName::Shuffle)
                                .label("Spin the Dial")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_dial.update(cx, |c, cx| c.spin_the_dial(cx));
                                })),
                        )
                        .child(
                            Button::new("radio-add-custom")
                                .with_variant(if show_custom {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .small()
                                .icon(IconName::Plus)
                                .label("Custom URL")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.show_custom_station = !this.show_custom_station;
                                    if this.show_custom_station {
                                        gpui_kit::Focusable::focus_handle(&this.custom_url_input, cx)
                                            .focus(window, cx);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("radio-refresh")
                                .ghost()
                                .small()
                                .icon(IconName::RefreshCw)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_refresh.update(cx, |c, cx| c.refresh_radio_stations(cx));
                                })),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .child(
                            Input::new(&win.radio_search_input)
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
                    Button::new("radio-search-go")
                        .secondary()
                        .small()
                        .label("Search")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let q = this.radio_search_input.read(cx).value().to_string();
                            core_search.update(cx, |c, cx| c.set_radio_query(q, cx));
                        })),
                ),
        )
        .when(show_custom, |col| {
            let name_ent = win.custom_name_input.clone();
            let url_ent = win.custom_url_input.clone();
            col.child(
                h_flex()
                    .p_2p5()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.))
                    .bg(hsla(pal.bg_panel))
                    .border_1()
                    .border_color(hsla(pal.border))
                    .child(div().w(px(180.)).child(Input::new(&win.custom_name_input).small()))
                    .child(div().flex_1().child(Input::new(&win.custom_url_input).small()))
                    .child(
                        Button::new("custom-url-add-btn")
                            .primary()
                            .small()
                            .label("Add & Tune")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name = name_ent.read(cx).value().to_string();
                                let url = url_ent.read(cx).value().to_string();
                                core_custom.update(cx, |c, cx| c.add_custom_station(name, url, cx));
                                this.custom_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.custom_url_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.show_custom_station = false;
                                cx.notify();
                            })),
                    ),
            )
        })
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap_1p5()
                .children(GENRE_CHIPS.iter().map(|&genre| {
                    let is_sel = selected_genre.eq_ignore_ascii_case(genre);
                    let core_ent = win.core.clone();
                    Button::new(SharedString::from(format!("genre-{genre}")))
                        .with_variant(if is_sel {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .xsmall()
                        .label(genre)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            core_ent.update(cx, |c, cx| c.set_genre(genre, cx));
                        }))
                })),
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
                    .child(format!("RadioBrowser error: {err}")),
            )
        })
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .overflow_y_scrollbar()
                .when(loading && stations_with_art.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Loading global stations from RadioBrowser…"),
                    )
                })
                .when(!loading && stations_with_art.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("No stations matched. Try another search or genre chip."),
                    )
                })
                .children(stations_with_art.into_iter().enumerate().map(|(ix, (station, favicon_path))| {
                    let is_active = active_url
                        .as_deref()
                        .is_some_and(|u| u == station.stream_url().to_lowercase());
                    let is_fav = favorites.iter().any(|f| {
                        (!f.stationuuid.is_empty() && f.stationuuid == station.stationuuid)
                            || f.stream_url().eq_ignore_ascii_case(station.stream_url())
                    });
                    let core_play = win.core.clone();
                    let core_fav = win.core.clone();
                    let st_play = station.clone();
                    let st_fav = station.clone();

                    let meta_line = if station.tags.is_empty() {
                        station.country.clone()
                    } else if station.country.is_empty() {
                        station.tags.clone()
                    } else {
                        format!("{} · {}", station.country, station.tags)
                    };

                    div()
                        .id(SharedString::from(format!("station-row-{ix}")))
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
                            let s = st_play.clone();
                            core_play.update(cx, |c, cx| c.play_station(s, cx));
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
                                        .child(art::render_art_tile(
                                            favicon_path.as_ref(),
                                            38.,
                                            7.,
                                            IconName::Radio,
                                            &station.name,
                                            is_active,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_0p5()
                                                .child(
                                                    h_flex()
                                                        .gap_1p5()
                                                        .items_center()
                                                        .when(is_active && playing, |r| {
                                                            r.child(
                                                                Icon::new(IconName::AudioLines)
                                                                    .xsmall()
                                                                    .text_color(hsla(pal.accent)),
                                                            )
                                                        })
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
                                                                .child(station.name.clone()),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .truncate()
                                                        .child(meta_line),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .flex_shrink_0()
                                        .when(!station.countrycode.is_empty(), |r| {
                                            r.child(
                                                div()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded(px(4.))
                                                    .bg(hsla(pal.bg_panel))
                                                    .text_size(px(10.))
                                                    .text_color(hsla(pal.fg_subtle))
                                                    .child(station.countrycode.to_uppercase()),
                                            )
                                        })
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(px(4.))
                                                .bg(hsla(pal.bg_btn))
                                                .text_size(px(10.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(hsla(if station.bitrate >= 192 {
                                                    pal.green
                                                } else {
                                                    pal.aqua
                                                }))
                                                .child(station.display_codec()),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("fav-btn-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(Icon::new(IconName::Star).xsmall().text_color(
                                                    hsla(if is_fav {
                                                        pal.yellow
                                                    } else {
                                                        pal.fg_muted
                                                    }),
                                                ))
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let s = st_fav.clone();
                                                    core_fav.update(cx, |c, cx| {
                                                        c.toggle_favorite_station(s, cx)
                                                    });
                                                })),
                                        ),
                                ),
                        )
                })),
        )
        .into_any_element()
}

fn render_radio_tabs_bar(
    win: &VibesWindow,
    active_tab: RadioTab,
    fav_count: usize,
    log_count: usize,
    pal: &crate::theme::Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let _ = pal;
    h_flex()
        .gap_1p5()
        .pb_1()
        .children(RadioTab::ALL.into_iter().map(|t| {
            let active = t == active_tab;
            let core_ent = win.core.clone();
            let label = match t {
                RadioTab::Directory => "Directory".to_string(),
                RadioTab::Favorites => format!("Starred Presets ({fav_count})"),
                RadioTab::SongLog => format!("Heard on Radio ({log_count})"),
            };
            Button::new(SharedString::from(format!("rad-tab-{}", t.label())))
                .with_variant(if active {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .xsmall()
                .label(label)
                .on_click(cx.listener(move |_, _, _, cx| {
                    core_ent.update(cx, |c, cx| {
                        c.radio_tab = t;
                        cx.notify();
                    });
                }))
        }))
}
