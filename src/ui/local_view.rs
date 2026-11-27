use super::{VibesWindow, art};
use crate::config::{PlayTarget, QueueItem};
use crate::core::{AlbumSortMode, NavSection};
use crate::local_library::{LocalAlbum, LocalArtist};
use crate::theme::{Palette, hsla, rgb_with_alpha};
use crate::youtube::format_duration;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use std::path::PathBuf;

/// Primary Author-First Library view (`Artists`).
pub fn render_artists_view(
    win: &mut VibesWindow,
    _window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (
        pal,
        snapshot,
        loading,
        filter_query,
        selected_artist_key,
        selected_album_folder,
        albums_dir,
        active_local_path,
        lyrics_backfill,
        artist_sync,
        followed_artists,
        liked_tracks,
        first_playlist_id,
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
            core.selected_local_artist.clone(),
            core.selected_local_album.clone(),
            core.config.albums_dir.clone(),
            active_path,
            core.lyrics_backfill.clone(),
            core.artist_sync.clone(),
            core.state.followed_artists.clone(),
            core.state.liked_tracks.clone(),
            core.state.playlists.first().map(|p| (p.id.clone(), p.name.clone())),
        )
    };

    let core_refresh = win.core.clone();
    let core_open = win.core.clone();
    let core_backfill = win.core.clone();
    let core_sync_all = win.core.clone();
    let core_disc = win.core.clone();
    let open_root = albums_dir.clone();

    let backfill_active = lyrics_backfill.active;
    let show_backfill_banner = backfill_active || lyrics_backfill.processed > 0;
    let backfill_summary = lyrics_backfill.summary();
    let sync_active = artist_sync.is_active();
    let sync_summary = artist_sync.summary_line();
    let show_sync_banner = sync_active || !sync_summary.is_empty();

    let total_artists = snapshot.artists.len();
    let total_tracks: usize = snapshot.artists.iter().map(|a| a.total_tracks).sum();
    let followed_count = followed_artists.len();
    let stats_text = format!(
        "{total_artists} artists · {followed_count} followed · {total_tracks} tracks"
    );

    let selected_artist = selected_artist_key
        .as_ref()
        .and_then(|k| snapshot.artists.iter().find(|a| &a.key == k).cloned());
    let selected_album = selected_album_folder
        .as_ref()
        .and_then(|f| snapshot.albums.iter().find(|a| &a.folder == f).cloned());

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
                                .child("Artists"),
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
                            Button::new("art-sync-all-btn")
                                .with_variant(if sync_active {
                                    ButtonVariant::Danger
                                } else {
                                    ButtonVariant::Primary
                                })
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .label(if sync_active {
                                    "Cancel Sync"
                                } else {
                                    "Sync Followed"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_sync_all.update(cx, |c, cx| {
                                        if c.artist_sync.is_active() {
                                            c.cancel_artist_sync(cx);
                                        } else {
                                            c.sync_all_followed_artists(cx);
                                        }
                                    });
                                })),
                        )
                        .child(
                            Button::new("art-discover-btn")
                                .secondary()
                                .xsmall()
                                .icon(IconName::Plus)
                                .label("Follow Artist")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_disc.update(cx, |c, cx| {
                                        c.section = NavSection::Discover;
                                        c.discover_tab = crate::core::DiscoverTab::Artists;
                                        cx.notify();
                                    });
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
                            Button::new("local-refresh-btn")
                                .ghost()
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .label(if loading { "Scanning…" } else { "Refresh" })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_refresh.update(cx, |c, cx| c.refresh_local_library(cx));
                                })),
                        )
                        .child(
                            Button::new("local-open-music-btn")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Folder)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_open.read(cx).open_folder(&open_root);
                                })),
                        ),
                ),
        )
        .when(show_sync_banner, |col| {
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
                        Icon::new(IconName::Download)
                            .xsmall()
                            .text_color(hsla(if sync_active { pal.yellow } else { pal.green })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(hsla(pal.fg_subtle))
                            .child(sync_summary),
                    ),
            )
        })
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
        .when(selected_artist.is_none(), |col| {
            col.child(
                div().w_full().child(
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
        })
        .child(if let Some(album) = selected_album {
            let back_label = selected_artist
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Albums".to_string());
            render_local_album_detail(
                win,
                album,
                &back_label,
                active_local_path.as_ref(),
                &liked_tracks,
                first_playlist_id.as_ref(),
                &pal,
                cx,
            )
            .into_any_element()
        } else if let Some(artist) = selected_artist {
            render_artist_detail(
                win,
                artist,
                active_local_path.as_ref(),
                &followed_artists,
                &liked_tracks,
                first_playlist_id.as_ref(),
                &pal,
                cx,
            )
            .into_any_element()
        } else {
            render_artists_grid(
                win,
                &snapshot.artists,
                &filter_query,
                loading,
                active_local_path.as_ref(),
                &followed_artists,
                &pal,
                cx,
            )
            .into_any_element()
        })
}

/// Dedicated flat multi-track Albums view (`Albums`) with sorting and filtering.
pub fn render_local_albums_view(
    win: &mut VibesWindow,
    window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (
        pal,
        mut albums,
        loading,
        filter_query,
        sort_mode,
        selected_folder,
        active_local_path,
        lyrics_open,
        liked_tracks,
        first_playlist_id,
    ) = {
        let core = win.core.read(cx);
        let active_path = match &core.current_target {
            Some(PlayTarget::Local(t)) => Some(t.path.clone()),
            _ => None,
        };
        (
            core.palette.clone(),
            core.local_library.albums.clone(),
            core.local_library_loading,
            core.local_filter_query.clone(),
            core.album_sort,
            core.selected_local_album.clone(),
            active_path,
            core.lyrics_open,
            core.state.liked_tracks.clone(),
            core.state.playlists.first().map(|p| (p.id.clone(), p.name.clone())),
        )
    };

    match sort_mode {
        AlbumSortMode::Artist => albums.sort_by(|a, b| {
            a.artist
                .to_lowercase()
                .cmp(&b.artist.to_lowercase())
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        }),
        AlbumSortMode::Title => {
            albums.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        }
        AlbumSortMode::YearDesc => albums.sort_by(|a, b| {
            b.year
                .unwrap_or(0)
                .cmp(&a.year.unwrap_or(0))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        }),
        AlbumSortMode::TrackCount => albums.sort_by(|a, b| {
            b.tracks
                .len()
                .cmp(&a.tracks.len())
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        }),
    }

    let selected_album = selected_folder
        .as_ref()
        .and_then(|f| albums.iter().find(|a| &a.folder == f).cloned());
    let total_albums = albums.len();

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
                        .child(Icon::new(IconName::Disc3).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child(format!("Studio Albums ({total_albums})")),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .children(AlbumSortMode::ALL.into_iter().map(|m| {
                            let active = m == sort_mode;
                            let core_s = win.core.clone();
                            Button::new(SharedString::from(format!("alb-sort-{}", m.label())))
                                .with_variant(if active {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(m.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_s.update(cx, |c, cx| c.set_album_sort(m, cx));
                                }))
                        })),
                ),
        )
        .when(selected_album.is_none(), |col| {
            col.child(
                div().w_full().child(
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
        })
        .child(if let Some(album) = selected_album {
            render_local_album_detail(
                win,
                album,
                "Albums",
                active_local_path.as_ref(),
                &liked_tracks,
                first_playlist_id.as_ref(),
                &pal,
                cx,
            )
            .into_any_element()
        } else {
            render_local_albums_list(
                win,
                &albums,
                &filter_query,
                loading,
                active_local_path.as_ref(),
                lyrics_open,
                &pal,
                window,
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn render_artists_grid(
    win: &VibesWindow,
    artists: &[LocalArtist],
    filter_query: &str,
    loading: bool,
    active_local_path: Option<&PathBuf>,
    followed_artists: &[crate::config::FollowedArtist],
    pal: &Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let q = filter_query.trim().to_lowercase();
    let filtered: Vec<LocalArtist> = if q.is_empty() {
        artists.to_vec()
    } else {
        artists
            .iter()
            .filter(|a| {
                a.name.to_lowercase().contains(&q)
                    || a.albums.iter().any(|alb| alb.title.to_lowercase().contains(&q))
                    || a.singles.iter().any(|s| s.title.to_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    };

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1p5()
        .overflow_y_scrollbar()
        .when(filtered.is_empty(), |col| {
            col.child(
                div()
                    .p_6()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child(if loading {
                        "Scanning ~/Music by artist…"
                    } else if artists.is_empty() {
                        "No local music yet. Use Search & Follow to follow an artist and auto-sync their clean discography."
                    } else {
                        "No artists match your filter."
                    }),
            )
        })
        .children(filtered.into_iter().enumerate().map(|(ix, artist)| {
            let is_playing_artist = active_local_path.is_some_and(|p| {
                artist.albums.iter().any(|a| a.tracks.iter().any(|t| &t.path == p))
                    || artist.singles.iter().any(|s| &s.path == p)
            });
            let is_followed = followed_artists.iter().any(|f| {
                crate::discography::normalize_artist_key(&f.name) == artist.key
            });
            let core_open = win.core.clone();
            let core_play = win.core.clone();
            let core_shuf = win.core.clone();
            let core_follow = win.core.clone();
            let key_open = artist.key.clone();
            let art_play = artist.clone();
            let art_shuf = artist.clone();
            let name_follow = artist.name.clone();

            let mut parts = Vec::new();
            if !artist.albums.is_empty() {
                parts.push(format!(
                    "{} {}",
                    artist.albums.len(),
                    if artist.albums.len() == 1 { "album" } else { "albums" }
                ));
            }
            if !artist.singles.is_empty() {
                parts.push(format!(
                    "{} {}",
                    artist.singles.len(),
                    if artist.singles.len() == 1 { "single" } else { "singles" }
                ));
            }
            parts.push(format!("{} tracks", artist.total_tracks));
            let sub = parts.join(" · ");

            div()
                .id(SharedString::from(format!("loc-artist-{ix}")))
                .w_full()
                .px_3()
                .py_2p5()
                .rounded(px(8.))
                .cursor_pointer()
                .bg(if is_playing_artist {
                    rgb_with_alpha(pal.accent, 0x20)
                } else {
                    hsla(pal.bg_sidebar)
                })
                .border_1()
                .border_color(hsla(if is_playing_artist {
                    pal.accent
                } else {
                    pal.border_subtle
                }))
                .hover(|s| s.bg(hsla(pal.bg_panel)))
                .on_click(cx.listener(move |_, _, _, cx| {
                    let k = key_open.clone();
                    core_open.update(cx, |c, cx| c.select_local_artist(Some(k), cx));
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
                                .gap_3()
                                .child(art::render_art_tile(
                                    artist.cover_path.as_ref(),
                                    52.,
                                    26.,
                                    IconName::Library,
                                    &artist.name,
                                    is_playing_artist,
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
                                                        .text_sm()
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(hsla(if is_playing_artist {
                                                            pal.accent
                                                        } else {
                                                            pal.fg
                                                        }))
                                                        .truncate()
                                                        .child(artist.name),
                                                )
                                                .when(is_followed, |r| {
                                                    r.child(
                                                        div()
                                                            .px_1p5()
                                                            .py_0p5()
                                                            .rounded(px(4.))
                                                            .bg(rgb_with_alpha(pal.green, 0x26))
                                                            .text_size(px(9.5))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(hsla(pal.green))
                                                            .child("FOLLOWING"),
                                                    )
                                                }),
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
                                    Button::new(SharedString::from(format!("loc-art-play-{ix}")))
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::Play)
                                        .label("Play")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let a = art_play.clone();
                                            core_play.update(cx, |c, cx| {
                                                c.play_artist_all(&a, false, cx)
                                            });
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("loc-art-shuf-{ix}")))
                                        .secondary()
                                        .xsmall()
                                        .icon(IconName::Shuffle)
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let a = art_shuf.clone();
                                            core_shuf.update(cx, |c, cx| {
                                                c.play_artist_all(&a, true, cx)
                                            });
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("loc-art-flw-{ix}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(if is_followed {
                                            IconName::RefreshCw
                                        } else {
                                            IconName::Plus
                                        })
                                        .label(if is_followed { "Sync" } else { "Follow" })
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let n = name_follow.clone();
                                            core_follow.update(cx, |c, cx| {
                                                c.follow_local_artist(n, cx)
                                            });
                                        })),
                                ),
                        ),
                )
        }))
}

#[allow(clippy::too_many_arguments)]
fn render_artist_detail(
    win: &VibesWindow,
    artist: LocalArtist,
    active_local_path: Option<&PathBuf>,
    followed_artists: &[crate::config::FollowedArtist],
    liked_tracks: &[PathBuf],
    first_playlist: Option<&(String, String)>,
    pal: &Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let core_back = win.core.clone();
    let core_play_all = win.core.clone();
    let core_shuf_all = win.core.clone();
    let core_follow = win.core.clone();
    let core_unfollow = win.core.clone();
    let core_dir = win.core.clone();

    let followed_entry = followed_artists
        .iter()
        .find(|f| crate::discography::normalize_artist_key(&f.name) == artist.key)
        .cloned();
    let is_followed = followed_entry.is_some();
    let art_play = artist.clone();
    let art_shuf = artist.clone();
    let art_name_flw = artist.name.clone();
    let open_dir = artist.folder.clone();
    let unfollow_key = followed_entry
        .as_ref()
        .map(|f| f.browse_id.clone())
        .unwrap_or_else(|| artist.name.clone());

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
                            Button::new("loc-art-back")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowLeft)
                                .label("Artists")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_back.update(cx, |c, cx| c.select_local_artist(None, cx));
                                })),
                        )
                        .child(art::render_art_tile(
                            artist.cover_path.as_ref(),
                            72.,
                            36.,
                            IconName::Library,
                            &artist.name,
                            false,
                            &pal,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(hsla(pal.fg))
                                        .truncate()
                                        .child(artist.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(hsla(pal.fg_muted))
                                        .truncate()
                                        .child(format!(
                                            "{} albums · {} singles · {} tracks",
                                            artist.albums.len(),
                                            artist.singles.len(),
                                            artist.total_tracks
                                        )),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .flex_shrink_0()
                        .child(
                            Button::new("loc-art-det-play")
                                .primary()
                                .small()
                                .icon(IconName::Play)
                                .label("Play All")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let a = art_play.clone();
                                    core_play_all.update(cx, |c, cx| c.play_artist_all(&a, false, cx));
                                })),
                        )
                        .child(
                            Button::new("loc-art-det-shuf")
                                .secondary()
                                .small()
                                .icon(IconName::Shuffle)
                                .label("Shuffle")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let a = art_shuf.clone();
                                    core_shuf_all.update(cx, |c, cx| c.play_artist_all(&a, true, cx));
                                })),
                        )
                        .child(
                            Button::new("loc-art-det-flw")
                                .secondary()
                                .small()
                                .icon(if is_followed {
                                    IconName::RefreshCw
                                } else {
                                    IconName::Plus
                                })
                                .label(if is_followed {
                                    "Sync Now"
                                } else {
                                    "Follow & Sync"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let n = art_name_flw.clone();
                                    core_follow.update(cx, |c, cx| c.follow_local_artist(n, cx));
                                })),
                        )
                        .when(is_followed, |r| {
                            r.child(
                                Button::new("loc-art-det-unflw")
                                    .ghost()
                                    .small()
                                    .label("Unfollow")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let k = unfollow_key.clone();
                                        core_unfollow.update(cx, |c, cx| c.unfollow_artist(&k, cx));
                                    })),
                            )
                        })
                        .child(
                            Button::new("loc-art-det-dir")
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
                .gap_2()
                .overflow_y_scrollbar()
                .when(!artist.albums.is_empty(), |col| {
                    col.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(hsla(pal.fg_muted))
                            .child(format!("ALBUMS & EPs ({})", artist.albums.len())),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .children(artist.albums.iter().enumerate().map(|(ix, alb)| {
                                let is_playing_album = active_local_path
                                    .is_some_and(|p| alb.tracks.iter().any(|t| &t.path == p));
                                let core_open = win.core.clone();
                                let core_play = win.core.clone();
                                let folder_open = alb.folder.clone();
                                let alb_play = alb.clone();
                                let meta = match alb.year {
                                    Some(y) => format!("{y} · {} trk", alb.tracks.len()),
                                    None => format!("{} trk", alb.tracks.len()),
                                };

                                div()
                                    .id(SharedString::from(format!("art-alb-{ix}")))
                                    .w(px(164.))
                                    .p_2()
                                    .rounded(px(9.))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
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
                                        146.,
                                        7.,
                                        IconName::Disc3,
                                        &alb.title,
                                        is_playing_album,
                                        &pal,
                                    ))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(hsla(if is_playing_album {
                                                pal.accent
                                            } else {
                                                pal.fg
                                            }))
                                            .truncate()
                                            .child(alb.title.clone()),
                                    )
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(hsla(pal.fg_muted))
                                                    .child(meta),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!("art-alb-play-{ix}")))
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
                .when(!artist.singles.is_empty(), |col| {
                    let sng_snapshot = artist.singles.clone();
                    col.child(
                        div()
                            .mt_2()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(hsla(pal.fg_muted))
                            .child(format!("SINGLES ({})", artist.singles.len())),
                    )
                    .children(artist.singles.into_iter().enumerate().map(|(ix, tr)| {
                        let is_active = active_local_path == Some(&tr.path);
                        let is_liked = liked_tracks.iter().any(|p| p == &tr.path);
                        let list_play = sng_snapshot.clone();
                        let core_play = win.core.clone();
                        let core_next = win.core.clone();
                        let core_q = win.core.clone();
                        let core_like = win.core.clone();
                        let core_pl = win.core.clone();
                        let tr_next = tr.clone();
                        let tr_q = tr.clone();
                        let path_like = tr.path.clone();
                        let path_pl = tr.path.clone();

                        div()
                            .id(SharedString::from(format!("art-sng-{ix}")))
                            .w_full()
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .bg(if is_active {
                                rgb_with_alpha(pal.accent, 0x24)
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
                                                32.,
                                                5.,
                                                IconName::Music,
                                                &tr.title,
                                                is_active,
                                                &pal,
                                            ))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(hsla(if is_active {
                                                        pal.accent
                                                    } else {
                                                        pal.fg
                                                    }))
                                                    .truncate()
                                                    .child(tr.title),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .flex_shrink_0()
                                            .when_some(tr.duration_secs, |r, d| {
                                                r.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .child(format_duration(d)),
                                                )
                                            })
                                            .child(
                                                Button::new(SharedString::from(format!("art-sng-like-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Icon::new(IconName::Star).xsmall().text_color(hsla(
                                                        if is_liked { pal.yellow } else { pal.fg_muted },
                                                    )))
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.stop_propagation();
                                                        let p = path_like.clone();
                                                        core_like.update(cx, |c, cx| c.toggle_like_track(p, cx));
                                                    })),
                                            )
                                            .when_some(first_playlist.cloned(), |r, (pid, _pname)| {
                                                r.child(
                                                    Button::new(SharedString::from(format!("art-sng-pl-{ix}")))
                                                        .ghost()
                                                        .xsmall()
                                                        .icon(IconName::ListMusic)
                                                        .on_click(cx.listener(move |_, _, _, cx| {
                                                            cx.stop_propagation();
                                                            let p = path_pl.clone();
                                                            core_pl.update(cx, |c, cx| {
                                                                c.add_track_to_playlist(&pid, p, cx)
                                                            });
                                                        })),
                                                )
                                            })
                                            .child(
                                                Button::new(SharedString::from(format!("art-sng-next-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .label("Next")
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.stop_propagation();
                                                        let t = tr_next.clone();
                                                        core_next.update(cx, |c, cx| {
                                                            c.queue_play_next(QueueItem::Local(t), cx)
                                                        });
                                                    })),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!("art-sng-q-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::Plus)
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.stop_propagation();
                                                        let t = tr_q.clone();
                                                        core_q.update(cx, |c, cx| c.queue_local_track(t, cx));
                                                    })),
                                            ),
                                    ),
                            )
                    }))
                }),
        )
}

fn compute_album_grid_cols(viewport_width: f32, lyrics_open: bool) -> usize {
    const SIDEBAR_WIDTH: f32 = 210.0;
    const LYRICS_WIDTH: f32 = 320.0;
    const OUTER_PAD_X: f32 = 32.0;
    const SCROLLBAR_PAD: f32 = 12.0;
    const CARD_WIDTH: f32 = 172.0;
    const CARD_GAP: f32 = 12.0;

    let used = SIDEBAR_WIDTH
        + if lyrics_open { LYRICS_WIDTH } else { 0.0 }
        + OUTER_PAD_X
        + SCROLLBAR_PAD;
    let avail = (viewport_width - used).max(CARD_WIDTH);
    (((avail + CARD_GAP) / (CARD_WIDTH + CARD_GAP)).floor() as usize).max(1)
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

#[allow(clippy::too_many_arguments)]
fn render_local_albums_list(
    win: &mut VibesWindow,
    albums: &[LocalAlbum],
    filter_query: &str,
    loading: bool,
    active_local_path: Option<&PathBuf>,
    lyrics_open: bool,
    pal: &Palette,
    window: &Window,
) -> AnyElement {
    let pal = pal.clone();
    if win.last_local_filter != filter_query {
        win.last_local_filter = filter_query.to_string();
        win.local_albums_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

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

    if filtered.is_empty() {
        return v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .p_6()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child(if loading {
                        "Scanning ~/Music for offline albums…"
                    } else if albums.is_empty() {
                        "No local album folders found yet. Follow an artist or download albums from Search & Follow."
                    } else {
                        "No local albums match your filter."
                    }),
            )
            .into_any_element();
    }

    let cols = compute_album_grid_cols(f32::from(window.viewport_size().width), lyrics_open);
    let row_count = filtered.len().div_ceil(cols);
    let core = win.core.clone();
    let active_local_path = active_local_path.cloned();
    let scroll_handle = win.local_albums_scroll.clone();

    div()
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            uniform_list("local-albums-grid", row_count, move |range, _window, _cx| {
                range
                    .map(|row_ix| {
                        let start = row_ix * cols;
                        let end = (start + cols).min(filtered.len());
                        h_flex()
                            .w_full()
                            .gap_3()
                            .pb_3()
                            .children((start..end).map(|ix| {
                                let alb = filtered[ix].clone();
                                let core_open = core.clone();
                                let core_play = core.clone();
                                let core_art = core.clone();
                                let folder_open = alb.folder.clone();
                                let alb_play = alb.clone();
                                let art_name = alb.artist.clone();
                                let is_playing_album = active_local_path
                                    .as_ref()
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
                                    .on_click(move |_, _, cx| {
                                        let f = folder_open.clone();
                                        core_open
                                            .update(cx, |c, cx| c.select_local_album(Some(f), cx));
                                    })
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
                                                    .id(SharedString::from(format!("loc-alb-art-{ix}")))
                                                    .text_xs()
                                                    .text_color(hsla(pal.fg_muted))
                                                    .hover(|s| s.text_color(hsla(pal.accent)))
                                                    .truncate()
                                                    .on_click(move |_, _, cx| {
                                                        cx.stop_propagation();
                                                        core_art.update(cx, |c, cx| {
                                                            c.jump_to_artist_by_name(&art_name, cx);
                                                        });
                                                    })
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
                                                .on_click(move |_, _, cx| {
                                                    cx.stop_propagation();
                                                    let a = alb_play.clone();
                                                    core_play.update(cx, |c, cx| {
                                                        c.play_local_album_from(&a, 0, cx)
                                                    });
                                                }),
                                            ),
                                    )
                            }))
                    })
                    .collect()
            })
            .size_full()
            .track_scroll(&scroll_handle),
        )
        .child(Scrollbar::vertical(&scroll_handle))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_local_album_detail(
    win: &VibesWindow,
    album: LocalAlbum,
    back_label: &str,
    active_local_path: Option<&PathBuf>,
    liked_tracks: &[PathBuf],
    first_playlist: Option<&(String, String)>,
    pal: &Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let core_back = win.core.clone();
    let core_play_all = win.core.clone();
    let core_shuf_all = win.core.clone();
    let core_dir = win.core.clone();
    let core_jump_art = win.core.clone();

    let alb_play = album.clone();
    let alb_shuf = album.clone();
    let open_dir = album.folder.clone();
    let sub = format_album_subtitle(&album);
    let alb_artist = album.artist.clone();
    let alb_artist_jump = album.artist.clone();
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
                                .label(back_label.to_string())
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
                                        .id("loc-det-jump-artist")
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(hsla(pal.fg_muted))
                                        .hover(|s| s.text_color(hsla(pal.accent)))
                                        .truncate()
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            core_jump_art.update(cx, |c, cx| {
                                                c.jump_to_artist_by_name(&alb_artist_jump, cx);
                                            });
                                        }))
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
                            Button::new("loc-detail-shuf")
                                .secondary()
                                .small()
                                .icon(IconName::Shuffle)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let a = alb_shuf.clone();
                                    core_shuf_all.update(cx, |c, cx| {
                                        c.play_tracks_shuffled(a.tracks, cx)
                                    });
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
                    let is_liked = liked_tracks.iter().any(|p| p == &tr.path);
                    let tr_num = tr.track_number.unwrap_or((ix + 1) as u32);
                    let show_artist = !tr.artist.eq_ignore_ascii_case(&alb_artist);
                    let core_play = win.core.clone();
                    let core_next = win.core.clone();
                    let core_q = win.core.clone();
                    let core_like = win.core.clone();
                    let core_pl = win.core.clone();
                    let alb_for_play = album.clone();
                    let tr_for_next = tr.clone();
                    let tr_for_q = tr.clone();
                    let path_like = tr.path.clone();
                    let path_pl = tr.path.clone();

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
                                        .gap_1p5()
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
                                            Button::new(SharedString::from(format!("loc-det-like-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(Icon::new(IconName::Star).xsmall().text_color(hsla(
                                                    if is_liked { pal.yellow } else { pal.fg_muted },
                                                )))
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let p = path_like.clone();
                                                    core_like.update(cx, |c, cx| c.toggle_like_track(p, cx));
                                                })),
                                        )
                                        .when_some(first_playlist.cloned(), |r, (pid, _pname)| {
                                            r.child(
                                                Button::new(SharedString::from(format!("loc-det-pl-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::ListMusic)
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.stop_propagation();
                                                        let p = path_pl.clone();
                                                        core_pl.update(cx, |c, cx| {
                                                            c.add_track_to_playlist(&pid, p, cx)
                                                        });
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!("loc-det-next-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .label("Next")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let t = tr_for_next.clone();
                                                    core_next.update(cx, |c, cx| {
                                                        c.queue_play_next(QueueItem::Local(t), cx)
                                                    });
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "loc-det-q-{ix}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Plus)
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

#[cfg(test)]
mod tests {
    use super::compute_album_grid_cols;

    #[test]
    fn compute_album_grid_cols_accounts_for_viewport_and_lyrics_panel() {
        assert_eq!(compute_album_grid_cols(0.0, false), 1);
        assert_eq!(compute_album_grid_cols(420.0, false), 1);
        assert_eq!(compute_album_grid_cols(630.0, false), 2);
        assert_eq!(compute_album_grid_cols(1100.0, false), 4);
        assert_eq!(compute_album_grid_cols(1280.0, false), 5);
        assert_eq!(compute_album_grid_cols(1920.0, false), 9);

        assert_eq!(compute_album_grid_cols(1100.0, true), 2);
        assert_eq!(compute_album_grid_cols(1280.0, true), 3);
        assert_eq!(compute_album_grid_cols(420.0, true), 1);
    }
}
