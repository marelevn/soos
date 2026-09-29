//! The tray icon, and hiding and showing the window.

use super::*;

pub(crate) fn window_icon() -> egui::IconData {
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

impl SoosApp {
    /// Hide or show the window. macOS minimizes instead of hiding: winit
    /// registers no `NSApplicationDelegate`, so a hidden window can't be
    /// brought back from the Dock, while a minimized one can (`logic`
    /// notices that restore).
    pub(crate) fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        self.visible = visible;
        self.hidden_unfocused = false;
        if cfg!(target_os = "macos") {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(!visible));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Visible(visible));
        }
        if visible {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    /// Build the tray icon and register the global hotkey. Needs the event
    /// loop running, so it's called from the first `logic()`. The app works
    /// without either: with no tray, closing the window quits (see `logic`),
    /// and the hotkey button says hotkeys are unavailable.
    pub(crate) fn init_tray_and_hotkey(&mut self, ctx: &egui::Context) {
        let menu = Menu::new();
        let show_item = MenuItem::new("Show Soos", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();
        if let Err(e) = menu.append(&show_item) {
            eprintln!("soos: tray menu: {e}");
        }
        if let Err(e) = menu.append(&quit_item) {
            eprintln!("soos: tray menu: {e}");
        }

        self.tray = match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
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

        self.hotkeys = match GlobalHotKeyManager::new() {
            Ok(manager) => Some(manager),
            Err(e) => {
                eprintln!("soos: global hotkey manager unavailable: {e}");
                None
            }
        };
        if self.hotkeys.is_some() {
            let label = hotkey_label(&self.current_hotkey);
            self.hotkey_message = self
                .apply_hotkey(self.current_hotkey)
                .err()
                .map(|e| format!("{e} ({label})"));
        }

        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let _ = tx.send(AppEvent::Toggle);
                repaint_ctx.request_repaint();
            }
        }));

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
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = tx.send(AppEvent::Show);
                repaint_ctx.request_repaint();
            }
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
