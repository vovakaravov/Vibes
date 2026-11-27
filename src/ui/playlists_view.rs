use super::{VibesWindow, art};
use crate::config::{PlayTarget, QueueItem};
use crate::theme::{hsla, rgb_with_alpha};
use crate::youtube::format_duration;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px,
};

pub fn render_playlists_view(
    win: &mut VibesWindow,
    cx: &mut Context<VibesWindow>,
) -> impl IntoElement {
    let (pal, liked_tracks, playlists, selected_id, active_local_path, resolved_tracks) = {
        let core = win.core.read(cx);
        let active_local = match &core.current_target {
            Some(PlayTarget::Local(t)) => Some(t.path.clone()),
            _ => None,
        };
        let sel_id = core.selected_playlist_id.clone();
        let tracks = match sel_id.as_deref() {
            None => core.resolve_tracks_by_paths(&core.state.liked_tracks),
            Some(pid) => core
                .state
                .playlists
                .iter()
                .find(|p| p.id == pid)
                .map(|p| core.resolve_tracks_by_paths(&p.track_paths))
                .unwrap_or_default(),
        };
        (
            core.palette.clone(),
            core.state.liked_tracks.len(),
            core.state.playlists.clone(),
            sel_id,
            active_local,
            tracks,
        )
    };

    let core_create = win.core.clone();
    let core_liked_tab = win.core.clone();
    let core_play_all = win.core.clone();
    let core_shuffle = win.core.clone();
    let core_del = win.core.clone();

    let selected_name = match selected_id.as_deref() {
        None => "Liked Songs".to_string(),
        Some(pid) => playlists
            .iter()
            .find(|p| p.id == pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Playlist".to_string()),
    };

    let sel_for_play = selected_id.clone();
    let sel_for_shuf = selected_id.clone();
    let sel_for_del = selected_id.clone();

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
                        .child(Icon::new(IconName::ListMusic).text_color(hsla(pal.accent)))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child("Playlists & Liked Tracks"),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(div().w(px(210.)).child(Input::new(&win.playlist_name_input).small()))
                        .child(
                            Button::new("pl-create-btn")
                                .primary()
                                .small()
                                .icon(IconName::Plus)
                                .label("New Playlist")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let name = this.playlist_name_input.read(cx).value().to_string();
                                    core_create.update(cx, |c, cx| c.create_playlist(name, cx));
                                    this.playlist_name_input
                                        .update(cx, |s, cx| s.set_value("", window, cx));
                                })),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_1p5()
                .flex_wrap()
                .child(
                    Button::new("pl-tab-liked")
                        .with_variant(if selected_id.is_none() {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .xsmall()
                        .icon(IconName::Star)
                        .label(format!("Liked Songs ({liked_tracks})"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            core_liked_tab.update(cx, |c, cx| {
                                c.selected_playlist_id = None;
                                cx.notify();
                            });
                        })),
                )
                .children(playlists.iter().enumerate().map(|(ix, pl)| {
                    let active = selected_id.as_deref() == Some(pl.id.as_str());
                    let pid = pl.id.clone();
                    let core_sel = win.core.clone();
                    Button::new(SharedString::from(format!("pl-tab-{ix}")))
                        .with_variant(if active {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .xsmall()
                        .icon(IconName::ListMusic)
                        .label(format!("{} ({})", pl.name, pl.track_paths.len()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let id = pid.clone();
                            core_sel.update(cx, |c, cx| {
                                c.selected_playlist_id = Some(id);
                                cx.notify();
                            });
                        }))
                })),
        )
        .child(
            h_flex()
                .px_3()
                .py_2()
                .justify_between()
                .items_center()
                .rounded(px(8.))
                .bg(hsla(pal.bg_panel))
                .border_1()
                .border_color(hsla(pal.border_subtle))
                .child(
                    v_flex()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(pal.fg))
                                .child(selected_name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(pal.fg_muted))
                                .child(format!("{} tracks", resolved_tracks.len())),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .when(!resolved_tracks.is_empty(), |r| {
                            r.child(
                                Button::new("pl-play-all")
                                    .primary()
                                    .xsmall()
                                    .icon(IconName::Play)
                                    .label("Play All")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let sid = sel_for_play.clone();
                                        core_play_all.update(cx, |c, cx| match sid.as_deref() {
                                            None => c.play_liked_songs(0, false, cx),
                                            Some(id) => c.play_playlist(id, 0, false, cx),
                                        });
                                    })),
                            )
                            .child(
                                Button::new("pl-shuffle-all")
                                    .secondary()
                                    .xsmall()
                                    .icon(IconName::Shuffle)
                                    .label("Shuffle")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let sid = sel_for_shuf.clone();
                                        core_shuffle.update(cx, |c, cx| match sid.as_deref() {
                                            None => c.play_liked_songs(0, true, cx),
                                            Some(id) => c.play_playlist(id, 0, true, cx),
                                        });
                                    })),
                            )
                        })
                        .when_some(sel_for_del, |r, del_id| {
                            r.child(
                                Button::new("pl-delete")
                                    .danger()
                                    .xsmall()
                                    .icon(IconName::Trash)
                                    .label("Delete")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let id = del_id.clone();
                                        core_del.update(cx, |c, cx| c.delete_playlist(&id, cx));
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
                .when(resolved_tracks.is_empty(), |list| {
                    list.child(
                        div()
                            .p_6()
                            .text_xs()
                            .text_color(hsla(pal.fg_muted))
                            .child(if selected_id.is_none() {
                                "No liked offline tracks yet. Click the ★ button on any track in Artists, Albums, or the Player Deck."
                            } else {
                                "This playlist is empty. Add tracks from any Artist, Album, or save your current Up Next queue as a playlist."
                            }),
                    )
                })
                .children(resolved_tracks.into_iter().enumerate().map(|(ix, tr)| {
                    let is_active = active_local_path.as_ref() == Some(&tr.path);
                    let core_play = win.core.clone();
                    let core_next = win.core.clone();
                    let core_q = win.core.clone();
                    let core_rm = win.core.clone();
                    let core_art = win.core.clone();
                    let sel_play = selected_id.clone();
                    let sel_rm = selected_id.clone();
                    let tr_next = tr.clone();
                    let tr_q = tr.clone();
                    let tr_path = tr.path.clone();
                    let art_name = tr.artist.clone();

                    div()
                        .id(SharedString::from(format!("pl-trk-{ix}")))
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
                            let sid = sel_play.clone();
                            core_play.update(cx, |c, cx| match sid.as_deref() {
                                None => c.play_liked_songs(ix, false, cx),
                                Some(id) => c.play_playlist(id, ix, false, cx),
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
                                            tr.cover_path.as_ref(),
                                            34.,
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
                                                .child(
                                                    div()
                                                        .id(SharedString::from(format!("pl-art-jump-{ix}")))
                                                        .text_xs()
                                                        .text_color(hsla(pal.fg_muted))
                                                        .hover(|s| s.text_color(hsla(pal.accent)))
                                                        .truncate()
                                                        .on_click(cx.listener(move |_, _, _, cx| {
                                                            cx.stop_propagation();
                                                            core_art.update(cx, |c, cx| {
                                                                c.jump_to_artist_by_name(&art_name, cx);
                                                            });
                                                        }))
                                                        .child(format!("{} · {}", tr.artist, tr.album)),
                                                ),
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
                                            Button::new(SharedString::from(format!("pl-next-{ix}")))
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
                                            Button::new(SharedString::from(format!("pl-q-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::Plus)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let t = tr_q.clone();
                                                    core_q.update(cx, |c, cx| {
                                                        c.queue_local_track(t, cx)
                                                    });
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("pl-rm-{ix}")))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::X)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let sid = sel_rm.clone();
                                                    let p = tr_path.clone();
                                                    core_rm.update(cx, |c, cx| match sid.as_deref() {
                                                        None => c.toggle_like_track(p, cx),
                                                        Some(id) => c.remove_track_from_playlist(id, ix, cx),
                                                    });
                                                })),
                                        ),
                                ),
                        )
                })),
        )
}
