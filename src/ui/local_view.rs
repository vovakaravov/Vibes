use super::{VibesWindow, art};
use crate::config::PlayTarget;
use crate::local_library::{LocalAlbum, LocalTrack};
use crate::theme::{Palette, hsla, rgb_with_alpha};
use crate::youtube::format_duration;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use std::path::PathBuf;

pub fn render_local_view(
    win: &mut VibesWindow,
    _window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (
        pal,
        snapshot,
        loading,
        filter_query,
        tab_singles,
        selected_folder,
        albums_dir,
        active_local_path,
        lyrics_backfill,
    ) = {
        let core = win.core.read(cx);
        let active_path = match &core.current_target {
            Some(PlayTarget::Local(t)) => Some(t.path.clone()),
            _ => None,
        };
        (
            core.palette.clone(),
            core.local_library.clone(),
            core.local_library_loading,
            core.local_filter_query.clone(),
            core.local_tab_singles,
            core.selected_local_album.clone(),
            core.config.albums_dir.clone(),
            active_path,
            core.lyrics_backfill.clone(),
        )
    };

    let core_refresh = win.core.clone();
    let core_open = win.core.clone();
    let core_backfill = win.core.clone();
    let core_tab_albums = win.core.clone();
    let core_tab_singles = win.core.clone();
    let open_root = albums_dir.clone();
    let backfill_active = lyrics_backfill.active;
    let show_backfill_banner = backfill_active || lyrics_backfill.processed > 0;
    let backfill_summary = lyrics_backfill.summary();

    let total_albums = snapshot.albums.len();
    let total_singles = snapshot.singles.len();
    let stats_text = format!(
        "{total_albums} {} · {total_singles} {}",
        if total_albums == 1 { "album" } else { "albums" },
        if total_singles == 1 {
            "single"
        } else {
            "singles"
        }
    );

    let selected_album = if !tab_singles {
        selected_folder
            .as_ref()
            .and_then(|f| snapshot.albums.iter().find(|a| &a.folder == f).cloned())
    } else {
        None
    };

    v_flex()
        .size_full()
        .p_4()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Music).text_color(hsla(pal.green)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Local Library · Offline Playback"),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .flex_shrink_0()
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(hsla(pal.bg_panel))
                                .text_size(px(10.5))
                                .text_color(hsla(pal.green))
                                .child(stats_text),
                        )
                        .child(
                            Button::new("local-refresh-btn")
                                .secondary()
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .label(if loading { "Scanning…" } else { "Refresh" })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_refresh.update(cx, |c, cx| c.refresh_local_library(cx));
                                })),
                        )
                        .child(
                            Button::new("local-embed-lyrics-btn")
                                .with_variant(if backfill_active {
                                    ButtonVariant::Danger
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .icon(IconName::Mic)
                                .label(if backfill_active {
                                    "Cancel Lyrics"
                                } else {
                                    "Embed Lyrics"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_backfill.update(cx, |c, cx| {
                                        if c.lyrics_backfill.active {
                                            c.cancel_lyrics_backfill(cx);
                                        } else {
                                            c.start_lyrics_backfill(cx);
                                        }
                                    });
                                })),
                        )
                        .child(
                            Button::new("local-open-music-btn")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Folder)
                                .label("Open Music")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_open.read(cx).open_folder(&open_root);
                                })),
                        ),
                ),
        )
        .when(show_backfill_banner, |col| {
            col.child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .rounded(px(6.))
                    .bg(hsla(pal.bg_panel))
                    .border_1()
                    .border_color(hsla(pal.border_subtle))
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Mic)
                            .xsmall()
                            .text_color(hsla(if backfill_active {
                                pal.yellow
                            } else {
                                pal.green
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(hsla(pal.fg_subtle))
                            .child(backfill_summary.clone()),
                    ),
            )
        })
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div().flex_1().child(
                        Input::new(&win.local_search_input)
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
                    h_flex()
                        .gap_1()
                        .flex_shrink_0()
                        .child(
                            Button::new("local-tab-albums")
                                .with_variant(if !tab_singles {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .small()
                                .icon(IconName::Disc3)
                                .label(format!("Albums ({total_albums})"))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_tab_albums
                                        .update(cx, |c, cx| c.set_local_tab_singles(false, cx));
                                })),
                        )
                        .child(
                            Button::new("local-tab-singles")
                                .with_variant(if tab_singles {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .small()
                                .icon(IconName::AudioLines)
                                .label(format!("Radio & Singles ({total_singles})"))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_tab_singles
                                        .update(cx, |c, cx| c.set_local_tab_singles(true, cx));
                                })),
                        ),
                ),
        )
        .child(if tab_singles {
            render_local_singles_list(
                win,
                &snapshot.singles,
                &filter_query,
                loading,
                active_local_path.as_ref(),
                &pal,
                cx,
            )
            .into_any_element()
        } else if let Some(album) = selected_album {
            render_local_album_detail(win, album, active_local_path.as_ref(), &pal, cx)
                .into_any_element()
        } else {
            render_local_albums_list(
                win,
                &snapshot.albums,
                &filter_query,
                loading,
                active_local_path.as_ref(),
                &pal,
                cx,
            )
            .into_any_element()
        })
}

fn format_album_subtitle(album: &LocalAlbum) -> String {
    let mut parts = vec![album.artist.clone()];
    if let Some(y) = album.year {
        parts.push(y.to_string());
    }
    let count = album.tracks.len();
    parts.push(format!(
        "{count} {}",
        if count == 1 { "track" } else { "tracks" }
    ));
    let tot = album.total_duration_secs();
    if tot > 0.0 {
        parts.push(format_duration(tot));
    }
    parts.join(" · ")
}

fn render_local_albums_list(
    win: &VibesWindow,
    albums: &[LocalAlbum],
    filter_query: &str,
    loading: bool,
    active_local_path: Option<&PathBuf>,
    pal: &Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let q = filter_query.trim().to_lowercase();
    let filtered: Vec<LocalAlbum> = if q.is_empty() {
        albums.to_vec()
    } else {
        albums
            .iter()
            .filter(|a| {
                a.title.to_lowercase().contains(&q)
                    || a.artist.to_lowercase().contains(&q)
                    || a.tracks.iter().any(|t| t.title.to_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    };

    v_flex()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .when(filtered.is_empty(), |list| {
            list.child(
                div()
                    .p_6()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child(if loading {
                        "Scanning ~/Music for offline albums…"
                    } else if albums.is_empty() {
                        "No local album folders found yet. Download albums from the Albums tab or add folders under ~/Music."
                    } else {
                        "No local albums match your filter."
                    }),
            )
        })
        .when(!filtered.is_empty(), |list| {
            list.child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .pb_2()
                    .children(filtered.into_iter().enumerate().map(|(ix, alb)| {
                        let core_open = win.core.clone();
                        let core_play = win.core.clone();
                        let folder_open = alb.folder.clone();
                        let alb_play = alb.clone();
                        let is_playing_album = active_local_path
                            .is_some_and(|p| alb.tracks.iter().any(|t| &t.path == p));
                        let fmt_badge = alb.format_summary();
                        let trk_count = alb.tracks.len();
                        let meta_txt = if is_playing_album {
                            format!("PLAYING · {trk_count} trk")
                        } else {
                            format!("{trk_count} trk · {fmt_badge}")
                        };

                        div()
                            .id(SharedString::from(format!("loc-alb-{ix}")))
                            .w(px(172.))
                            .p_2p5()
                            .rounded(px(9.))
                            .cursor_pointer()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .bg(if is_playing_album {
                                rgb_with_alpha(pal.accent, 0x20)
                            } else {
                                hsla(pal.bg_sidebar)
                            })
                            .border_1()
                            .border_color(hsla(if is_playing_album {
                                pal.accent
                            } else {
                                pal.border_subtle
                            }))
                            .hover(|s| s.bg(hsla(pal.bg_panel)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let f = folder_open.clone();
                                core_open.update(cx, |c, cx| c.select_local_album(Some(f), cx));
                            }))
                            .child(art::render_art_tile(
                                alb.cover_path.as_ref(),
                                150.,
                                7.,
                                IconName::Disc3,
                                &alb.title,
                                is_playing_album,
                                &pal,
                            ))
                            .child(
                                v_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(hsla(if is_playing_album {
                                                pal.accent
                                            } else {
                                                pal.fg
                                            }))
                                            .truncate()
                                            .child(alb.title),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(hsla(pal.fg_muted))
                                            .truncate()
                                            .child(alb.artist),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(hsla(if is_playing_album {
                                                pal.green
                                            } else {
                                                pal.fg_muted
                                            }))
                                            .truncate()
                                            .child(meta_txt),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "loc-alb-play-{ix}"
                                        )))
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::Play)
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let a = alb_play.clone();
                                            core_play.update(cx, |c, cx| {
                                                c.play_local_album_from(&a, 0, cx)
                                            });
                                        })),
                                    ),
                            )
                    })),
            )
        })
}

fn render_local_album_detail(
    win: &VibesWindow,
    album: LocalAlbum,
    active_local_path: Option<&PathBuf>,
    pal: &Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let core_back = win.core.clone();
    let core_play_all = win.core.clone();
    let core_dir = win.core.clone();

    let alb_play = album.clone();
    let open_dir = album.folder.clone();
    let sub = format_album_subtitle(&album);
    let alb_artist = album.artist.clone();
    let alb_title = album.title.clone();
    let alb_cover = album.cover_path.clone();
    let tracks = album.tracks.clone();

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_3()
        .child(
            h_flex()
                .p_3()
                .justify_between()
                .items_center()
                .gap_3()
                .rounded(px(8.))
                .bg(hsla(pal.bg_panel))
                .border_1()
                .border_color(hsla(pal.border))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_3()
                        .child(
                            Button::new("loc-detail-back")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowLeft)
                                .label("Albums")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_back.update(cx, |c, cx| c.select_local_album(None, cx));
                                })),
                        )
                        .child(art::render_art_tile(
                            alb_cover.as_ref(),
                            84.,
                            8.,
                            IconName::Disc3,
                            &alb_title,
                            false,
                            &pal,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_base()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(hsla(pal.fg))
                                                .truncate()
                                                .child(alb_title),
                                        )
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(px(4.))
                                                .bg(rgb_with_alpha(pal.green, 0x26))
                                                .text_size(px(9.5))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(hsla(pal.green))
                                                .child("OFFLINE"),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(hsla(pal.fg_muted))
                                        .truncate()
                                        .child(sub),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .flex_shrink_0()
                        .child(
                            Button::new("loc-detail-play-all")
                                .primary()
                                .small()
                                .icon(IconName::Play)
                                .label("Play Album")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let a = alb_play.clone();
                                    core_play_all
                                        .update(cx, |c, cx| c.play_local_album_from(&a, 0, cx));
                                })),
                        )
                        .child(
                            Button::new("loc-detail-folder")
                                .ghost()
                                .small()
                                .icon(IconName::Folder)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_dir.read(cx).open_folder(&open_dir);
                                })),
                        ),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .overflow_y_scrollbar()
                .children(tracks.into_iter().enumerate().map(|(ix, tr)| {
                    let is_active = active_local_path == Some(&tr.path);
                    let tr_num = tr.track_number.unwrap_or((ix + 1) as u32);
                    let show_artist = !tr.artist.eq_ignore_ascii_case(&alb_artist);
                    let core_play = win.core.clone();
                    let core_q = win.core.clone();
                    let alb_for_play = album.clone();
                    let tr_for_q = tr.clone();

                    div()
                        .id(SharedString::from(format!("loc-det-trk-{ix}")))
                        .w_full()
                        .px_3()
                        .py_1p5()
                        .rounded(px(6.))
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
                            let a = alb_for_play.clone();
                            core_play.update(cx, |c, cx| c.play_local_album_from(&a, ix, cx));
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
                                        .child(
                                            div()
                                                .w(px(22.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(hsla(if is_active {
                                                    pal.accent
                                                } else {
                                                    pal.fg_muted
                                                }))
                                                .child(if is_active {
                                                    Icon::new(IconName::AudioLines)
                                                        .xsmall()
                                                        .text_color(hsla(pal.accent))
                                                        .into_any_element()
                                                } else {
                                                    div()
                                                        .child(format!("{tr_num:02}"))
                                                        .into_any_element()
                                                }),
                                        )
                                        .child(art::render_art_tile(
                                            tr.cover_path.as_ref(),
                                            28.,
                                            5.,
                                            IconName::Music,
                                            &tr.title,
                                            is_active,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(hsla(if is_active {
                                                            pal.accent
                                                        } else {
                                                            pal.fg
                                                        }))
                                                        .truncate()
                                                        .child(tr.title),
                                                )
                                                .when(show_artist, |col| {
                                                    col.child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(hsla(pal.fg_muted))
                                                            .truncate()
                                                            .child(tr.artist),
                                                    )
                                                }),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .flex_shrink_0()
                                        .when(!tr.format.is_empty(), |r| {
                                            r.child(
                                                div()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded(px(4.))
                                                    .bg(hsla(pal.bg_btn))
                                                    .text_size(px(9.5))
                                                    .text_color(hsla(pal.aqua))
                                                    .child(tr.format),
                                            )
                                        })
                                        .when_some(tr.duration_secs, |r, d| {
                                            r.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(hsla(pal.fg_muted))
                                                    .child(format_duration(d)),
                                            )
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "loc-det-q-{ix}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Plus)
                                            .label("Queue")
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.stop_propagation();
                                                let t = tr_for_q.clone();
                                                core_q.update(cx, |c, cx| {
                                                    c.queue_local_track(t, cx)
                                                });
                                            })),
                                        ),
                                ),
                        )
                })),
        )
}

fn render_local_singles_list(
    win: &VibesWindow,
    singles: &[LocalTrack],
    filter_query: &str,
    loading: bool,
    active_local_path: Option<&PathBuf>,
    pal: &Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let q = filter_query.trim().to_lowercase();
    let filtered: Vec<LocalTrack> = if q.is_empty() {
        singles.to_vec()
    } else {
        singles
            .iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    };

    let play_snapshot = filtered.clone();

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1p5()
        .overflow_y_scrollbar()
        .when(filtered.is_empty(), |list| {
            list.child(
                div()
                    .p_6()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child(if loading {
                        "Scanning ~/Music/Radio for saved singles…"
                    } else if singles.is_empty() {
                        "No saved radio tracks or loose singles found yet. Use Save Song (Ctrl+S) while listening to Radio or YouTube Audio."
                    } else {
                        "No saved tracks match your filter."
                    }),
            )
        })
        .children(filtered.into_iter().enumerate().map(|(ix, tr)| {
            let is_active = active_local_path == Some(&tr.path);
            let core_play = win.core.clone();
            let core_q = win.core.clone();
            let list_play = play_snapshot.clone();
            let tr_q = tr.clone();

            let mut sub_parts = vec![tr.artist.clone()];
            if !tr.format.is_empty() {
                sub_parts.push(tr.format.clone());
            }
            if let Some(d) = tr.duration_secs {
                sub_parts.push(format_duration(d));
            }
            let sub = sub_parts.join(" · ");

            div()
                .id(SharedString::from(format!("loc-sng-{ix}")))
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
                    let tracks = list_play.clone();
                    core_play.update(cx, |c, cx| c.play_local_singles_from(&tracks, ix, cx));
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
                                    tr.cover_path.as_ref(),
                                    40.,
                                    6.,
                                    IconName::Music,
                                    &tr.title,
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
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(hsla(if is_active {
                                                    pal.accent
                                                } else {
                                                    pal.fg
                                                }))
                                                .truncate()
                                                .child(tr.title),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(hsla(pal.fg_muted))
                                                .truncate()
                                                .child(sub),
                                        ),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .flex_shrink_0()
                                .child(
                                    Button::new(SharedString::from(format!("loc-sng-q-{ix}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Plus)
                                        .label("Queue")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let t = tr_q.clone();
                                            core_q.update(cx, |c, cx| c.queue_local_track(t, cx));
                                        })),
                                ),
                        ),
                )
        }))
}
