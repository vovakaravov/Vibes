use super::{VibesWindow, art};
use crate::albums::{AlbumDetails, AlbumDownloadStatus};
use crate::config::PlayTarget;
use crate::local_library::{find_local_album_dir, find_local_track_in_dir};
use crate::theme::{hsla, rgb_with_alpha};
use crate::youtube::YoutubeItem;
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

pub fn render_albums_view(
    win: &mut VibesWindow,
    _window: &mut Window,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (
        pal,
        results_with_art,
        selected_with_art,
        search_loading,
        detail_loading,
        err_msg,
        active_vid_url,
        active_local_path,
        album_downloads,
        albums_dir,
        folder_mode,
        audio_fmt,
    ) = {
        let core = win.core.read(cx);
        let (active_url, active_local) = match &core.current_target {
            Some(PlayTarget::Youtube(y)) => (Some(y.url.clone()), None),
            Some(PlayTarget::Local(t)) => (None, Some(t.path.clone())),
            _ => (None, None),
        };
        let res_art: Vec<(crate::albums::AlbumSummary, Option<PathBuf>)> = core
            .album_results
            .iter()
            .map(|s| {
                let art = s
                    .cover_url
                    .as_deref()
                    .and_then(|u| core.art_path_for_url(u).cloned())
                    .or_else(|| {
                        core.local_library
                            .albums
                            .iter()
                            .find(|a| {
                                a.title.eq_ignore_ascii_case(&s.title)
                                    && a.artist.eq_ignore_ascii_case(&s.artist)
                            })
                            .and_then(|a| a.cover_path.clone())
                    });
                (s.clone(), art)
            })
            .collect();
        let sel_art = core.selected_album.as_ref().map(|d| {
            let art = d
                .cover_url
                .as_deref()
                .or(d.summary.cover_url.as_deref())
                .and_then(|u| core.art_path_for_url(u).cloned())
                .or_else(|| {
                    core.local_library
                        .albums
                        .iter()
                        .find(|a| {
                            a.title.eq_ignore_ascii_case(&d.summary.title)
                                && a.artist.eq_ignore_ascii_case(&d.summary.artist)
                        })
                        .and_then(|a| a.cover_path.clone())
                });
            (d.clone(), art)
        });
        (
            core.palette.clone(),
            res_art,
            sel_art,
            core.album_loading,
            core.album_detail_loading,
            core.album_error.clone(),
            active_url,
            active_local,
            core.album_downloads.clone(),
            core.config.albums_dir.clone(),
            core.config.album_folder_mode,
            core.config.save_audio_format,
        )
    };

    let core_search = win.core.clone();
    let dest_preview = format!(
        "{} · {} · .{}",
        albums_dir.display(),
        folder_mode.label(),
        audio_fmt.extension()
    );

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
                                .child("Albums · Full Album Search & Download"),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded(px(4.))
                        .bg(hsla(pal.bg_panel))
                        .text_size(px(10.5))
                        .text_color(hsla(pal.aqua))
                        .truncate()
                        .child(dest_preview),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div().flex_1().child(
                        Input::new(&win.album_search_input)
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
                    Button::new("album-search-go")
                        .primary()
                        .small()
                        .label(if search_loading {
                            "Searching…"
                        } else {
                            "Search Albums"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let q = this.album_search_input.read(cx).value().to_string();
                            core_search.update(cx, |c, cx| c.search_albums_query(q, cx));
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
                .child("Loading official tracklist & album artwork…")
                .into_any_element()
        } else if let Some((details, detail_cover)) = selected_with_art {
            render_album_detail(
                win,
                details,
                detail_cover,
                active_vid_url,
                active_local_path,
                &album_downloads,
                &albums_dir,
                &pal,
                cx,
            )
            .into_any_element()
        } else {
            render_album_results(win, results_with_art, search_loading, &pal, cx)
                .into_any_element()
        })
}

fn render_album_results(
    win: &VibesWindow,
    results: Vec<(crate::albums::AlbumSummary, Option<PathBuf>)>,
    loading: bool,
    pal: &crate::theme::Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1p5()
        .overflow_y_scrollbar()
        .when(results.is_empty() && !loading, |list| {
            list.child(
                div()
                    .p_6()
                    .text_xs()
                    .text_color(hsla(pal.fg_muted))
                    .child("Search any artist or album title (e.g. 'Gorillaz Demon Days', 'Hybrid Theory', 'Pendulum In Silico') or paste a YouTube Music playlist link to browse tracks, stream seamlessly, or download the complete tagged album with square cover art."),
            )
        })
        .children(results.into_iter().enumerate().map(|(ix, (summary, cover_path))| {
            let core_open = win.core.clone();
            let core_open_btn = win.core.clone();
            let core_dl = win.core.clone();
            let sum_open = summary.clone();
            let sum_open_btn = summary.clone();
            let sum_dl = summary.clone();
            let sub_line = match summary.year {
                Some(y) => format!("{} · {} · {y}", summary.artist, summary.release_type),
                None => format!("{} · {}", summary.artist, summary.release_type),
            };

            div()
                .id(SharedString::from(format!("album-res-{ix}")))
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
                    let s = sum_open.clone();
                    core_open.update(cx, |c, cx| c.select_album(s, cx));
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
                                    cover_path.as_ref(),
                                    52.,
                                    7.,
                                    IconName::Disc3,
                                    &summary.title,
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
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(hsla(pal.fg))
                                                .truncate()
                                                .child(summary.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(hsla(pal.fg_muted))
                                                .truncate()
                                                .child(sub_line),
                                        ),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .flex_shrink_0()
                                .child(
                                    Button::new(SharedString::from(format!("alb-view-{ix}")))
                                        .secondary()
                                        .xsmall()
                                        .label("Tracklist")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let s = sum_open_btn.clone();
                                            core_open_btn.update(cx, |c, cx| c.select_album(s, cx));
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("alb-dl-{ix}")))
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::Download)
                                        .label("Download Album")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            let s = sum_dl.clone();
                                            core_dl.update(cx, |c, cx| c.quick_download_album(s, cx));
                                        })),
                                ),
                        ),
                )
        }))
}

#[allow(clippy::too_many_arguments)]
fn render_album_detail(
    win: &VibesWindow,
    details: AlbumDetails,
    detail_cover: Option<PathBuf>,
    active_vid_url: Option<String>,
    active_local_path: Option<PathBuf>,
    album_downloads: &[crate::albums::AlbumDownloadJob],
    albums_dir: &std::path::Path,
    pal: &crate::theme::Palette,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let pal = pal.clone();
    let core_back = win.core.clone();
    let core_play_all = win.core.clone();
    let core_dl_all = win.core.clone();
    let core_open_dir = win.core.clone();

    let det_play = details.clone();
    let det_dl = details.clone();

    let matching_job = album_downloads.iter().find(|j| {
        j.title.eq_ignore_ascii_case(&details.summary.title)
            && j.artist.eq_ignore_ascii_case(&details.summary.artist)
    });

    let found_local_dir = find_local_album_dir(
        albums_dir,
        &details.summary.artist,
        &details.summary.title,
    );
    let local_matches: Vec<Option<PathBuf>> = details
        .tracks
        .iter()
        .map(|tr| {
            found_local_dir
                .as_deref()
                .and_then(|d| find_local_track_in_dir(d, tr.track_number, &tr.title))
        })
        .collect();
    let local_count = local_matches.iter().filter(|m| m.is_some()).count();
    let all_local = !details.tracks.is_empty() && local_count == details.tracks.len();

    let (dl_label, dl_busy) = match matching_job.map(|j| &j.status) {
        Some(AlbumDownloadStatus::Queued) => ("Queued…".to_string(), true),
        Some(AlbumDownloadStatus::Downloading {
            completed, total, ..
        }) => (format!("Saving {completed}/{total}…"), true),
        Some(AlbumDownloadStatus::Completed { .. }) => {
            ("Update / Re-Check Album".to_string(), false)
        }
        Some(AlbumDownloadStatus::Failed(_)) | None => {
            if found_local_dir.is_some() {
                ("Download / Sync Missing Tracks".to_string(), false)
            } else {
                ("Download Full Album".to_string(), false)
            }
        }
    };

    let dur_tot = details.total_duration_text();
    let base_subtitle = match (details.summary.year, dur_tot.is_empty()) {
        (Some(y), false) => format!(
            "{} · {} · {y} · {} tracks · {dur_tot}",
            details.summary.artist,
            details.summary.release_type,
            details.tracks.len()
        ),
        (Some(y), true) => format!(
            "{} · {} · {y} · {} tracks",
            details.summary.artist,
            details.summary.release_type,
            details.tracks.len()
        ),
        (None, false) => format!(
            "{} · {} · {} tracks · {dur_tot}",
            details.summary.artist,
            details.summary.release_type,
            details.tracks.len()
        ),
        (None, true) => format!(
            "{} · {} · {} tracks",
            details.summary.artist,
            details.summary.release_type,
            details.tracks.len()
        ),
    };
    let subtitle = if all_local {
        format!("{base_subtitle} · Plays Offline")
    } else if local_count > 0 {
        format!("{base_subtitle} · {local_count}/{} Local", details.tracks.len())
    } else {
        base_subtitle
    };

    let album_name = details.summary.title.clone();
    let tracks_snapshot = details.tracks.clone();

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
                            Button::new("album-back-btn")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowLeft)
                                .label("Results")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_back.update(cx, |c, cx| c.clear_selected_album(cx));
                                })),
                        )
                        .child(art::render_art_tile(
                            detail_cover.as_ref(),
                            84.,
                            8.,
                            IconName::Disc3,
                            &album_name,
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
                                        .child(album_name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(hsla(pal.fg_muted))
                                        .truncate()
                                        .child(subtitle),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .flex_shrink_0()
                        .child(
                            Button::new("album-play-all")
                                .secondary()
                                .small()
                                .icon(IconName::Play)
                                .label(if all_local {
                                    "Play Offline"
                                } else {
                                    "Play Album"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    let d = det_play.clone();
                                    core_play_all
                                        .update(cx, |c, cx| c.play_album_from_track(&d, 0, cx));
                                })),
                        )
                        .child(
                            Button::new("album-dl-all")
                                .with_variant(if dl_busy {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Success
                                })
                                .small()
                                .icon(IconName::Download)
                                .label(dl_label)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    if !dl_busy {
                                        let d = det_dl.clone();
                                        core_dl_all
                                            .update(cx, |c, cx| c.enqueue_album_download(d, cx));
                                    }
                                })),
                        )
                        .when_some(found_local_dir.clone(), |r, f_open| {
                            r.child(
                                Button::new("album-open-dir")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Folder)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_open_dir.read(cx).open_folder(&f_open);
                                    })),
                            )
                        }),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .overflow_y_scrollbar()
                .children(tracks_snapshot.into_iter().enumerate().map(|(ix, tr)| {
                    let local_match = local_matches.get(ix).and_then(|m| m.as_ref());
                    let on_disk = local_match.is_some();
                    let is_active = active_vid_url.as_deref() == Some(tr.url.as_str())
                        || (on_disk && local_match == active_local_path.as_ref());

                    let core_play = win.core.clone();
                    let core_save = win.core.clone();
                    let det_for_row = details.clone();
                    let tr_artist = tr.artist.clone();
                    let yt_single = YoutubeItem {
                        id: tr.video_id.clone(),
                        title: tr.title.clone(),
                        channel: tr.artist.clone(),
                        duration_secs: tr.duration_secs,
                        url: tr.url.clone(),
                        is_live: false,
                        view_count: None,
                    };

                    div()
                        .id(SharedString::from(format!("alb-trk-{ix}")))
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
                            let d = det_for_row.clone();
                            core_play.update(cx, |c, cx| c.play_album_from_track(&d, ix, cx));
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
                                                .w(px(24.))
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
                                                        .child(format!("{:02}", tr.track_number))
                                                        .into_any_element()
                                                }),
                                        )
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
                                                .when(
                                                    !tr_artist
                                                        .eq_ignore_ascii_case(&details.summary.artist),
                                                    |col| {
                                                        col.child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(hsla(pal.fg_muted))
                                                                .truncate()
                                                                .child(tr_artist),
                                                        )
                                                    },
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .flex_shrink_0()
                                        .when(on_disk, |r| {
                                            r.child(
                                                div()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded(px(4.))
                                                    .bg(rgb_with_alpha(pal.green, 0x26))
                                                    .text_size(px(9.5))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(hsla(pal.green))
                                                    .child("LOCAL"),
                                            )
                                            .child(
                                                Icon::new(IconName::Check)
                                                    .xsmall()
                                                    .text_color(hsla(pal.green)),
                                            )
                                        })
                                        .when(!tr.duration_text.is_empty(), |r| {
                                            r.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(hsla(pal.fg_muted))
                                                    .child(tr.duration_text),
                                            )
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "alb-trk-save-{ix}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Download)
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.stop_propagation();
                                                let it = yt_single.clone();
                                                core_save
                                                    .update(cx, |c, cx| c.save_youtube_item(&it, cx));
                                            })),
                                        ),
                                ),
                        )
                })),
        )
}
