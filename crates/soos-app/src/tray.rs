//! The tray icon, and hiding and showing the window.

use std::time::Duration;

use eframe::egui::{self, ViewportCommand};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::{AppEvent, SoosApp};

/// eframe sets the macOS Dock icon from this at runtime, over the bundle's
/// `soos.icns` -- and the full-bleed logo is too big for Apple's icon
/// grid. An empty icon is the one it leaves alone.
pub(crate) fn window_icon() -> egui::IconData {
    if cfg!(target_os = "macos") {
        return egui::IconData::default();
    }
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icons/hicolor/256x256.png"))
        .expect("bundled window icon is a valid png")
}

/// Decoded with eframe's PNG loader, so the app needs no image crate.
pub(crate) fn tray_icon_image() -> Icon {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/icons/hicolor/32x32.png"
    ))
    .expect("bundled tray icon is a valid png");
    Icon::from_rgba(icon.rgba, icon.width, icon.height)
        .expect("bundled tray icon has valid dimensions")
}

/// An `Accessory` app has no Dock icon and isn't in Cmd-Tab, but keeps its
/// menu-bar icon.
#[cfg(target_os = "macos")]
fn show_in_dock(show: bool) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let policy = if show {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}

#[cfg(target_os = "macos")]
thread_local! {
    /// macOS: tray-icon hands the menu to the status item, and macOS then
    /// opens it on every click without tray-icon seeing the click. So the
    /// menu is only attached while a right-click opens it, and taken off
    /// once the pointer leaves the icon. Both run in the tray's event
    /// handler, on the main thread, where the tray and menu live.
    static TRAY_AND_MENU: std::cell::RefCell<Option<(tray_icon::TrayIcon, Menu)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "macos")]
fn attach_tray_menu(attach: bool) {
    TRAY_AND_MENU.with_borrow(|tray_and_menu| {
        if let Some((tray, menu)) = tray_and_menu {
            tray.set_menu(attach.then(|| Box::new(menu.clone()) as _));
        }
    });
}

/// Whether the window can hide to the tray, be raised by a global hotkey
/// and stay always on top. Not on Linux: Wayland can do none of these, and
/// one behaviour for both Linux sessions is simpler to trust than detecting
/// which one winit picked. So on Linux closing quits, and the status bar
/// has no `⌨` or `▲`.
pub(crate) const FULL_DESKTOP: bool = !cfg!(target_os = "linux");

/// How long a window closed on macOS takes to give up focus as it minimizes.
/// Focus seen before this is the window on its way down, not a restore.
pub(crate) const MINIMIZE_SETTLES: Duration = Duration::from_millis(500);

/// Whether the system put the window back without Soos showing it. On macOS
/// closing minimizes the window, and a click on the Dock icon restores it
/// with no event of ours. Two things then stay wrong: `visible` is still
/// false, so the hotkey would "show" a window already on screen; and egui
/// still holds the `Minimized(true)` we sent, which it never refreshes on
/// macOS, so it skips drawing the UI and the window looks frozen. Having
/// focus while minimized is the sign (`minimized_for` is the time since the
/// close).
pub(crate) fn restored_by_the_system(
    visible: bool,
    minimized_for: Option<Duration>,
    focused: bool,
) -> bool {
    !visible && focused && minimized_for.is_some_and(|time| time >= MINIMIZE_SETTLES)
}

impl SoosApp {
    /// Hide or show the window. On macOS the Dock icon goes with it, since
    /// the menu-bar icon is the way back (winit's `NSApplicationDelegate`
    /// doesn't handle a click on the Dock icon, so it couldn't show a hidden
    /// window anyway). Showing also brings back a window that closing
    /// minimized.
    pub(crate) fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if !visible && !FULL_DESKTOP {
            return;
        }
        self.visible = visible;
        self.minimized_at = None;
        #[cfg(target_os = "macos")]
        show_in_dock(visible);
        ctx.send_viewport_cmd(ViewportCommand::Visible(visible));
        if visible {
            #[cfg(target_os = "macos")]
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    /// Build the tray icon. Needs the event loop running, so it's called
    /// from the first `logic()`. The app works without it: with no tray,
    /// closing the window quits (see `logic`).
    pub(crate) fn init_tray(&mut self, ctx: &egui::Context) {
        // A click shows the window and a right-click opens the menu. Linux
        // keeps "Show Soos" there too: some AppIndicator hosts open the
        // menu on any click and never report the click itself.
        let menu = Menu::new();
        let show_item = MenuItem::new("Show Soos", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();
        if cfg!(target_os = "linux") {
            if let Err(e) = menu.append(&show_item) {
                eprintln!("soos: tray menu: {e}");
            }
        }
        if let Err(e) = menu.append(&quit_item) {
            eprintln!("soos: tray menu: {e}");
        }

        let builder = TrayIconBuilder::new();
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_menu(Box::new(menu));
        self.tray = match builder
            .with_menu_on_left_click(false)
            .with_icon(tray_icon_image())
            .with_tooltip("Soos")
            .build()
        {
            Ok(tray) => Some(tray),
            Err(e) => {
                eprintln!("soos: tray icon unavailable: {e}");
                None
            }
        };
        #[cfg(target_os = "macos")]
        TRAY_AND_MENU.set(self.tray.clone().map(|tray| (tray, menu)));

        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let msg = if event.id == show_id {
                Some(AppEvent::Show)
            } else if event.id == quit_id {
                Some(AppEvent::Quit)
            } else {
                None
            };
            if let Some(msg) = msg {
                let _ = tx.send(msg);
                repaint_ctx.request_repaint();
            }
        }));

        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                let _ = tx.send(AppEvent::Show);
                repaint_ctx.request_repaint();
            }
            #[cfg(target_os = "macos")]
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Down,
                ..
            } => attach_tray_menu(true),
            #[cfg(target_os = "macos")]
            TrayIconEvent::Leave { .. } => attach_tray_menu(false),
            _ => {}
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_and_window_icons_decode() {
        let _ = window_icon();
        let _ = tray_icon_image();
    }

    /// Focus counts only once the minimize has had time to take it, so the
    /// frames just after the close, which still report focus, don't undo it.
    #[test]
    fn a_minimized_window_that_has_focus_was_restored() {
        let later = Some(MINIMIZE_SETTLES);
        let just_now = Some(Duration::from_millis(50));
        assert!(restored_by_the_system(false, later, true));
        assert!(!restored_by_the_system(false, just_now, true));
        assert!(!restored_by_the_system(false, later, false));
        assert!(!restored_by_the_system(true, later, true));
        // Hidden to the tray rather than minimized: nothing to undo.
        assert!(!restored_by_the_system(false, None, true));
    }
}
