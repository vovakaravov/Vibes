use super::{VibesWindow, albums_view, art, youtube_view};
use crate::core::DiscoverTab;
use crate::discography::filter_clean_releases;
use crate::theme::hsla;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};

pub fn render_discover_view(
    win: &mut VibesWindow,
    window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, tab, clean_editions) = {
        let core = win.core.read(cx);
        (core.palette.clone(), core.discover_tab, core.config.clean_editions)
    };
    let core_clean = win.core.clone();

    v_flex()
        .size_full()
        .gap_0()
        .child(
            h_flex()
                .px_4()
                .pt_3()
                .pb_2()
                .justify_between()
                .items_center()
                .border_b_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    h_flex()
                        .gap_1p5()
                        .children(DiscoverTab::ALL.into_iter().map(|t| {
                            let active = t == tab;
                            let core_ent = win.core.clone();
                            Button::new(SharedString::from(format!("disc-tab-{}", t.label())))
                                .with_variant(if active {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .icon(match t {
                                    DiscoverTab::Artists => IconName::Library,
                                    DiscoverTab::Albums => IconName::Disc3,
                                    DiscoverTab::Tracks => IconName::Headphones,
                                })
                                .label(t.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_ent.update(cx, |c, cx| {
                                        c.discover_tab = t;
                                        cx.notify();
                                    });
                                }))
                        })),
                )
                .child(
                    Button::new("disc-clean-editions")
                        .with_variant(if clean_editions {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .xsmall()
                        .icon(IconName::Check)
                        .label(if clean_editions {
                            "Clean Editions: On"
                        } else {
                            "Clean Editions: Off"
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            core_clean.update(cx, |c, cx| c.toggle_clean_editions(cx));
                        })),
                ),
        )
        .child(
            div().flex_1().min_h_0().w_full().child(match tab {
                DiscoverTab::Artists => render_artist_discovery(win, cx).into_any_element(),
                DiscoverTab::Albums => {
                    albums_view::render_albums_view(win, window, cx).into_any_element()
                }
                DiscoverTab::Tracks => {
                    youtube_view::render_youtube_view(win, window, cx).into_any_element()
                }
            }),
        )
}

fn render_artist_discovery(win: &VibesWindow, cx: &mut Context<VibesWindow>) -> impl IntoElement {
    let (
        pal,
        results_with_art,
        selected_disco,
        loading,
        detail_loading,
        err_msg,
        clean_editions,
        followed_list,
        sync_status,
        art_cache,
    ) = {
        let core = win.core.read(cx);
        let res_art: Vec<(crate::discography::ArtistSummary, Option<std::path::PathBuf>)> = core
            .artist_results
            .iter()
            .map(|a| {
                let p = a
                    .avatar_url
                    .as_deref()
                    .and_then(|u| core.art_path_for_url(u).cloned());
                (a.clone(), p)
            })
            .collect();
        (
            core.palette.clone(),
            res_art,
            core.selected_online_artist.clone(),
            core.artist_loading,
            core.online_artist_loading,
            core.artist_error.clone(),
            core.config.clean_editions,
            core.state.followed_artists.clone(),
            core.artist_sync.clone(),
            core.art_cache.clone(),
        )
    };

    let core_search = win.core.clone();
    let core_cancel = win.core.clone();

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
                        .child(Icon::new(IconName::Library).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Discover & Follow Artists"),
                        ),
                )
                .when(sync_status.is_active(), |r| {
                    r.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(px(4.))
                                    .bg(hsla(pal.bg_panel))
                                    .text_xs()
                                    .text_color(hsla(pal.yellow))
                                    .truncate()
                                    .child(sync_status.summary_line()),
                            )
                            .child(
                                Button::new("disc-cancel-sync")
                                    .danger()
                                    .xsmall()
                                    .label("Stop")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_cancel.update(cx, |c, cx| c.cancel_artist_sync(cx));
                                    })),
                            ),
                    )
                }),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div().flex_1().child(
                        Input::new(&win.artist_search_input)
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
                    Button::new("art-search-go")
                        .primary()
                        .small()
                        .label(if loading { "Searching…" } else { "Search Artists" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let q = this.artist_search_input.read(cx).value().to_string();
                            core_search.update(cx, |c, cx| c.search_artists_query(q, cx));
                        })),
                ),
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
        .child(if detail_loading {
            div()
                .flex_1()
                .p_6()
                .text_xs()
                .text_color(hsla(pal.fg_muted))
                .child("Loading artist discography (Albums & Singles)…")
                .into_any_element()
        } else if let Some(disco) = selected_disco {
            let core_back = win.core.clone();
            let core_follow = win.core.clone();
            let core_unfollow = win.core.clone();
            let is_followed = followed_list
                .iter()
                .any(|f| f.browse_id == disco.artist.browse_id);
            let avatar_path = disco
                .artist
                .avatar_url
                .as_deref()
                .and_then(|u| art_cache.get(u));
            let clean_albums = filter_clean_releases(&disco.albums, clean_editions);
            let clean_singles = filter_clean_releases(&disco.singles_and_eps, clean_editions);
            let hidden_count = (disco.albums.len() + disco.singles_and_eps.len())
                .saturating_sub(clean_albums.len() + clean_singles.len());
            let art_sum = disco.artist.clone();
            let art_bid = disco.artist.browse_id.clone();

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
                                    Button::new("disc-art-back")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ArrowLeft)
                                        .label("Results")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            core_back.update(cx, |c, cx| {
                                                c.clear_selected_online_artist(cx)
                                            });
                                        })),
                                )
                                .child(art::render_art_tile(
                                    avatar_path,
                                    64.,
                                    32.,
                                    IconName::Library,
                                    &disco.artist.name,
                                    false,
                                    &pal,
                                ))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_base()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(hsla(pal.fg))
                                                .truncate()
                                                .child(disco.artist.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(hsla(pal.fg_muted))
                                                .truncate()
                                                .child(format!(
                                                    "{} albums · {} singles/EPs{}",
                                                    clean_albums.len(),
                                                    clean_singles.len(),
                                                    if hidden_count > 0 {
                                                        format!(" · {hidden_count} live/bloat editions hidden")
                                                    } else {
                                                        String::new()
                                                    }
                                                )),
                                        ),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .child(
                                    Button::new("disc-follow-sync")
                                        .primary()
                                        .small()
                                        .icon(IconName::Download)
                                        .label(if is_followed {
                                            "Sync Discography"
                                        } else {
                                            "Follow & Sync All"
                                        })
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            let s = art_sum.clone();
                                            core_follow.update(cx, |c, cx| {
                                                c.follow_artist_summary(s, cx)
                                            });
                                        })),
                                )
                                .when(is_followed, |r| {
                                    r.child(
                                        Button::new("disc-unfollow")
                                            .ghost()
                                            .small()
                                            .label("Unfollow")
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                let bid = art_bid.clone();
                                                core_unfollow.update(cx, |c, cx| {
                                                    c.unfollow_artist(&bid, cx)
                                                });
                                            })),
                                    )
                                }),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg_muted))
                                .child(format!("STUDIO ALBUMS ({})", clean_albums.len())),
                        )
                        .children(clean_albums.into_iter().enumerate().map(|(ix, alb)| {
                            render_release_row(win, "disc-alb", ix, alb, &art_cache, &pal, cx)
                        }))
                        .when(!clean_singles.is_empty(), |col| {
                            col.child(
                                div()
                                    .mt_2()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(hsla(pal.fg_muted))
                                    .child(format!("SINGLES & EPs ({})", clean_singles.len())),
                            )
                            .children(clean_singles.into_iter().enumerate().map(|(ix, sng)| {
                                render_release_row(win, "disc-sng", ix, sng, &art_cache, &pal, cx)
                            }))
                        }),
                )
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1p5()
                .overflow_y_scrollbar()
                .when(results_with_art.is_empty() && !loading, |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Search any artist (e.g. 'Metallica', 'Linkin Park', 'Poppy', 'Enter Shikari') to inspect their clean studio discography and Follow & Auto-Sync all albums and singles into ~/Music/<Artist>/."),
                    )
                })
                .children(results_with_art.into_iter().enumerate().map(|(ix, (art, avatar))| {
                    let is_followed = followed_list
                        .iter()
                        .any(|f| f.browse_id == art.browse_id);
                    let core_open = win.core.clone();
                    let core_follow = win.core.clone();
                    let art_open = art.clone();
                    let art_follow = art.clone();

                    div()
                        .id(SharedString::from(format!("art-res-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2p5()
                        .rounded(px(7.))
                        .cursor_pointer()
                        .bg(hsla(pal.bg_sidebar))
                        .border_1()
                        .border_color(hsla(pal.border_subtle))
                        .hover(|s| s.bg(hsla(pal.bg_panel)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let a = art_open.clone();
                            core_open.update(cx, |c, cx| c.select_online_artist(a, cx));
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
                                            avatar.as_ref(),
                                            46.,
                                            23.,
                                            IconName::Library,
                                            &art.name,
                                            false,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(hsla(pal.fg))
                                                        .truncate()
                                                        .child(art.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .truncate()
                                                        .child(art.subtitle.clone()),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .child(
                                            Button::new(SharedString::from(format!("art-flw-{ix}")))
                                                .with_variant(if is_followed {
                                                    ButtonVariant::Secondary
                                                } else {
                                                    ButtonVariant::Primary
                                                })
                                                .xsmall()
                                                .icon(if is_followed {
                                                    IconName::RefreshCw
                                                } else {
                                                    IconName::Plus
                                                })
                                                .label(if is_followed {
                                                    "Following · Sync"
                                                } else {
                                                    "Follow & Sync"
                                                })
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let a = art_follow.clone();
                                                    core_follow.update(cx, |c, cx| {
                                                        c.follow_artist_summary(a, cx)
                                                    });
                                                })),
                                        ),
                                ),
                        )
                }))
                .into_any_element()
        })
}

fn render_release_row(
    win: &VibesWindow,
    prefix: &str,
    ix: usize,
    rel: crate::albums::AlbumSummary,
    art_cache: &std::collections::HashMap<String, std::path::PathBuf>,
    pal: &crate::theme::Palette,
    cx: &mut Context<VibesWindow>,
) -> gpui_kit::AnyElement {
    let cover = rel.cover_url.as_deref().and_then(|u| art_cache.get(u));
    let core_open = win.core.clone();
    let core_dl = win.core.clone();
    let rel_open = rel.clone();
    let rel_dl = rel.clone();
    let sub = match rel.year {
        Some(y) => format!("{} · {y}", rel.release_type),
        None => rel.release_type.clone(),
    };

    div()
        .id(SharedString::from(format!("{prefix}-{ix}")))
        .w_full()
        .px_3()
        .py_2()
        .rounded(px(7.))
        .cursor_pointer()
        .bg(hsla(pal.bg_sidebar))
        .border_1()
        .border_color(hsla(pal.border_subtle))
        .hover(|s| s.bg(hsla(pal.bg_panel)))
        .on_click(cx.listener(move |_, _, _, cx| {
            let s = rel_open.clone();
            core_open.update(cx, |c, cx| {
                c.discover_tab = DiscoverTab::Albums;
                c.select_album(s, cx);
            });
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
                            cover,
                            42.,
                            6.,
                            IconName::Disc3,
                            &rel.title,
                            false,
                            pal,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(hsla(pal.fg))
                                        .truncate()
                                        .child(rel.title),
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
                    Button::new(SharedString::from(format!("{prefix}-dl-{ix}")))
                        .secondary()
                        .xsmall()
                        .icon(IconName::Download)
                        .label("Download")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            let s = rel_dl.clone();
                            core_dl.update(cx, |c, cx| c.quick_download_album(s, cx));
                        })),
                ),
        )
        .into_any_element()
}
