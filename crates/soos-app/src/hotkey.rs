//! The global show/hide hotkey: its default, persistence, and rebinding.
//! Not on Linux (see `FULL_DESKTOP`), where this module isn't built.

use super::*;

/// The keyboard's Calculator key on Windows (upstream `global-hotkey`
/// can't register it; see CONTRIBUTING.md, "The hotkey patch"),
/// Ctrl+Shift+Space on macOS, which has no such key.
pub(crate) fn default_hotkey() -> HotKey {
    if cfg!(target_os = "macos") {
        HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space)
    } else {
        HotKey::new(None, Code::LaunchApp2)
    }
}

/// The saved hotkey, or the default if none was saved or its key no longer
/// parses.
pub(crate) fn load_hotkey(saved: &SavedState) -> HotKey {
    let code = saved
        .hotkey_code
        .as_deref()
        .and_then(|s| s.parse::<Code>().ok());
    let mods = Modifiers::from_bits(saved.hotkey_mods);
    match (code, mods) {
        (Some(code), Some(mods)) => HotKey::new(Some(mods), code),
        _ => default_hotkey(),
    }
}

/// `HotKey`'s own text, except the Calculator key, which it calls
/// "LaunchApp2".
pub(crate) fn hotkey_label(hk: &HotKey) -> String {
    if hk.mods.is_empty() && hk.key == Code::LaunchApp2 {
        "Calculator key".to_string()
    } else {
        hk.to_string()
    }
}

/// The `global-hotkey` code for an egui key, or `None` for keys that can't
/// be a hotkey (a modifier on its own, or one egui can't tell apart).
pub(crate) fn egui_key_to_hotkey_code(key: egui::Key) -> Option<Code> {
    if is_bare_modifier_key(key) {
        return None;
    }
    // Most egui key names are already `Code` names; letters, digits and
    // arrows need a prefix, and four differ.
    let name = key.name();
    let code_name = match name {
        "Equals" => "Equal".to_string(),
        "Backtick" => "Backquote".to_string(),
        "OpenBracket" => "BracketLeft".to_string(),
        "CloseBracket" => "BracketRight".to_string(),
        "Up" | "Down" | "Left" | "Right" => format!("Arrow{name}"),
        _ if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() => format!("Digit{name}"),
        _ if name.len() == 1 => format!("Key{name}"),
        _ => name.to_string(),
    };
    code_name.parse().ok()
}

pub(crate) fn egui_modifiers_to_hotkey_modifiers(m: egui::Modifiers) -> Modifiers {
    let mut mods = Modifiers::empty();
    mods.set(Modifiers::CONTROL, m.ctrl);
    mods.set(Modifiers::SHIFT, m.shift);
    mods.set(Modifiers::ALT, m.alt);
    mods.set(Modifiers::SUPER, m.mac_cmd);
    mods
}

/// Holding Ctrl+Alt+K sends a key event for each modifier first; capture
/// has to skip those and wait for K.
pub(crate) fn is_bare_modifier_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ShiftLeft
            | egui::Key::ShiftRight
            | egui::Key::ControlLeft
            | egui::Key::ControlRight
            | egui::Key::AltLeft
            | egui::Key::AltRight
            | egui::Key::SuperLeft
            | egui::Key::SuperRight
    )
}

const NEEDS_MODIFIER: &str = if cfg!(target_os = "macos") {
    "Hold Cmd, Ctrl or Option with that key"
} else {
    "Hold Ctrl, Alt or Win with that key"
};

/// A global hotkey takes its key away from every other app, so a key that
/// types or moves the cursor needs Ctrl, Alt or Cmd/Win/Super held with it
/// (Shift alone would still steal capital letters). Only F-keys may be bare.
pub(crate) fn needs_a_modifier(code: Code, mods: Modifiers) -> bool {
    let name = code.to_string();
    let function_key = name
        .strip_prefix('F')
        .is_some_and(|n| n.parse::<u8>().is_ok());
    !function_key && !mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
}

impl SoosApp {
    /// Register the saved hotkey. Like the tray, needs the event loop
    /// running; without a manager the `⌨` button says hotkeys are
    /// unavailable.
    pub(crate) fn init_hotkey(&mut self, ctx: &egui::Context) {
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
    }

    /// `⌨`: click to record a new hotkey, click again to cancel.
    pub(crate) fn hotkey_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let tooltip = if self.hotkeys.is_none() {
            "Global hotkeys aren't available on this system".to_string()
        } else if self.capturing_hotkey {
            "Press the new hotkey, or click here to cancel".to_string()
        } else {
            format!(
                "Show or hide Soos from any app with {}. Click to change it.",
                hotkey_label(&self.current_hotkey)
            )
        };
        let color = if self.capturing_hotkey {
            palette.keyword
        } else {
            palette.comment
        };
        let clicked = status_symbol(ui, "\u{2328}", color, palette)
            .on_hover_text(tooltip)
            .clicked();
        if clicked && self.hotkeys.is_some() {
            self.capturing_hotkey = !self.capturing_hotkey;
            self.hotkey_message = None;
            // So the key pressed next doesn't also land in the document.
            ui.ctx().memory_mut(|m| m.stop_text_input());
        }
    }

    /// While capturing: the first real key press becomes the hotkey, Esc
    /// cancels, and a key that can't be used says why and waits for another.
    pub(crate) fn handle_hotkey_capture(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if is_bare_modifier_key(key) {
                continue;
            }
            if key == egui::Key::Escape {
                self.capturing_hotkey = false;
                self.hotkey_message = None;
                return;
            }
            let mods = egui_modifiers_to_hotkey_modifiers(modifiers);
            let result = match egui_key_to_hotkey_code(key) {
                None => Err("That key can't be a hotkey".to_string()),
                Some(code) if needs_a_modifier(code, mods) => Err(NEEDS_MODIFIER.to_string()),
                Some(code) => self.apply_hotkey(HotKey::new(Some(mods), code)),
            };
            self.capturing_hotkey = result.is_err();
            self.hotkey_message = result.err();
            return;
        }
    }

    /// Replace the registered hotkey with `new`. On failure the previous one
    /// is registered again, so there's always a working hotkey, and the
    /// error is a message for the status bar. Must run on the thread that
    /// created the `GlobalHotKeyManager` (egui's).
    pub(crate) fn apply_hotkey(&mut self, new: HotKey) -> Result<(), String> {
        let Some(manager) = &self.hotkeys else {
            return Err("Global hotkeys aren't available".to_string());
        };
        // Fails harmlessly when nothing is registered yet.
        let _ = manager.unregister(self.current_hotkey);
        match manager.register(new) {
            Ok(()) => {
                self.current_hotkey = new;
                Ok(())
            }
            // macOS reports a collision as FailedToRegister, like any other
            // failure, so it gets the second message.
            Err(HotkeyError::AlreadyRegistered(_)) => {
                let _ = manager.register(self.current_hotkey);
                Err("Another app already uses that hotkey".to_string())
            }
            Err(_) => {
                let _ = manager.register(self.current_hotkey);
                Err("That hotkey can't be registered".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_label_special_cases_bare_calculator_key() {
        assert_eq!(
            hotkey_label(&HotKey::new(None, Code::LaunchApp2)),
            "Calculator key"
        );
    }

    #[test]
    fn egui_key_mapping_covers_common_keys_and_excludes_modifiers() {
        assert_eq!(egui_key_to_hotkey_code(egui::Key::A), Some(Code::KeyA));
        assert_eq!(egui_key_to_hotkey_code(egui::Key::Num5), Some(Code::Digit5));
        assert_eq!(egui_key_to_hotkey_code(egui::Key::F13), Some(Code::F13));
        assert_eq!(egui_key_to_hotkey_code(egui::Key::Space), Some(Code::Space));
        assert_eq!(
            egui_key_to_hotkey_code(egui::Key::ArrowUp),
            Some(Code::ArrowUp)
        );
        assert_eq!(
            egui_key_to_hotkey_code(egui::Key::Equals),
            Some(Code::Equal)
        );
        assert_eq!(
            egui_key_to_hotkey_code(egui::Key::Backtick),
            Some(Code::Backquote)
        );
        assert_eq!(
            egui_key_to_hotkey_code(egui::Key::OpenBracket),
            Some(Code::BracketLeft)
        );
        assert_eq!(
            egui_key_to_hotkey_code(egui::Key::CloseBracket),
            Some(Code::BracketRight)
        );
        assert_eq!(egui_key_to_hotkey_code(egui::Key::ShiftLeft), None);
    }

    #[test]
    fn keys_that_type_need_a_real_modifier() {
        assert!(needs_a_modifier(Code::KeyK, Modifiers::empty()));
        assert!(needs_a_modifier(Code::KeyK, Modifiers::SHIFT));
        assert!(needs_a_modifier(Code::ArrowUp, Modifiers::empty()));
        assert!(needs_a_modifier(Code::Space, Modifiers::SHIFT));
        assert!(!needs_a_modifier(Code::KeyK, Modifiers::CONTROL));
        assert!(!needs_a_modifier(
            Code::KeyK,
            Modifiers::ALT | Modifiers::SHIFT
        ));
        assert!(!needs_a_modifier(Code::Space, Modifiers::SUPER));
        assert!(!needs_a_modifier(Code::F13, Modifiers::empty()));
        assert!(!needs_a_modifier(Code::F5, Modifiers::SHIFT));
    }

    #[test]
    fn bare_modifier_keys_are_recognized() {
        assert!(is_bare_modifier_key(egui::Key::AltLeft));
        assert!(is_bare_modifier_key(egui::Key::ControlRight));
        assert!(is_bare_modifier_key(egui::Key::SuperLeft));
        assert!(!is_bare_modifier_key(egui::Key::K));
    }

    #[test]
    fn egui_modifiers_mapping() {
        let mods = egui_modifiers_to_hotkey_modifiers(egui::Modifiers {
            alt: true,
            ctrl: true,
            shift: false,
            mac_cmd: false,
            command: true,
        });
        assert_eq!(mods, Modifiers::ALT | Modifiers::CONTROL);
        assert_eq!(
            egui_modifiers_to_hotkey_modifiers(egui::Modifiers::NONE),
            Modifiers::empty()
        );
    }

    /// `HotKey`'s own string parser can't read "LaunchApp2" back, which is
    /// why the key and modifiers are saved separately.
    #[test]
    fn saved_hotkey_round_trips_through_load_hotkey() {
        for original in [
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyK),
            HotKey::new(None, Code::LaunchApp2),
        ] {
            let saved = SavedState {
                hotkey_code: Some(original.key.to_string()),
                hotkey_mods: original.mods.bits(),
                ..Default::default()
            };
            let loaded = load_hotkey(&saved);
            assert_eq!(loaded.key, original.key);
            assert_eq!(loaded.mods, original.mods);
        }
    }

    #[test]
    fn load_hotkey_falls_back_to_default_when_nothing_saved() {
        let saved = SavedState::default();
        assert_eq!(load_hotkey(&saved), default_hotkey());
    }
}
