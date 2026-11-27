use super::{VibesWindow, art};
use crate::albums::AlbumDownloadStatus;
use crate::config::{AlbumFolderMode, SaveAudioFormat, SaveFolderMode};
use crate::core::{DiscoverTab, NavSection, QueueItem};
use crate::downloader::DownloadStatus;
use crate::theme::{ThemePreset, hsla, rgb_with_alpha};
use crate::youtube::format_duration;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    ClipboardItem, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px,
};
use std::path::PathBuf;

pub fn render_favorites_view(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, favorites) = {
        let core = win.core.read(cx);
        let list: Vec<(crate::radio::RadioStation, Option<PathBuf>)> = core
            .state
            .favorites
            .iter()
            .map(|st| (st.clone(), core.art_path_for_url(&st.favicon).cloned()))
            .collect();
        (core.palette.clone(), list)
    };
    let fav_len = favorites.len();

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
                        .child(Icon::new(IconName::Star).text_color(hsla(pal.yellow)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Favorite Stations & Stereo Presets"),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(hsla(pal.fg_muted))
                        .child("Keys 1-9 tune presets instantly"),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1p5()
                .overflow_y_scrollbar()
                .when(favorites.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Star any radio station to pin it here and in your KDE system tray Favorites menu."),
                    )
                })
                .children(favorites.into_iter().enumerate().map(|(ix, (st, fav_art))| {
                    let core_play = win.core.clone();
                    let core_up = win.core.clone();
                    let core_dn = win.core.clone();
                    let core_rm = win.core.clone();
                    let st_play = st.clone();
                    let st_rm = st.clone();
                    let preset_badge = if ix < 9 {
                        format!("[{}]", ix + 1)
                    } else {
                        format!("#{}", ix + 1)
                    };

                    div()
                        .id(SharedString::from(format!("fav-row-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .bg(hsla(pal.bg_sidebar))
                        .border_1()
                        .border_color(hsla(pal.border_subtle))
                        .hover(|s| s.bg(hsla(pal.bg_panel)))
                        .cursor_pointer()
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
                                        .gap_2p5()
                                        .items_center()
                                        .child(art::render_art_tile(
                                            fav_art.as_ref(),
                                            38.,
                                            6.,
                                            IconName::Radio,
                                            &st.name,
                                            false,
                                            &pal,
                                        ))
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(px(4.))
                                                .bg(hsla(pal.bg_btn))
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(hsla(pal.accent))
                                                .child(preset_badge),
                                        )
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
                                                        .child(st.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .truncate()
                                                        .child(st.display_codec()),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .when(ix > 0, |r| {
                                            r.child(
                                                Button::new(SharedString::from(format!("fav-up-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .label("Up")
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.stop_propagation();
                                                        core_up.update(cx, |c, cx| {
                                                            c.move_favorite(ix, -1, cx)
                                                        });
                                                    })),
                                            )
                                        })
                                        .when(ix + 1 < fav_len, |r| {
                                            r.child(
                                                Button::new(SharedString::from(format!("fav-dn-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .label("Down")
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.stop_propagation();
                                                        core_dn.update(cx, |c, cx| {
                                                            c.move_favorite(ix, 1, cx)
                                                        });
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!("fav-rm-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::X)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let s = st_rm.clone();
                                                    core_rm.update(cx, |c, cx| {
                                                        c.toggle_favorite_station(s, cx)
                                                    });
                                                })),
                                        ),
                                ),
                        )
                })),
        )
}

pub fn render_song_log_view(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, log) = {
        let core = win.core.read(cx);
        (core.palette.clone(), core.state.song_log.clone())
    };
    let core_clr = win.core.clone();

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
                        .child(Icon::new(IconName::Clock).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Radio Song Log"),
                        ),
                )
                .when(!log.is_empty(), |row| {
                    row.child(
                        Button::new("clear-song-log")
                            .ghost()
                            .xsmall()
                            .label("Clear History")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                core_clr.update(cx, |c, cx| c.clear_song_log(cx));
                            })),
                    )
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .overflow_y_scrollbar()
                .when(log.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Songs heard on internet radio stations appear here automatically. Save or search any track later with one click."),
                    )
                })
                .children(log.into_iter().enumerate().map(|(ix, entry)| {
                    let core_save = win.core.clone();
                    let core_yt = win.core.clone();
                    let core_toast = win.core.clone();
                    let trk_save = entry.track.clone();
                    let st_save = entry.station.clone();
                    let trk_yt = entry.track.clone();
                    let trk_cp = entry.track.clone();

                    div()
                        .id(SharedString::from(format!("log-row-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .bg(hsla(pal.bg_sidebar))
                        .border_1()
                        .border_color(hsla(pal.border_subtle))
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
                                            None,
                                            32.,
                                            6.,
                                            IconName::Music,
                                            &entry.track,
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
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(hsla(pal.fg))
                                                        .truncate()
                                                        .child(entry.track),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .truncate()
                                                        .child(format!(
                                                            "{} · {}",
                                                            entry.station, entry.timestamp
                                                        )),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .flex_shrink_0()
                                        .child(
                                            Button::new(SharedString::from(format!("log-save-{ix}")))
                                                .primary()
                                                .xsmall()
                                                .icon(IconName::Download)
                                                .label("Save Song")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    core_save.update(cx, |c, cx| {
                                                        c.save_song_from_text(&trk_save, &st_save, cx)
                                                    });
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("log-yt-{ix}")))
                                                .secondary()
                                                .xsmall()
                                                .icon(IconName::Headphones)
                                                .label("YT")
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    let q = trk_yt.clone();
                                                    this.yt_search_input.update(cx, |s, cx| {
                                                        s.set_value(&q, window, cx)
                                                    });
                                                    core_yt.update(cx, |c, cx| {
                                                        c.section = NavSection::Discover;
                                                        c.discover_tab = DiscoverTab::Tracks;
                                                        c.search_youtube_query(q, cx);
                                                    });
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("log-cp-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::Copy)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                                        trk_cp.clone(),
                                                    ));
                                                    core_toast.update(cx, |c, cx| {
                                                        c.show_toast(format!("Copied: {trk_cp}"), cx)
                                                    });
                                                })),
                                        ),
                                ),
                        )
                })),
        )
}

pub fn render_queue_view(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, queue, shuffle_enabled, repeat_mode) = {
        let core = win.core.read(cx);
        let q: Vec<(QueueItem, Option<PathBuf>)> = core
            .queue
            .iter()
            .map(|it| {
                let art = match it {
                    QueueItem::Local(t) => t.cover_path.clone(),
                    QueueItem::Youtube(y) => core.art_path_for_youtube(&y.id).cloned(),
                };
                (it.clone(), art)
            })
            .collect();
        (
            core.palette.clone(),
            q,
            core.state.shuffle_enabled,
            core.state.repeat_mode,
        )
    };
    let q_len = queue.len();
    let core_clr = win.core.clone();
    let core_shuf = win.core.clone();
    let core_rep = win.core.clone();
    let core_save_pl = win.core.clone();

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
                        .child(Icon::new(IconName::Play).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child(format!("Up Next ({q_len})")),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(
                            Button::new("q-tog-shuf")
                                .with_variant(if shuffle_enabled {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .icon(IconName::Shuffle)
                                .label(if shuffle_enabled { "Shuffle: On" } else { "Shuffle" })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_shuf.update(cx, |c, cx| c.toggle_shuffle(cx));
                                })),
                        )
                        .child(
                            Button::new("q-cyc-rep")
                                .with_variant(if repeat_mode != crate::config::RepeatMode::Off {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .label(format!("Repeat: {}", repeat_mode.label()))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_rep.update(cx, |c, cx| c.cycle_repeat_mode(cx));
                                })),
                        )
                        .when(q_len > 0, |row| {
                            row.child(
                                Button::new("q-save-pl")
                                    .secondary()
                                    .xsmall()
                                    .icon(IconName::ListMusic)
                                    .label("Save Playlist")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_save_pl.update(cx, |c, cx| {
                                            c.save_queue_as_playlist(String::new(), cx)
                                        });
                                    })),
                            )
                            .child(
                                Button::new("clear-queue-btn")
                                    .ghost()
                                    .xsmall()
                                    .label("Clear")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_clr.update(cx, |c, cx| c.clear_queue(cx));
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
                .when(queue.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Up Next queue is empty. Add tracks from Artists, Albums, Playlists, or Search & Follow."),
                    )
                })
                .children(queue.into_iter().enumerate().map(|(ix, (item, art_path))| {
                    let core_play = win.core.clone();
                    let core_up = win.core.clone();
                    let core_dn = win.core.clone();
                    let core_rm = win.core.clone();
                    let is_local = item.is_local();
                    let title_txt = item.title().to_string();
                    let sub_txt = item.subtitle();
                    let dur_txt = item
                        .duration_secs()
                        .map(format_duration)
                        .unwrap_or_else(|| if is_local { "LOCAL".to_string() } else { "AUDIO".to_string() });

                    div()
                        .id(SharedString::from(format!("q-row-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .bg(hsla(pal.bg_sidebar))
                        .border_1()
                        .border_color(hsla(pal.border_subtle))
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
                                            art_path.as_ref(),
                                            36.,
                                            6.,
                                            if is_local {
                                                IconName::Music
                                            } else {
                                                IconName::Headphones
                                            },
                                            &title_txt,
                                            false,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    h_flex()
                                                        .gap_1p5()
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_color(hsla(pal.fg))
                                                                .truncate()
                                                                .child(title_txt),
                                                        )
                                                        .when(is_local, |r| {
                                                            r.child(
                                                                div()
                                                                    .px_1p5()
                                                                    .py_0p5()
                                                                    .rounded(px(4.))
                                                                    .bg(rgb_with_alpha(
                                                                        pal.green, 0x26,
                                                                    ))
                                                                    .text_size(px(9.))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(hsla(pal.green))
                                                                    .child("LOCAL"),
                                                            )
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .truncate()
                                                        .child(format!("{sub_txt} · {dur_txt}")),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .when(ix > 0, |r| {
                                            r.child(
                                                Button::new(SharedString::from(format!("q-up-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::ChevronUp)
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        core_up.update(cx, |c, cx| {
                                                            c.move_queue_item(ix, -1, cx)
                                                        });
                                                    })),
                                            )
                                        })
                                        .when(ix + 1 < q_len, |r| {
                                            r.child(
                                                Button::new(SharedString::from(format!("q-dn-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::ChevronDown)
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        core_dn.update(cx, |c, cx| {
                                                            c.move_queue_item(ix, 1, cx)
                                                        });
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!("q-play-{ix}")))
                                                .primary()
                                                .xsmall()
                                                .icon(IconName::Play)
                                                .label("Play")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    core_play.update(cx, |c, cx| {
                                                        c.play_queue_index(ix, cx);
                                                    });
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("q-rm-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::X)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    core_rm.update(cx, |c, cx| {
                                                        c.remove_queue_index(ix, cx)
                                                    });
                                                })),
                                        ),
                                ),
                        )
                })),
        )
}

pub fn render_downloads_view(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, jobs, album_jobs, failed_items, artist_sync, albums_dir, fmt_label) = {
        let core = win.core.read(cx);
        let jobs_with_art: Vec<(crate::downloader::DownloadJob, Option<PathBuf>)> = core
            .downloads
            .iter()
            .map(|j| {
                let art = match &j.status {
                    DownloadStatus::Saved(p) | DownloadStatus::AlreadyExists(p) => core
                        .local_library
                        .singles
                        .iter()
                        .chain(core.local_library.albums.iter().flat_map(|a| a.tracks.iter()))
                        .find(|t| &t.path == p)
                        .and_then(|t| t.cover_path.clone()),
                    _ => None,
                };
                (j.clone(), art)
            })
            .collect();
        let alb_jobs_with_art: Vec<(crate::albums::AlbumDownloadJob, Option<PathBuf>)> = core
            .album_downloads
            .iter()
            .map(|j| {
                let art = j
                    .details
                    .cover_url
                    .as_deref()
                    .or(j.details.summary.cover_url.as_deref())
                    .and_then(|u| core.art_path_for_url(u).cloned())
                    .or_else(|| {
                        core.local_library
                            .albums
                            .iter()
                            .find(|a| a.folder == j.folder)
                            .and_then(|a| a.cover_path.clone())
                    });
                (j.clone(), art)
            })
            .collect();
        (
            core.palette.clone(),
            jobs_with_art,
            alb_jobs_with_art,
            core.state.failed_downloads.clone(),
            core.artist_sync.clone(),
            core.config.albums_dir.clone(),
            core.config.save_audio_format.label(),
        )
    };
    let core_open_alb = win.core.clone();
    let core_retry_all = win.core.clone();
    let core_clear_fail = win.core.clone();
    let core_cancel_sync = win.core.clone();
    let albums_root = albums_dir.clone();
    let sync_active = artist_sync.is_active();
    let sync_summary = artist_sync.summary_line();

    v_flex()
        .size_full()
        .p_4()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    v_flex()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(IconName::Download).text_color(hsla(pal.green)))
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(hsla(pal.fg))
                                        .child("Downloads, Sync & Recovery"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(pal.fg_muted))
                                .child(format!(
                                    "Format: {fmt_label} · Destination: {}/<Artist>/<Album or Singles>/",
                                    albums_dir.display()
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .when(!failed_items.is_empty(), |r| {
                            r.child(
                                Button::new("dl-retry-all-failed")
                                    .primary()
                                    .small()
                                    .icon(IconName::RefreshCw)
                                    .label(format!("Retry All Failed ({})", failed_items.len()))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_retry_all
                                            .update(cx, |c, cx| c.retry_all_failed_downloads(cx));
                                    })),
                            )
                            .child(
                                Button::new("dl-clear-failed")
                                    .ghost()
                                    .small()
                                    .label("Clear Failed")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        core_clear_fail
                                            .update(cx, |c, cx| c.clear_failed_downloads(cx));
                                    })),
                            )
                        })
                        .child(
                            Button::new("open-albums-folder")
                                .secondary()
                                .small()
                                .icon(IconName::Folder)
                                .label("Music Folder")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_open_alb.read(cx).open_folder(&albums_root);
                                })),
                        ),
                ),
        )
        .when(!sync_summary.is_empty(), |col| {
            col.child(
                h_flex()
                    .p_2p5()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .rounded(px(7.))
                    .bg(hsla(pal.bg_panel))
                    .border_1()
                    .border_color(hsla(if sync_active { pal.yellow } else { pal.border_subtle }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(hsla(if sync_active { pal.yellow } else { pal.green }))
                            .truncate()
                            .child(sync_summary),
                    )
                    .when(sync_active, |r| {
                        r.child(
                            Button::new("dl-cancel-sync")
                                .danger()
                                .xsmall()
                                .label("Stop Sync")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_cancel_sync.update(cx, |c, cx| c.cancel_artist_sync(cx));
                                })),
                        )
                    }),
            )
        })
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1p5()
                .overflow_y_scrollbar()
                .when(jobs.is_empty() && album_jobs.is_empty() && failed_items.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Follow artists or download albums & singles from Search & Follow, or press Ctrl+S while listening to Radio / YouTube. Any failed track stays tracked here so you never lose it."),
                    )
                })
                .when(!failed_items.is_empty(), |col| {
                    col.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(hsla(pal.red))
                            .child(format!("FAILED DOWNLOADS ({})", failed_items.len())),
                    )
                    .children(failed_items.into_iter().enumerate().map(|(ix, item)| {
                        let fid_retry = item.id.clone();
                        let fid_dismiss = item.id.clone();
                        let core_r = win.core.clone();
                        let core_d = win.core.clone();
                        let title_line = match &item.album {
                            Some(alb) => format!("{} — {} ({alb})", item.artist, item.title),
                            None => format!("{} — {}", item.artist, item.title),
                        };
                        div()
                            .id(SharedString::from(format!("dl-fail-{ix}")))
                            .w_full()
                            .px_3()
                            .py_2()
                            .rounded(px(7.))
                            .bg(rgb_with_alpha(pal.red, 0x16))
                            .border_1()
                            .border_color(rgb_with_alpha(pal.red, 0x55))
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .gap_3()
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
                                                    .child(title_line),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(hsla(pal.red))
                                                    .truncate()
                                                    .child(format!("{} · {}", item.failed_at, item.error)),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .flex_shrink_0()
                                            .child(
                                                Button::new(SharedString::from(format!("fail-retry-{ix}")))
                                                    .secondary()
                                                    .xsmall()
                                                    .icon(IconName::RefreshCw)
                                                    .label("Retry")
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        let id = fid_retry.clone();
                                                        core_r.update(cx, |c, cx| {
                                                            c.retry_failed_download(id, cx)
                                                        });
                                                    })),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!("fail-dim-{ix}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::X)
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        core_d.update(cx, |c, cx| {
                                                            c.dismiss_failed_download(&fid_dismiss, cx)
                                                        });
                                                    })),
                                            ),
                                    ),
                            )
                    }))
                })
                .children(album_jobs.into_iter().enumerate().map(|(ix, (job, job_art))| {
                    let job_id = job.id;
                    let can_retry = matches!(
                        job.status,
                        AlbumDownloadStatus::Failed(_)
                            | AlbumDownloadStatus::Completed {
                                failed: 1..,
                                ..
                            }
                    );
                    let (status_txt, status_col) = match &job.status {
                        AlbumDownloadStatus::Queued => {
                            ("Fetching official cover art & preparing album folder…".to_string(), pal.yellow)
                        }
                        AlbumDownloadStatus::Downloading {
                            completed,
                            total,
                            current_track,
                        } => (
                            format!("Downloading track {}/{}: {current_track}", completed + 1, total),
                            pal.accent,
                        ),
                        AlbumDownloadStatus::Completed {
                            saved,
                            skipped,
                            failed,
                            ..
                        } => (
                            format!(
                                "Album complete · {saved} saved, {skipped} skipped, {failed} failed"
                            ),
                            if *failed > 0 { pal.yellow } else { pal.green },
                        ),
                        AlbumDownloadStatus::Failed(e) => (format!("Failed: {e}"), pal.red),
                    };
                    let core_retry = win.core.clone();
                    let core_folder = win.core.clone();
                    let folder_path = job.folder.clone();
                    let title_line = match job.year {
                        Some(y) => format!("{} — {} ({y})", job.artist, job.title),
                        None => format!("{} — {}", job.artist, job.title),
                    };

                    div()
                        .id(SharedString::from(format!("alb-dl-row-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .bg(hsla(pal.bg_panel))
                        .border_1()
                        .border_color(hsla(pal.border))
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
                                            job_art.as_ref(),
                                            44.,
                                            6.,
                                            IconName::Disc3,
                                            &job.title,
                                            false,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_1()
                                                .child(
                                                    h_flex()
                                                        .justify_between()
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_weight(FontWeight::BOLD)
                                                                .text_color(hsla(pal.fg))
                                                                .truncate()
                                                                .child(title_line),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(10.5))
                                                                .text_color(hsla(pal.fg_muted))
                                                                .child(job.created_at),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(status_col))
                                                        .truncate()
                                                        .child(status_txt),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .flex_shrink_0()
                                        .when(can_retry, |r| {
                                            r.child(
                                                Button::new(SharedString::from(format!(
                                                    "alb-retry-{ix}"
                                                )))
                                                .secondary()
                                                .xsmall()
                                                .icon(IconName::RefreshCw)
                                                .label("Resume")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    core_retry.update(cx, |c, cx| {
                                                        c.retry_album_download(job_id, cx)
                                                    });
                                                })),
                                            )
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "alb-open-{ix}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Folder)
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                core_folder.read(cx).open_folder(&folder_path);
                                            })),
                                        ),
                                ),
                        )
                }))
                .children(jobs.into_iter().enumerate().map(|(ix, (job, song_art))| {
                    let is_failed = matches!(job.status, DownloadStatus::Failed(_));
                    let job_id = job.id;
                    let core_retry = win.core.clone();
                    let (status_txt, status_col) = match &job.status {
                        DownloadStatus::Searching => {
                            ("Searching best official YouTube audio match…".to_string(), pal.yellow)
                        }
                        DownloadStatus::Downloading(_) => {
                            ("Downloading & tagging audio via yt-dlp…".to_string(), pal.accent)
                        }
                        DownloadStatus::AlreadyExists(p) => {
                            (format!("Already saved at {}", p.display()), pal.aqua)
                        }
                        DownloadStatus::Saved(p) => (format!("Saved -> {}", p.display()), pal.green),
                        DownloadStatus::Failed(e) => (format!("Failed: {e}"), pal.red),
                    };

                    div()
                        .id(SharedString::from(format!("dl-row-{ix}")))
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .bg(hsla(pal.bg_sidebar))
                        .border_1()
                        .border_color(hsla(pal.border_subtle))
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
                                            song_art.as_ref(),
                                            36.,
                                            6.,
                                            IconName::Music,
                                            &job.label,
                                            false,
                                            &pal,
                                        ))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_1()
                                                .child(
                                                    h_flex()
                                                        .justify_between()
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(hsla(pal.fg))
                                                                .truncate()
                                                                .child(job.label),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(10.5))
                                                                .text_color(hsla(pal.fg_muted))
                                                                .child(format!(
                                                                    "from {} · {}",
                                                                    job.source_station,
                                                                    job.created_at
                                                                )),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(hsla(status_col))
                                                        .truncate()
                                                        .child(status_txt),
                                                ),
                                        ),
                                )
                                .when(is_failed, |r| {
                                    r.child(
                                        Button::new(SharedString::from(format!("dl-retry-{ix}")))
                                            .secondary()
                                            .xsmall()
                                            .icon(IconName::RefreshCw)
                                            .label("Retry")
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                core_retry.update(cx, |c, cx| {
                                                    c.retry_download(job_id, cx)
                                                });
                                            })),
                                    )
                                }),
                        )
                })),
        )
}

pub fn render_settings_view(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, cfg, backfill) = {
        let core = win.core.read(cx);
        (
            core.palette.clone(),
            core.config.clone(),
            core.lyrics_backfill.clone(),
        )
    };

    let core_cc = win.core.clone();
    let core_blend = win.core.clone();
    let core_vis = win.core.clone();
    let core_tray = win.core.clone();
    let core_dir = win.core.clone();
    let core_alb_dir = win.core.clone();
    let core_f1 = win.core.clone();
    let core_f2 = win.core.clone();
    let core_lyr_backfill = win.core.clone();
    let core_clean_ed = win.core.clone();
    let core_reorg = win.core.clone();
    let core_log = win.core.clone();
    let reorg_running = win.core.read(cx).reorg_running;
    let backfill_active = backfill.active;
    let backfill_text = if backfill_active || backfill.processed > 0 {
        backfill.summary()
    } else {
        "Scans your Music & Albums folders, skips tracks that already have embedded lyrics, and embeds synced/plain LRCLIB lyrics in place.".to_string()
    };

    v_flex()
        .size_full()
        .p_4()
        .gap_4()
        .overflow_y_scrollbar()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(IconName::Settings).text_color(hsla(pal.accent)))
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(hsla(pal.fg))
                        .child("Vibes Preferences"),
                ),
        )
        .child(
            v_flex()
                .p_3p5()
                .gap_2()
                .rounded(px(8.))
                .bg(hsla(pal.bg_sidebar))
                .border_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(pal.fg))
                        .child("Theme & Plasma Appearance"),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1p5()
                        .children(ThemePreset::ALL.into_iter().map(|preset| {
                            let active = preset == cfg.theme;
                            let core_ent = win.core.clone();
                            Button::new(SharedString::from(format!("thm-{}", preset.label())))
                                .with_variant(if active {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(preset.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_ent.update(cx, |c, cx| c.set_theme_preset(preset, cx));
                                }))
                        })),
                ),
        )
        .child(
            v_flex()
                .p_3p5()
                .gap_2p5()
                .rounded(px(8.))
                .bg(hsla(pal.bg_sidebar))
                .border_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(pal.fg))
                        .child("Radio, Tray & Spectrum"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(pal.fg_subtle))
                                .child("Country code (2-letter, e.g. US / RU / DE):"),
                        )
                        .child(div().w(px(90.)).child(Input::new(&win.country_input).small()))
                        .child(
                            Button::new("set-cc-btn")
                                .secondary()
                                .xsmall()
                                .label("Apply")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let cc = this.country_input.read(cx).value().to_string();
                                    core_cc.update(cx, |c, cx| c.set_country_code(cc, cx));
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("tog-blend")
                                .with_variant(if cfg.local_blending {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(if cfg.local_blending {
                                    "Local Station Blending: On"
                                } else {
                                    "Local Station Blending: Off"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_blend.update(cx, |c, cx| c.toggle_local_blending(cx));
                                })),
                        )
                        .child(
                            Button::new("tog-vis")
                                .with_variant(if cfg.visualizer_enabled {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(if cfg.visualizer_enabled {
                                    "Spectrum Visualizer: On"
                                } else {
                                    "Spectrum Visualizer: Off"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_vis.update(cx, |c, cx| c.toggle_visualizer(cx));
                                })),
                        )
                        .child(
                            Button::new("tog-tray")
                                .with_variant(if cfg.close_to_tray {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(if cfg.close_to_tray {
                                    "Close Button Minimizes to Tray: On"
                                } else {
                                    "Close Button Quits App"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_tray.update(cx, |c, cx| c.toggle_close_to_tray(cx));
                                })),
                        ),
                ),
        )
        .child(
            v_flex()
                .p_3p5()
                .gap_2p5()
                .rounded(px(8.))
                .bg(hsla(pal.bg_sidebar))
                .border_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(pal.fg))
                        .child("Save Song Library (yt-dlp)"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(pal.fg_subtle))
                                .child("Music directory:"),
                        )
                        .child(div().flex_1().child(Input::new(&win.music_dir_input).small()))
                        .child(
                            Button::new("save-music-dir")
                                .secondary()
                                .xsmall()
                                .label("Apply")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let p = this.music_dir_input.read(cx).value().trim().to_string();
                                    if !p.is_empty() {
                                        core_dir.update(cx, |c, cx| {
                                            c.set_music_dir(PathBuf::from(p), cx)
                                        });
                                    }
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("fm-flat")
                                .with_variant(if cfg.save_folder_mode == SaveFolderMode::Flat {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(SaveFolderMode::Flat.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_f1.update(cx, |c, cx| {
                                        c.set_save_folder_mode(SaveFolderMode::Flat, cx)
                                    });
                                })),
                        )
                        .child(
                            Button::new("fm-artist")
                                .with_variant(
                                    if cfg.save_folder_mode == SaveFolderMode::ByArtist {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Secondary
                                    },
                                )
                                .xsmall()
                                .label(SaveFolderMode::ByArtist.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_f2.update(cx, |c, cx| {
                                        c.set_save_folder_mode(SaveFolderMode::ByArtist, cx)
                                    });
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1p5()
                        .children(SaveAudioFormat::ALL.into_iter().map(|fmt| {
                            let active = fmt == cfg.save_audio_format;
                            let core_ent = win.core.clone();
                            Button::new(SharedString::from(format!("fmt-{}", fmt.extension())))
                                .with_variant(if active {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(fmt.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_ent.update(cx, |c, cx| c.set_save_audio_format(fmt, cx));
                                }))
                        })),
                )
                .child(
                    div()
                        .pt_1()
                        .border_t_1()
                        .border_color(hsla(pal.border_subtle))
                        .child(
                            v_flex()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(hsla(pal.fg))
                                        .child("YouTube Auth Cookies (yt-dlp --cookies-from-browser)"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(hsla(pal.fg_muted))
                                        .child("Optional: use a signed-in browser session for streaming & downloads if YouTube asks to confirm you're not a bot."),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_1p5()
                                        .children(
                                            crate::config::YoutubeCookiesSource::ALL
                                                .into_iter()
                                                .map(|src| {
                                                    let active = src == cfg.yt_cookies_browser;
                                                    let core_ent = win.core.clone();
                                                    Button::new(SharedString::from(format!(
                                                        "yt-ck-{}",
                                                        src.label()
                                                    )))
                                                    .with_variant(if active {
                                                        ButtonVariant::Primary
                                                    } else {
                                                        ButtonVariant::Secondary
                                                    })
                                                    .xsmall()
                                                    .label(src.label())
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        core_ent.update(cx, |c, cx| {
                                                            c.set_yt_cookies_browser(src, cx)
                                                        });
                                                    }))
                                                }),
                                        ),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .pt_1()
                        .border_t_1()
                        .border_color(hsla(pal.border_subtle))
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(pal.fg_muted))
                                .child(format!(
                                    "Active theme background: #{:06x} · KDE tray protocol: org.kde.StatusNotifierItem (ksni)",
                                    pal.bg_main
                                )),
                        )
                        .child(
                            Button::new("settings-open-log")
                                .ghost()
                                .xsmall()
                                .label("Open vibes.log")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_log.read(cx).open_log_file();
                                })),
                        ),
                ),
        )
        .child(
            v_flex()
                .p_3p5()
                .gap_2p5()
                .rounded(px(8.))
                .bg(hsla(pal.bg_sidebar))
                .border_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(pal.fg))
                        .child("Full Albums Library"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(pal.fg_subtle))
                                .child("Albums base directory:"),
                        )
                        .child(div().flex_1().child(Input::new(&win.albums_dir_input).small()))
                        .child(
                            Button::new("save-albums-dir")
                                .secondary()
                                .xsmall()
                                .label("Apply")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let p = this.albums_dir_input.read(cx).value().trim().to_string();
                                    if !p.is_empty() {
                                        core_alb_dir.update(cx, |c, cx| {
                                            c.set_albums_dir(PathBuf::from(p), cx)
                                        });
                                    }
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .flex_wrap()
                        .children(AlbumFolderMode::ALL.into_iter().map(|mode| {
                            let active = mode == cfg.album_folder_mode;
                            let core_ent = win.core.clone();
                            Button::new(SharedString::from(format!("alb-mode-{}", mode.label())))
                                .with_variant(if active {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .label(mode.label())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_ent.update(cx, |c, cx| c.set_album_folder_mode(mode, cx));
                                }))
                        })),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("tog-clean-editions")
                                .with_variant(if cfg.clean_editions {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .xsmall()
                                .icon(IconName::Check)
                                .label(if cfg.clean_editions {
                                    "Clean Editions Filter: On (skip live/demo/box-set bloat)"
                                } else {
                                    "Clean Editions Filter: Off"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_clean_ed.update(cx, |c, cx| c.toggle_clean_editions(cx));
                                })),
                        )
                        .child(
                            Button::new("settings-reorg-btn")
                                .secondary()
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .label(if reorg_running {
                                    "Organizing…"
                                } else {
                                    "Organize & Clean ~/Music"
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    core_reorg.update(cx, |c, cx| c.reorganize_local_library(cx));
                                })),
                        ),
                ),
        )
        .child(
            v_flex()
                .p_3p5()
                .gap_2p5()
                .rounded(px(8.))
                .bg(hsla(pal.bg_sidebar))
                .border_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(pal.fg))
                        .child("Embedded Lyrics (LRCLIB)"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(hsla(if backfill_active {
                            pal.yellow
                        } else {
                            pal.fg_subtle
                        }))
                        .child(backfill_text),
                )
                .child(
                    h_flex().child(
                        Button::new("settings-lyrics-backfill")
                            .with_variant(if backfill_active {
                                ButtonVariant::Danger
                            } else {
                                ButtonVariant::Secondary
                            })
                            .xsmall()
                            .icon(IconName::Mic)
                            .label(if backfill_active {
                                "Cancel"
                            } else {
                                "Fetch & Embed Missing Lyrics"
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                core_lyr_backfill.update(cx, |c, cx| {
                                    if c.lyrics_backfill.active {
                                        c.cancel_lyrics_backfill(cx);
                                    } else {
                                        c.start_lyrics_backfill(cx);
                                    }
                                });
                            })),
                    ),
                ),
        )
        .child(
            div()
                .p_3()
                .rounded(px(8.))
                .bg(rgb_with_alpha(pal.accent, 0x16))
                .text_xs()
                .text_color(hsla(pal.fg_subtle))
                .child("Shortcuts: Space Play/Pause · 1-9 Presets · L Lyrics · Ctrl+S Save Song · Ctrl+M Compact Deck · M Mute · Left/Right Seek YT"),
        )
}
