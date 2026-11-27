use super::{VibesWindow, art};
use crate::core::{DiscoverTab, NavSection, VibesCore};
use crate::theme::{hsla, rgb_with_alpha};
use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px,
};

struct SidebarRow {
    sec: NavSection,
    disc_tracks: Option<bool>,
    icon: IconName,
    label: &'static str,
    badge: Option<String>,
    badge_danger: bool,
}

fn render_group_header(label: &'static str, pal: &crate::theme::Palette) -> AnyElement {
    div()
        .mt_2()
        .px_2()
        .py_0p5()
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(hsla(pal.fg_muted))
        .child(label)
        .into_any_element()
}

fn render_nav_row(
    item: SidebarRow,
    active_sec: NavSection,
    active_disc_tab: DiscoverTab,
    pal: &crate::theme::Palette,
    win: &VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> AnyElement {
    let is_active = match item.disc_tracks {
        Some(true) => active_sec == NavSection::Discover && active_disc_tab == DiscoverTab::Tracks,
        Some(false) => {
            active_sec == NavSection::Discover && active_disc_tab != DiscoverTab::Tracks
        }
        None => item.sec == active_sec,
    };
    let core_ent = win.core.clone();
    let sec = item.sec;
    let disc_tracks = item.disc_tracks;
    let pal = pal.clone();

    div()
        .id(SharedString::from(format!("nav-{}", item.label)))
        .w_full()
        .h(px(31.))
        .px_2()
        .rounded(px(6.))
        .cursor_pointer()
        .bg(if is_active {
            rgb_with_alpha(pal.accent, 0x22)
        } else {
            hsla(pal.bg_sidebar)
        })
        .border_l_2()
        .border_color(hsla(if is_active {
            pal.accent
        } else {
            pal.bg_sidebar
        }))
        .hover(|s| s.bg(hsla(pal.bg_panel)))
        .on_click(cx.listener(move |_, _, _, cx| {
            core_ent.update(cx, |c: &mut VibesCore, cx| {
                c.section = sec;
                match disc_tracks {
                    Some(true) => c.discover_tab = DiscoverTab::Tracks,
                    Some(false) if c.discover_tab == DiscoverTab::Tracks => {
                        c.discover_tab = DiscoverTab::Artists;
                    }
                    _ => {}
                }
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
                        .child(Icon::new(item.icon).small().text_color(hsla(if is_active {
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
                                .child(item.label),
                        ),
                )
                .when_some(item.badge, |row, b| {
                    row.child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(99.))
                            .bg(if item.badge_danger {
                                rgb_with_alpha(pal.red, 0x33)
                            } else {
                                hsla(pal.bg_btn)
                            })
                            .text_size(px(10.))
                            .font_weight(if item.badge_danger {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(hsla(if item.badge_danger {
                                pal.red
                            } else {
                                pal.fg_muted
                            }))
                            .child(b),
                    )
                }),
        )
        .into_any_element()
}

pub fn render_sidebar(win: &mut VibesWindow, cx: &mut Context<VibesWindow>) -> impl IntoElement {
    let (
        pal,
        active_sec,
        active_disc_tab,
        artists_count,
        albums_count,
        playlists_count,
        queue_count,
        active_dl,
        failed_dl,
        dl_total,
        presets,
        current_stream,
    ) = {
        let core = win.core.read(cx);
        let presets: Vec<(
            usize,
            String,
            crate::radio::RadioStation,
            Option<std::path::PathBuf>,
        )> = core
            .state
            .favorites
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, s)| {
                (
                    i + 1,
                    s.name.clone(),
                    s.clone(),
                    core.art_path_for_url(&s.favicon).cloned(),
                )
            })
            .collect();
        let current_stream = match &core.current_target {
            Some(crate::config::PlayTarget::Radio(s)) => Some(s.stream_url().to_lowercase()),
            _ => None,
        };
        (
            core.palette.clone(),
            core.section,
            core.discover_tab,
            core.local_library.artists.len(),
            core.local_library.albums.len(),
            core.state.playlists.len() + usize::from(!core.state.liked_tracks.is_empty()),
            core.queue.len(),
            core.active_saving_count(),
            core.state.failed_downloads.len(),
            core.downloads.len() + core.album_downloads.len(),
            presets,
            current_stream,
        )
    };

    let library_items = [
        SidebarRow {
            sec: NavSection::Artists,
            disc_tracks: None,
            icon: IconName::Library,
            label: "Artists",
            badge: (artists_count > 0).then(|| artists_count.to_string()),
            badge_danger: false,
        },
        SidebarRow {
            sec: NavSection::Albums,
            disc_tracks: None,
            icon: IconName::Disc3,
            label: "Albums",
            badge: (albums_count > 0).then(|| albums_count.to_string()),
            badge_danger: false,
        },
        SidebarRow {
            sec: NavSection::Playlists,
            disc_tracks: None,
            icon: IconName::ListMusic,
            label: "Playlists",
            badge: (playlists_count > 0).then(|| playlists_count.to_string()),
            badge_danger: false,
        },
    ];

    let discover_items = [
        SidebarRow {
            sec: NavSection::Discover,
            disc_tracks: Some(true),
            icon: IconName::Search,
            label: "Search Songs",
            badge: None,
            badge_danger: false,
        },
        SidebarRow {
            sec: NavSection::Discover,
            disc_tracks: Some(false),
            icon: IconName::Disc3,
            label: "Artists & Albums",
            badge: None,
            badge_danger: false,
        },
        SidebarRow {
            sec: NavSection::Radio,
            disc_tracks: None,
            icon: IconName::Radio,
            label: "Live Radio",
            badge: None,
            badge_danger: false,
        },
    ];

    let activity_items = [
        SidebarRow {
            sec: NavSection::Queue,
            disc_tracks: None,
            icon: IconName::Play,
            label: "Up Next",
            badge: (queue_count > 0).then(|| queue_count.to_string()),
            badge_danger: false,
        },
        SidebarRow {
            sec: NavSection::Downloads,
            disc_tracks: None,
            icon: IconName::Download,
            label: "Downloads",
            badge: if failed_dl > 0 {
                Some(format!("{failed_dl}!"))
            } else if active_dl > 0 {
                Some(format!("{active_dl}…"))
            } else if dl_total > 0 {
                Some(dl_total.to_string())
            } else {
                None
            },
            badge_danger: failed_dl > 0,
        },
        SidebarRow {
            sec: NavSection::Settings,
            disc_tracks: None,
            icon: IconName::Settings,
            label: "Settings",
            badge: None,
            badge_danger: false,
        },
    ];

    v_flex()
        .w(px(210.))
        .h_full()
        .flex_shrink_0()
        .bg(hsla(pal.bg_sidebar))
        .border_r_1()
        .border_color(hsla(pal.border_subtle))
        .p_2p5()
        .gap_0p5()
        .child(render_group_header("LIBRARY", &pal))
        .children(
            library_items
                .into_iter()
                .map(|it| render_nav_row(it, active_sec, active_disc_tab, &pal, win, cx)),
        )
        .child(render_group_header("DISCOVER", &pal))
        .children(
            discover_items
                .into_iter()
                .map(|it| render_nav_row(it, active_sec, active_disc_tab, &pal, win, cx)),
        )
        .child(render_group_header("ACTIVITY", &pal))
        .children(
            activity_items
                .into_iter()
                .map(|it| render_nav_row(it, active_sec, active_disc_tab, &pal, win, cx)),
        )
        .when(!presets.is_empty(), |rail| {
            rail.child(render_group_header("PRESETS (1-6)", &pal)).child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_0p5()
                    .overflow_y_scrollbar()
                    .children(presets.into_iter().map(|(slot, name, station, fav_art)| {
                        let is_playing_preset = current_stream
                            .as_deref()
                            .is_some_and(|u| u == station.stream_url().to_lowercase());
                        let core_ent = win.core.clone();
                        div()
                            .id(SharedString::from(format!("preset-{slot}")))
                            .w_full()
                            .h(px(30.))
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
                                    .gap_1p5()
                                    .child(art::render_art_tile(
                                        fav_art.as_ref(),
                                        20.,
                                        4.,
                                        IconName::Radio,
                                        &name,
                                        is_playing_preset,
                                        &pal,
                                    ))
                                    .child(
                                        div()
                                            .w(px(15.))
                                            .h(px(15.))
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
