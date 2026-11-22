use super::VibesWindow;
use crate::config::{SaveAudioFormat, SaveFolderMode};
use crate::core::NavSection;
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
        (core.palette.clone(), core.state.favorites.clone())
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
                .children(favorites.into_iter().enumerate().map(|(ix, st)| {
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
                                                .child(format!("{} · {}", entry.station, entry.timestamp)),
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
                                                        c.section = NavSection::Youtube;
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
    let (pal, queue) = {
        let core = win.core.read(cx);
        (core.palette.clone(), core.queue.clone())
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
                        .child(Icon::new(IconName::ListMusic).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Playback Queue"),
                        ),
                )
                .when(!queue.is_empty(), |row| {
                    row.child(
                        Button::new("clear-queue-btn")
                            .ghost()
                            .xsmall()
                            .label("Clear Queue")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                core_clr.update(cx, |c, cx| c.clear_queue(cx));
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
                .when(queue.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("Queue is empty. Add tracks or podcasts from YouTube Audio using +Queue."),
                    )
                })
                .children(queue.into_iter().enumerate().map(|(ix, item)| {
                    let core_play = win.core.clone();
                    let core_rm = win.core.clone();
                    let item_play = item.clone();
                    let dur_txt = item
                        .duration_secs
                        .map(format_duration)
                        .unwrap_or_else(|| "AUDIO".to_string());

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
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(hsla(pal.fg))
                                                .truncate()
                                                .child(item.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(hsla(pal.fg_muted))
                                                .truncate()
                                                .child(format!("{} · {dur_txt}", item.channel)),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .child(
                                            Button::new(SharedString::from(format!("q-play-{ix}")))
                                                .primary()
                                                .xsmall()
                                                .icon(IconName::Play)
                                                .label("Play")
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    let it = item_play.clone();
                                                    core_play.update(cx, |c, cx| {
                                                        c.remove_queue_index(ix, cx);
                                                        c.play_youtube(it, cx);
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
    let (pal, jobs, music_dir, fmt_label) = {
        let core = win.core.read(cx);
        (
            core.palette.clone(),
            core.downloads.clone(),
            core.config.music_dir.display().to_string(),
            core.config.save_audio_format.label(),
        )
    };
    let core_open = win.core.clone();

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
                                        .child("Saved Songs via yt-dlp"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(pal.fg_muted))
                                .child(format!("Saving {fmt_label} cleanly into {music_dir}")),
                        ),
                )
                .child(
                    Button::new("open-music-folder")
                        .secondary()
                        .small()
                        .icon(IconName::Folder)
                        .label("Open Music Folder")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            core_open.read(cx).open_music_folder();
                        })),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1p5()
                .overflow_y_scrollbar()
                .when(jobs.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child("When an identifiable song is playing on radio (or in YouTube Audio), click Save Song (or press Ctrl+S) to fetch the clean full track via yt-dlp directly into your library."),
                    )
                })
                .children(jobs.into_iter().enumerate().map(|(ix, job)| {
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
                                                            job.source_station, job.created_at
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
    let (pal, cfg) = {
        let core = win.core.read(cx);
        (core.palette.clone(), core.config.clone())
    };

    let core_cc = win.core.clone();
    let core_blend = win.core.clone();
    let core_vis = win.core.clone();
    let core_tray = win.core.clone();
    let core_dir = win.core.clone();
    let core_f1 = win.core.clone();
    let core_f2 = win.core.clone();

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
                        .text_size(px(11.))
                        .text_color(hsla(pal.fg_muted))
                        .child(format!(
                            "Active theme background: #{:06x} · KDE tray protocol: org.kde.StatusNotifierItem (ksni)",
                            pal.bg_main
                        )),
                ),
        )
        .child(
            div()
                .p_3()
                .rounded(px(8.))
                .bg(rgb_with_alpha(pal.accent, 0x16))
                .text_xs()
                .text_color(hsla(pal.fg_subtle))
                .child("Shortcuts: Space Play/Pause · 1-9 Presets · Ctrl+S Save Song · Ctrl+M Compact Deck · M Mute · Left/Right Seek YT"),
        )
}
