mod albums;
mod artwork;
mod config;
mod core;
mod downloader;
mod instance;
mod local_library;
mod lyrics;
mod lyrics_overlay;
mod mpris;
mod mpv;
mod radio;
mod theme;
mod tray;
mod ui;
mod visualizer;
mod youtube;

use crate::core::{VibesCore, WindowCmd};
use gpui_kit::assets::AllAssets;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, QuitMode, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use std::cell::RefCell;
use std::rc::Rc;

fn open_vibes_window(
    core: &Entity<VibesCore>,
    active_win: &Rc<RefCell<Option<AnyWindowHandle>>>,
    win_tx: &async_channel::Sender<WindowCmd>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let compact = core.read(cx).compact_mode;
    let dims = if compact {
        size(px(540.), px(148.))
    } else {
        size(px(1160.), px(740.))
    };
    let bounds = Bounds::centered(None, dims, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Vibes".into()),
            ..Default::default()
        }),
        app_id: Some("vibes".to_string()),
        ..Default::default()
    };

    let core_clone = core.clone();
    let tx_close = win_tx.clone();
    let active_win_close = active_win.clone();

    let Ok((handle, _)) = gpui_kit::open_window(options, cx, move |window, cx| {
        window.set_window_title("Vibes");
        let c_close = core_clone.clone();
        let tx = tx_close.clone();
        let aw = active_win_close.clone();
        window.on_window_should_close(cx, move |_window, cx| {
            let to_tray = c_close.read(cx).config.close_to_tray;
            if to_tray {
                *aw.borrow_mut() = None;
                c_close.update(cx, |c, cx| c.set_window_open(false, cx));
                true
            } else {
                let _ = tx.try_send(WindowCmd::Quit);
                false
            }
        });

        cx.new(|cx| ui::VibesWindow::new(core_clone, window, cx))
    }) else {
        return None;
    };

    *active_win.borrow_mut() = Some(handle);
    core.update(cx, |c, cx| c.set_window_open(true, cx));
    Some(handle)
}

fn main() {
    if instance::notify_existing_instance() {
        return;
    }

    gpui_kit::application()
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            ui::register_keybindings(cx);

            let (win_tx, win_rx) = async_channel::unbounded();
            let core = cx.new(|cx| VibesCore::new(win_tx.clone(), cx));
            let active_win: Rc<RefCell<Option<AnyWindowHandle>>> = Rc::new(RefCell::new(None));

            open_vibes_window(&core, &active_win, &win_tx, cx);

            let core_loop = core.clone();
            let aw_loop = active_win.clone();
            let tx_loop = win_tx.clone();
            cx.spawn(async move |cx| {
                while let Ok(cmd) = win_rx.recv().await {
                    let _ = cx.update(|cx| match cmd {
                        WindowCmd::Show => {
                            let existing = *aw_loop.borrow();
                            let activated = existing
                                .and_then(|h| h.update(cx, |_, w, _| w.activate_window()).ok())
                                .is_some();
                            if !activated {
                                open_vibes_window(&core_loop, &aw_loop, &tx_loop, cx);
                            }
                        }
                        WindowCmd::Hide => {
                            if let Some(h) = aw_loop.borrow_mut().take() {
                                let _ = h.update(cx, |_, w, _| w.remove_window());
                            }
                            core_loop.update(cx, |c, cx| c.set_window_open(false, cx));
                        }
                        WindowCmd::Toggle => {
                            let existing = *aw_loop.borrow();
                            if let Some(h) = existing {
                                *aw_loop.borrow_mut() = None;
                                let _ = h.update(cx, |_, w, _| w.remove_window());
                                core_loop.update(cx, |c, cx| c.set_window_open(false, cx));
                            } else {
                                open_vibes_window(&core_loop, &aw_loop, &tx_loop, cx);
                            }
                        }
                        WindowCmd::ApplyCompact(compact) => {
                            if let Some(h) = *aw_loop.borrow() {
                                let dims = if compact {
                                    size(px(540.), px(148.))
                                } else {
                                    size(px(1160.), px(740.))
                                };
                                let _ = h.update(cx, |_, w, _| w.resize(dims));
                            }
                        }
                        WindowCmd::Quit => {
                            core_loop.update(cx, |c, cx| c.shutdown(cx));
                        }
                    });
                }
            })
            .detach();
        });
}
