//! Soos: a document where every line is an expression and its answer shows
//! up beside it. The modules are the parts of the window: [`tabs`] across
//! the top, [`editor`] in the middle, [`status_bar`] at the bottom, and
//! the overlays behind it ([`example`], [`converters`]).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{self, Theme, ViewportCommand};
#[cfg(not(target_os = "linux"))]
use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use soos_core::{currency::RateSource, format::swap_separators, LineResult};
use tray_icon::TrayIcon;

mod converters;
mod editor;
mod example;
#[cfg(not(target_os = "linux"))]
mod hotkey;
mod recalc;
mod status_bar;
mod style;
mod tabs;
mod tray;
mod update;
mod window;

use converters::{converters_window, ConverterRow, CONVERTERS_GRID_WIDTH};
use example::example_preview;
#[cfg(not(target_os = "linux"))]
use hotkey::load_hotkey;
use recalc::{Recalculator, STALE_AFTER, WAIT_AT_LAUNCH, WAIT_FOR_RESULT};
use style::{app_visuals, install_fonts, Palette};
use tabs::{load_tabs, Tab};
use tray::{restored_by_the_system, window_icon, FULL_DESKTOP, MINIMIZE_SETTLES};
use update::UpdateStatus;
use window::{
    soos_modal, DEFAULT_HEIGHT, DEFAULT_WIDTH, EXAMPLE_MODAL_WIDTH, MAIN_MIN_HEIGHT, MAIN_MIN_WIDTH,
};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([DEFAULT_WIDTH, DEFAULT_HEIGHT])
            .with_min_inner_size([MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT])
            .with_icon(window_icon()),
        persistence_path: Some(app_state_path()),
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

/// `text` in the decimal mark the app is set to: with the decimal comma on,
/// `.` and `,` swapped in its numbers. The same swap reads what the user
/// types and shows what the engine prints (see [`swap_separators`]).
fn in_decimal_mark(text: &str, decimal_comma: bool) -> String {
    if decimal_comma {
        swap_separators(text)
    } else {
        text.to_string()
    }
}

/// Where eframe keeps the tabs and settings.
fn app_state_path() -> std::path::PathBuf {
    soos_core::storage::data_dir().join("app.ron")
}

/// Moves a save the app couldn't read to `app.ron.bak`, and says so for the
/// status bar. eframe treats an unreadable file as an empty one, so the next
/// autosave would otherwise replace every tab with a blank one. `None` when
/// there is no save to lose.
fn keep_unreadable_save(path: &std::path::Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() == 0 {
        return None;
    }
    // Never over an older backup: two unreadable saves in a row keep both.
    let backup = std::iter::once(path.with_extension("ron.bak"))
        .chain((2..).map(|n| path.with_extension(format!("ron.bak.{n}"))))
        .find(|candidate| !candidate.exists())?;
    let kept_as = backup
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    Some(match std::fs::rename(path, &backup) {
        Ok(_) => format!("Couldn't read your saved tabs. The old file is kept as {kept_as}."),
        Err(_) => {
            "Couldn't read your saved tabs, or move the file aside. Copy app.ron somewhere safe before Soos saves again.".to_string()
        }
    })
}

/// A file in the data folder, there while the first recalculation after a
/// launch runs. A line that overflows fend's stack ends the process (that
/// can't be caught), and its tab is saved, so Soos would close again at
/// every launch. Found at launch, it means the last one closed that way:
/// that tab isn't calculated until it's edited.
const LAUNCH_MARKER: &str = "recalculating";

/// Whether the last launch closed during its first recalculation, going by
/// `marker`, which is left in place, or written, for this launch's.
fn closed_while_calculating(marker: &std::path::Path) -> bool {
    let closed = marker.exists();
    if !closed {
        let _ = std::fs::write(marker, "");
    }
    closed
}

/// From the tray, hotkey and background threads to `logic()`, the only
/// place viewport commands may be sent from.
enum AppEvent {
    #[cfg(not(target_os = "linux"))]
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
    /// `,` is the decimal mark and `.` groups thousands (see
    /// [`SoosApp::decimal_comma`]).
    decimal_comma: bool,
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
    /// Set at launch when the saved tabs couldn't be read, or Soos closed
    /// while calculating, for the status bar.
    load_warning: Option<String>,
    /// The tab, and its text, that Soos closed while calculating at the
    /// last launch (see [`LAUNCH_MARKER`]): it isn't calculated until the
    /// text changes.
    paused: Option<(u64, String)>,
    /// This launch's marker, removed when the first results arrive.
    launch_marker: Option<std::path::PathBuf>,
    show_converters: bool,
    /// The window's size before it was widened for the converters table.
    size_before_converters: Option<egui::Vec2>,
    /// Put the cursor in this row of the converters table.
    focus_converter: Option<usize>,
    /// False while hidden to the tray, or minimized by closing on macOS.
    visible: bool,
    /// When closing minimized the window (macOS), until it is shown again.
    minimized_at: Option<Instant>,
    /// The tray's Quit was chosen, so the close really closes.
    quitting: bool,
    started: bool,
    high_precision: bool,
    /// Swaps `.` and `,` in what the user types and in the results shown,
    /// and nowhere else: the engine, `prev` and soos-cli's converters export
    /// always use `1,234.5`. The saved text is as typed.
    decimal_comma: bool,
    always_on_top: bool,
    #[cfg(not(target_os = "linux"))]
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
    #[cfg(not(target_os = "linux"))]
    hotkeys: Option<GlobalHotKeyManager>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl SoosApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let loaded = cc
            .storage
            .and_then(|s| eframe::get_value::<SavedState>(s, eframe::APP_KEY));
        let load_warning = match loaded {
            Some(_) => None,
            None => keep_unreadable_save(&app_state_path()),
        };
        let rates = RateSource::new(soos_core::currency::default_cache_path());
        let recalculator = Recalculator::spawn(cc.egui_ctx.clone());
        let mut app = Self::from_saved(loaded.unwrap_or_default(), rates, recalculator);
        app.load_warning = load_warning;
        let marker = soos_core::storage::data_dir().join(LAUNCH_MARKER);
        if closed_while_calculating(&marker) {
            let tab = &app.tabs[app.active];
            app.paused = Some((tab.id, tab.text.clone()));
            app.load_warning = app.load_warning.or_else(|| {
                Some(
                    "Soos closed while calculating this tab, so it waits until you edit it."
                        .to_string(),
                )
            });
        }
        app.launch_marker = Some(marker);
        app.recalc(WAIT_AT_LAUNCH);
        app.last_edit = None;
        app.export_converters();
        app
    }

    fn from_saved(saved: SavedState, rates: RateSource, recalculator: Recalculator) -> Self {
        let (tx, rx) = mpsc::channel();
        #[cfg(not(target_os = "linux"))]
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
            load_warning: None,
            paused: None,
            launch_marker: None,
            show_converters: false,
            size_before_converters: None,
            focus_converter: None,
            visible: true,
            minimized_at: None,
            quitting: false,
            started: false,
            high_precision: saved.high_precision,
            decimal_comma: saved.decimal_comma,
            always_on_top: saved.always_on_top,
            #[cfg(not(target_os = "linux"))]
            current_hotkey,
            capturing_hotkey: false,
            hotkey_message: None,
            show_example: false,
            update_status: None,
            copied: None,
            tray: None,
            #[cfg(not(target_os = "linux"))]
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
        if let Some((id, text)) = &self.paused {
            let tab = &self.tabs[self.active];
            if tab.id == *id && tab.text == *text {
                self.results.clear();
                return;
            }
            if tab.id == *id {
                self.paused = None;
            }
        }
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
            if let Some(marker) = self.launch_marker.take() {
                let _ = std::fs::remove_file(marker);
            }
        }
    }

    /// The converters as the engine reads them. With the decimal comma on,
    /// a factor or base typed `0,3048` is `0.3048`; without this it would be
    /// 3048, silently. The soos-cli export goes through here too.
    fn raw_converters(&self) -> Vec<soos_core::RawConverter> {
        self.converters
            .iter()
            .map(|row| {
                let mut raw = row.to_raw();
                raw.factor = in_decimal_mark(&raw.factor, self.decimal_comma);
                raw.base = in_decimal_mark(&raw.base, self.decimal_comma);
                raw
            })
            .collect()
    }

    fn force_recalc(&mut self) {
        let text = in_decimal_mark(self.active_text(), self.decimal_comma);
        self.requested += 1;
        self.requested_at = Some(Instant::now());
        self.recalculator.submit(
            self.requested,
            text,
            self.raw_converters(),
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
        let raw = self.raw_converters();
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
            decimal_comma: self.decimal_comma,
            converters: self.converters.clone(),
            tabs: self.tabs.clone(),
            active: self.active,
            always_on_top: self.always_on_top,
            ..Default::default()
        };
        #[cfg(not(target_os = "linux"))]
        let saved = SavedState {
            hotkey_code: Some(self.current_hotkey.key.to_string()),
            hotkey_mods: self.current_hotkey.mods.bits(),
            ..saved
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }

    /// Runs every frame, including while the window is hidden (when eframe
    /// skips `ui()`), so the tray, hotkey and rate refresh keep working.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.started {
            self.started = true;
            if FULL_DESKTOP {
                self.apply_always_on_top(ctx);
            }
            self.init_tray(ctx);
            #[cfg(not(target_os = "linux"))]
            self.init_hotkey(ctx);
        }

        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        let minimized_for = self.minimized_at.map(|at| at.elapsed());
        if cfg!(target_os = "macos") && restored_by_the_system(self.visible, minimized_for, focused)
        {
            self.visible = true;
            self.minimized_at = None;
            // Undo egui's stale `Minimized(true)`; see `restored_by_the_system`.
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        }

        while let Ok(event) = self.rx.try_recv() {
            match event {
                #[cfg(not(target_os = "linux"))]
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
        // back from and the window can hide (see `FULL_DESKTOP`); otherwise it
        // quits. On macOS it minimizes instead: a click on the Dock icon
        // brings a minimized window back, but not a hidden one.
        let hide_on_close = self.tray.is_some() && FULL_DESKTOP && !self.quitting;
        if hide_on_close && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if cfg!(target_os = "macos") {
                self.visible = false;
                self.minimized_at = Some(Instant::now());
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                // A frame after the minimize has settled, to look for a restore.
                ctx.request_repaint_after(MINIMIZE_SETTLES + std::time::Duration::from_millis(100));
            } else {
                self.set_visible(ctx, false);
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
                example_preview(ui, palette, self.decimal_comma);
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
                    self.decimal_comma,
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
    use soos_core::format::result_cells;

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

    /// A save with a `text` field, which `SavedState` no longer has, still
    /// loads: a failed load falls back to `SavedState::default()`, which
    /// would open with every tab gone.
    #[test]
    fn a_save_with_a_removed_field_still_loads() {
        #[derive(serde::Serialize)]
        struct SavedStateWithText {
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
            &SavedStateWithText {
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
        let saved: SavedState = eframe::get_value(&storage, eframe::APP_KEY)
            .expect("a save with a removed field loads");
        assert_eq!(saved.tabs.len(), 2);
        assert_eq!(saved.tabs[1].text, "rent = 1800");
        assert_eq!(saved.active, 1);
        assert!(saved.high_precision && saved.always_on_top);
        // A save without the setting has the point.
        assert!(!saved.decimal_comma);
    }

    /// eframe reads an unreadable file as an empty one, and the app then
    /// opens with one blank tab: the next autosave would overwrite the
    /// user's documents, so the file is kept first.
    #[test]
    fn an_unreadable_save_is_kept_before_the_next_autosave() {
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, eframe::APP_KEY, &"not a saved state");
        assert!(eframe::get_value::<SavedState>(&storage, eframe::APP_KEY).is_none());

        let dir = std::env::temp_dir().join(format!("soos-app-keep-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.ron");

        // No save yet, or an empty one: nothing to lose.
        assert_eq!(keep_unreadable_save(&path), None);
        std::fs::write(&path, "").unwrap();
        assert_eq!(keep_unreadable_save(&path), None);

        std::fs::write(&path, "({\"app\": \"garbage").unwrap();
        let message = keep_unreadable_save(&path).expect("a message");
        assert!(message.contains("app.ron.bak"), "{message}");
        assert_eq!(
            std::fs::read_to_string(dir.join("app.ron.bak")).unwrap(),
            "({\"app\": \"garbage"
        );
        assert!(!path.exists(), "the next autosave starts a new file");

        // A second unreadable save doesn't replace the first backup.
        std::fs::write(&path, "({\"app\": \"more garbage").unwrap();
        let message = keep_unreadable_save(&path).expect("a message");
        assert!(message.contains("app.ron.bak.2"), "{message}");
        assert_eq!(
            std::fs::read_to_string(dir.join("app.ron.bak")).unwrap(),
            "({\"app\": \"garbage"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A launch that finds the marker says so, and the tab Soos closed on
    /// isn't calculated until it's edited; other tabs are.
    #[test]
    fn a_tab_soos_closed_on_waits_for_an_edit() {
        let dir = std::env::temp_dir().join(format!("soos-app-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join(LAUNCH_MARKER);
        assert!(!closed_while_calculating(&marker), "a first launch");
        assert!(marker.exists(), "written for this launch");
        assert!(closed_while_calculating(&marker), "it never finished");
        let _ = std::fs::remove_dir_all(&dir);

        let mut app = app(false, Vec::new());
        app.tabs[app.active].text = "1 + 1".to_string();
        app.paused = Some((app.tabs[app.active].id, "1 + 1".to_string()));
        app.recalc(std::time::Duration::ZERO);
        assert_eq!(app.requested, 0, "nothing sent to the engine");
        app.tabs[app.active].text = "1 + 2".to_string();
        app.recalc(std::time::Duration::from_secs(5));
        assert!(app.paused.is_none());
        assert_eq!(app.results, [LineResult::Value("3".to_string())]);
    }

    fn app(decimal_comma: bool, converters: Vec<ConverterRow>) -> SoosApp {
        let saved = SavedState {
            decimal_comma,
            converters,
            ..Default::default()
        };
        SoosApp::from_saved(
            saved,
            RateSource::new(std::path::PathBuf::new()),
            Recalculator::spawn(egui::Context::default()),
        )
    }

    /// Not through `SoosApp::save`, which also writes the converters export
    /// into the real data directory.
    #[test]
    fn the_decimal_comma_is_saved_and_loaded() {
        let mut storage = MemoryStorage::default();
        let saved = SavedState {
            decimal_comma: true,
            ..Default::default()
        };
        eframe::set_value(&mut storage, eframe::APP_KEY, &saved);
        let loaded: SavedState = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        assert!(app(loaded.decimal_comma, Vec::new()).decimal_comma);
    }

    /// What the user types is read in their notation, the engine and the
    /// export get the point, and the results come back in the point's form.
    #[test]
    fn the_decimal_comma_swaps_what_is_typed_before_the_engine_reads_it() {
        let mut app = app(true, Vec::new());
        app.tabs[0].text = "3,5 + 1\n3.000.000 / 2\nNovember 15, 2027".to_string();
        app.recalc(std::time::Duration::from_secs(5));
        assert_eq!(
            app.results[..2],
            [
                LineResult::Value("4.5".to_string()),
                LineResult::Value("1500000".to_string())
            ]
        );
        let cells = result_cells(&app.tabs[0].text, &app.results, false, true);
        assert_eq!(cells[0].as_ref().unwrap().shown.text, "4,5");
        assert_eq!(cells[1].as_ref().unwrap().shown.text, "1.500.000");
        // A date's comma isn't between digits, so the engine reads it as
        // typed, as with the point.
        let mut point = self::app(false, Vec::new());
        point.tabs[0].text = "November 15, 2027".to_string();
        point.recalc(std::time::Duration::from_secs(5));
        assert_eq!(app.results[2], point.results[0]);
    }

    #[test]
    fn the_decimal_comma_reads_a_converter_factor_and_the_export_keeps_the_point() {
        let row = ConverterRow {
            unit: "foot2".to_string(),
            aliases: String::new(),
            base: "m".to_string(),
            factor: "0,3048".to_string(),
        };
        assert_eq!(
            app(true, vec![row.clone()]).raw_converters()[0].factor,
            "0.3048"
        );
        // Off, the engine reads what was typed.
        assert_eq!(app(false, vec![row]).raw_converters()[0].factor, "0,3048");
    }
}
