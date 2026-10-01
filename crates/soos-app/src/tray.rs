//! The tray icon, and hiding and showing the window.

use super::*;

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
    static TRAY_AND_MENU: std::cell::RefCell<Option<(TrayIcon, Menu)>> =
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

impl SoosApp {
    /// Hide or show the window. On macOS the Dock icon goes with it, since
    /// the menu-bar icon is the way back (winit registers no
    /// `NSApplicationDelegate`, so a click on the Dock icon couldn't show a
    /// hidden window anyway).
    pub(crate) fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if !visible && !FULL_DESKTOP {
            return;
        }
        self.visible = visible;
        #[cfg(target_os = "macos")]
        show_in_dock(visible);
        ctx.send_viewport_cmd(ViewportCommand::Visible(visible));
        if visible {
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
}
