//! Soos: a document where every line is an expression and its answer shows
//! up beside it. The modules are the parts of the window: [`tabs`] across
//! the top, [`editor`] in the middle, [`status_bar`] at the bottom, and
//! the overlays behind it ([`example`], [`converters`]).

// A release build has no console window; a debug build keeps one for
// `cargo run`'s output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{
    self, text::LayoutJob, Color32, FontId, RichText, TextFormat, Theme, ViewportCommand,
};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    Error as HotkeyError, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use soos_core::{currency::RateSource, format::Shown, highlight::TokenKind, LineResult};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

mod converters;
mod editor;
mod example;
mod hotkey;
mod recalc;
mod status_bar;
mod style;
mod tabs;
mod tray;
mod update;
mod window;

use converters::*;
use editor::*;
use example::*;
use hotkey::*;
use recalc::*;
use status_bar::*;
use style::*;
use tabs::*;
use tray::*;
use update::*;
use window::*;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([DEFAULT_WIDTH, DEFAULT_HEIGHT])
            .with_min_inner_size([MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT])
            .with_icon(window_icon()),
        persistence_path: Some(soos_core::storage::data_dir().join("app.ron")),
        ..Default::default()
    };
    eframe::run_native(
        "Soos",
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            for theme in [Theme::Dark, Theme::Light] {
                cc.egui_ctx.set_visuals_of(theme, app_visuals(theme));
            }
            // Reserve the scrollbar's width so content doesn't lay out under
            // it. Set once here: `Ui::style_mut` per frame clones the whole
            // style, which lags visibly.
            cc.egui_ctx.all_styles_mut(|style| {
                style.spacing.scroll.floating_allocated_width = style.spacing.scroll.bar_width;
            });
            Ok(Box::new(SoosApp::new(cc)))
        }),
    )
}

/// From the tray, hotkey and background threads to `logic()`, the only
/// place viewport commands may be sent from.
enum AppEvent {
    Toggle,
    Show,
    Quit,
    /// Rates arrived, so currency lines need recalculating.
    RatesRefreshed,
    UpdateChecked(UpdateStatus),
}

/// What's saved between runs. The hotkey is saved as key and modifiers
/// because `HotKey`'s own parser can't read back "LaunchApp2" (the
/// Calculator key). Every field has a default, so a missing one never
/// discards the rest.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct SavedState {
    high_precision: bool,
    hotkey_code: Option<String>,
    hotkey_mods: u32,
    /// Kept apart from the tabs, so clearing a document never loses them.
    converters: Vec<ConverterRow>,
    tabs: Vec<Tab>,
    active: usize,
    always_on_top: bool,
}

struct SoosApp {
    /// Never empty; `close_tab` replaces the last tab with an empty one.
    tabs: Vec<Tab>,
    /// Always a valid index into `tabs`.
    active: usize,
    /// Closed tabs, newest last, for Cmd/Ctrl+Shift+T. Not saved.
    closed: Vec<Tab>,
    next_tab_id: u64,
    /// The active tab's results, one per line -- possibly for an older
    /// version of its text while a recalculation runs.
    results: Vec<LineResult>,
    recalculator: Recalculator,
    /// The newest recalculation asked for, and the one `results` are from.
    requested: u64,
    received: u64,
    requested_at: Option<Instant>,
    /// When the text last changed, for holding back an error on the line
    /// being typed.
    last_edit: Option<Instant>,
    /// The tab the editor last took the keyboard for.
    focused_tab: Option<u64>,
    converters: Vec<ConverterRow>,
    /// One per converter row: its value or why it's invalid.
    converter_results: Vec<Result<String, String>>,
    rates: RateSource,
    /// What the newest recalculation was asked for.
    last_text: String,
    last_converters: Vec<ConverterRow>,
    /// What the converters export for soos-cli last held.
    exported_converters: Vec<soos_core::RawConverter>,
    /// Why the last converters export failed, for the status bar.
    export_error: Option<String>,
    show_converters: bool,
    /// The window's size before it was widened for the converters table.
    size_before_converters: Option<egui::Vec2>,
    /// Put the cursor in this row of the converters table.
    focus_converter: Option<usize>,
    /// False while hidden to the tray.
    visible: bool,
    /// macOS: the hidden window has lost focus, so focus returning means
    /// the Dock restored it (see `logic`).
    hidden_unfocused: bool,
    /// The tray's Quit was chosen, so the close really closes.
    quitting: bool,
    started: bool,
    high_precision: bool,
    always_on_top: bool,
    current_hotkey: HotKey,
    /// Waiting for a key press to become the new hotkey.
    capturing_hotkey: bool,
    /// Why the last hotkey couldn't be used.
    hotkey_message: Option<String>,
    show_example: bool,
    update_status: Option<UpdateStatus>,
    /// The last copied result and when, for the status bar's "Copied".
    copied: Option<(String, Instant)>,
    // Dropping these removes the tray icon and unregisters the hotkey.
    tray: Option<TrayIcon>,
    hotkeys: Option<GlobalHotKeyManager>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl SoosApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let saved = cc
            .storage
            .and_then(|s| eframe::get_value::<SavedState>(s, eframe::APP_KEY))
            .unwrap_or_default();
        let rates = RateSource::new(soos_core::currency::default_cache_path());
        let recalculator = Recalculator::spawn(cc.egui_ctx.clone());
        let mut app = Self::from_saved(saved, rates, recalculator);
        app.recalc(WAIT_AT_LAUNCH);
        app.last_edit = None;
        app.export_converters();
        app
    }

    fn from_saved(saved: SavedState, rates: RateSource, recalculator: Recalculator) -> Self {
        let (tx, rx) = mpsc::channel();
        let current_hotkey = load_hotkey(&saved);
        let (tabs, next_tab_id) = load_tabs(saved.tabs);
        let active = saved.active.min(tabs.len() - 1);
        Self {
            tabs,
            active,
            closed: Vec::new(),
            next_tab_id,
            results: Vec::new(),
            recalculator,
            requested: 0,
            received: 0,
            requested_at: None,
            last_edit: None,
            focused_tab: None,
            converters: saved.converters,
            converter_results: Vec::new(),
            rates,
            last_text: String::new(),
            last_converters: Vec::new(),
            exported_converters: Vec::new(),
            export_error: None,
            show_converters: false,
            size_before_converters: None,
            focus_converter: None,
            visible: true,
            hidden_unfocused: false,
            quitting: false,
            started: false,
            high_precision: saved.high_precision,
            always_on_top: saved.always_on_top,
            current_hotkey,
            capturing_hotkey: false,
            hotkey_message: None,
            show_example: false,
            update_status: None,
            copied: None,
            tray: None,
            hotkeys: None,
            tx,
            rx,
        }
    }

    fn active_text(&self) -> &str {
        &self.tabs[self.active].text
    }

    /// Ask for a recalculation if the text (or tab) or the converters
    /// changed, and take its result if it's ready within `wait`. Called
    /// every frame.
    fn recalc(&mut self, wait: std::time::Duration) {
        let text_changed = self.active_text() != self.last_text;
        if text_changed || self.converters != self.last_converters {
            if text_changed {
                self.last_edit = Some(Instant::now());
            }
            self.force_recalc();
        }
        if self.received == self.requested {
            return;
        }
        if let Some(done) = self.recalculator.take(self.requested, wait) {
            self.received = done.generation;
            self.results = done.results;
            self.converter_results = done.converter_results;
        }
    }

    fn force_recalc(&mut self) {
        let raw: Vec<soos_core::RawConverter> =
            self.converters.iter().map(ConverterRow::to_raw).collect();
        self.requested += 1;
        self.requested_at = Some(Instant::now());
        self.recalculator.submit(
            self.requested,
            self.active_text().to_string(),
            raw,
            self.rates.clone(),
        );
        self.last_text = self.active_text().to_string();
        self.last_converters = self.converters.clone();
    }

    /// Whether `results` have been out of date for [`STALE_AFTER`], so
    /// they're drawn dimmed. Until then, a repaint is scheduled for when
    /// they would be.
    fn results_stale(&self, ctx: &egui::Context) -> bool {
        if self.received == self.requested {
            return false;
        }
        let waited = self.requested_at.map_or(STALE_AFTER, |at| at.elapsed());
        match STALE_AFTER.checked_sub(waited) {
            Some(left) if !left.is_zero() => {
                ctx.request_repaint_after(left);
                false
            }
            _ => true,
        }
    }

    /// Write the converters file soos-cli reads, if it changed. Called at
    /// startup, when the converters overlay closes, and when eframe saves;
    /// not per keystroke, since each write is flushed to disk.
    fn export_converters(&mut self) {
        let raw: Vec<soos_core::RawConverter> =
            self.converters.iter().map(ConverterRow::to_raw).collect();
        if raw == self.exported_converters {
            return;
        }
        let path = soos_core::storage::converters_path();
        match soos_core::storage::save_converters(&path, &raw) {
            Ok(()) => {
                self.exported_converters = raw;
                self.export_error = None;
            }
            Err(e) => {
                self.export_error = Some(format!(
                    "Couldn't save converters for soos-cli to {}: {e}",
                    path.display()
                ))
            }
        }
    }
}

impl eframe::App for SoosApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.export_converters();
        let saved = SavedState {
            high_precision: self.high_precision,
            hotkey_code: Some(self.current_hotkey.key.to_string()),
            hotkey_mods: self.current_hotkey.mods.bits(),
            converters: self.converters.clone(),
            tabs: self.tabs.clone(),
            active: self.active,
            always_on_top: self.always_on_top,
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }

    /// Runs every frame, including while the window is hidden (when eframe
    /// skips `ui()`), so the tray, hotkey and rate refresh keep working.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.started {
            self.started = true;
            self.apply_always_on_top(ctx);
            self.init_tray_and_hotkey(ctx);
        }

        while let Ok(event) = self.rx.try_recv() {
            match event {
                AppEvent::Toggle => {
                    let visible = !self.visible;
                    self.set_visible(ctx, visible);
                }
                AppEvent::Show => self.set_visible(ctx, true),
                AppEvent::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                AppEvent::RatesRefreshed => self.force_recalc(),
                AppEvent::UpdateChecked(status) => self.update_status = Some(status),
            }
        }

        // Closing hides to the tray, but only if there is a tray to come
        // back from; otherwise it quits.
        let hide_on_close = self.tray.is_some() && !self.quitting;
        if hide_on_close && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.set_visible(ctx, false);
        }

        // macOS: egui never clears its `minimized` flag after the Dock
        // restores the window, so it would keep skipping `ui()`. Focus
        // coming back after a real loss of focus is the restore.
        if cfg!(target_os = "macos") && !self.visible {
            match ctx.input(|i| i.viewport().focused) {
                Some(false) => self.hidden_unfocused = true,
                Some(true) if self.hidden_unfocused => self.set_visible(ctx, true),
                _ => {}
            }
        }

        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        self.rates.refresh_in_background(move || {
            let _ = tx.send(AppEvent::RatesRefreshed);
            repaint_ctx.request_repaint();
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let palette = Palette::of(ui.ctx().theme());
        if !self.capturing_hotkey {
            self.handle_tab_shortcuts(ui.ctx());
        }
        // The tab strip and the document span the full width; each part
        // keeps `APP_PADDING` inside itself, so the document's scrollbar
        // runs along the window's edge.
        self.tab_bar(ui, palette);
        egui::Frame::central_panel(&ui.style().clone())
            .fill(palette.background)
            .inner_margin(0)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                self.status_bar(ui, palette);
                self.document(ui, palette);
            });

        if self.show_example {
            let modal = soos_modal(ui, palette, "example-doc", EXAMPLE_MODAL_WIDTH, |ui| {
                example_preview(ui, palette);
                // The preview itself takes no clicks, so without this a
                // click on it wouldn't close it.
                ui.interact(ui.min_rect(), ui.id().with("dismiss"), egui::Sense::click())
                    .clicked()
            });
            if modal.should_close() || modal.inner {
                self.show_example = false;
            }
        }

        self.fit_window_to_overlay(ui.ctx());
        if self.show_converters {
            let modal = soos_modal(ui, palette, "converters", CONVERTERS_GRID_WIDTH, |ui| {
                converters_window(
                    ui,
                    palette,
                    &mut self.converters,
                    &self.converter_results,
                    &mut self.focus_converter,
                );
            });
            // Clicks inside edit the table, so only Esc or a click outside
            // closes it.
            if modal.should_close() {
                self.show_converters = false;
                self.export_converters();
            }
        }

        self.recalc(WAIT_FOR_RESULT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory `eframe::Storage`, to save and load the way eframe does.
    #[derive(Default)]
    struct MemoryStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    /// v1.0.0 saved a `text` field that no longer exists. Its saves must
    /// still load: a failed load falls back to `SavedState::default()`,
    /// which would open with every tab gone.
    #[test]
    fn a_v1_0_0_save_still_loads() {
        #[derive(serde::Serialize)]
        struct SavedStateAtV1 {
            text: String,
            high_precision: bool,
            hotkey_code: Option<String>,
            hotkey_mods: u32,
            converters: Vec<ConverterRow>,
            tabs: Vec<Tab>,
            active: usize,
            always_on_top: bool,
        }
        let tab = |id, text: &str| Tab {
            id,
            text: text.to_string(),
        };
        let mut storage = MemoryStorage::default();
        eframe::set_value(
            &mut storage,
            eframe::APP_KEY,
            &SavedStateAtV1 {
                text: String::new(),
                high_precision: true,
                hotkey_code: Some("KeyK".to_string()),
                hotkey_mods: 0,
                converters: Vec::new(),
                tabs: vec![tab(0, "1 + 1"), tab(3, "rent = 1800")],
                active: 1,
                always_on_top: true,
            },
        );
        let saved: SavedState =
            eframe::get_value(&storage, eframe::APP_KEY).expect("a v1.0.0 save loads");
        assert_eq!(saved.tabs.len(), 2);
        assert_eq!(saved.tabs[1].text, "rent = 1800");
        assert_eq!(saved.active, 1);
        assert!(saved.high_precision && saved.always_on_top);
    }
}
