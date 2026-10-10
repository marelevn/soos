//! The status bar: the version, the exchange-rate status, one message slot
//! in the middle, and the toggles on the right.

use std::time::Duration;

use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Color32, Theme, ViewportCommand};

use crate::editor::APP_PADDING;
use crate::style::{cap_center_offset, mono, status_symbol, status_text, Palette};
use crate::tray::FULL_DESKTOP;
use crate::update::{open_url, update_message};
use crate::SoosApp;

/// How long "Copied ..." stays in the message slot.
const COPIED_FOR: Duration = Duration::from_millis(1500);
/// Rates older than this get an age warning.
const RATES_OLD_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The message in the middle of the status bar.
pub(crate) struct StatusMessage {
    pub(crate) text: String,
    pub(crate) color: Color32,
    /// Clicking the message opens this.
    pub(crate) url: Option<String>,
}

impl SoosApp {
    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui, palette: Palette) {
        #[cfg(not(target_os = "linux"))]
        if self.capturing_hotkey {
            self.handle_hotkey_capture(ui.ctx());
        }
        egui::Panel::bottom("status")
            .frame(
                // The page's side margin, so the bar lines up with the
                // editor, and its bottom margin below the bar.
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: APP_PADDING,
                        right: APP_PADDING,
                        top: 2,
                        bottom: 2 + APP_PADDING,
                    })
                    .fill(palette.background),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                let mut free = (0.0, 0.0);
                let row = ui
                    .horizontal(|ui| {
                        let version = status_symbol(
                            ui,
                            concat!("v", env!("CARGO_PKG_VERSION")),
                            palette.comment,
                            palette,
                        );
                        if version.on_hover_text("Check for updates").clicked() {
                            self.check_for_update(ui.ctx());
                        }
                        self.rates_status(ui, palette);
                        let left_end = ui.min_rect().right();
                        // Right to left, so this reads "↔ ? ▲ ± ◌ ⌨".
                        let icons = ui
                            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                #[cfg(not(target_os = "linux"))]
                                self.hotkey_control(ui, palette);
                                theme_control(ui, palette);
                                self.high_precision_control(ui, palette);
                                self.decimal_comma_control(ui, palette);
                                if FULL_DESKTOP {
                                    self.always_on_top_control(ui, palette);
                                }
                                self.example_control(ui, palette);
                                self.converters_control(ui, palette);
                            })
                            .response
                            .rect;
                        free = (left_end, icons.left());
                    })
                    .response
                    .rect;
                if let Some(message) = self.status_message(ui.ctx(), palette) {
                    paint_message(ui, row, free, message);
                }
            });
    }

    /// "rates offline" with no rates at all, or the rates' age once they're
    /// a day old; nothing while they're current.
    fn rates_status(&self, ui: &mut egui::Ui, palette: Palette) {
        let (text, tooltip) = match self.rates.age() {
            None => (
                "rates offline".to_string(),
                "Exchange rates haven't been downloaded yet. Soos keeps trying while it's open."
                    .to_string(),
            ),
            Some(age) if age > RATES_OLD_AFTER => {
                let days = age.as_secs() / (24 * 60 * 60);
                let days = if days == 1 {
                    "1 day".to_string()
                } else {
                    format!("{days} days")
                };
                (
                    format!("rates {days} old"),
                    format!("Currency results use exchange rates from {days} ago."),
                )
            }
            Some(_) => return,
        };
        status_text(ui, &text, palette.comment).on_hover_text(tooltip);
    }

    /// A hotkey prompt or error first, then a failed converters export, a
    /// recent copy, the result of an update check, and last the launch
    /// warning that the saved tabs couldn't be read.
    fn status_message(&self, ctx: &egui::Context, palette: Palette) -> Option<StatusMessage> {
        let plain = |text: String, color: Color32| StatusMessage {
            text,
            color,
            url: None,
        };
        if let Some(message) = &self.hotkey_message {
            return Some(plain(message.clone(), palette.error));
        }
        if self.capturing_hotkey {
            return Some(plain(
                "Press the new hotkey, or Esc to cancel".to_string(),
                palette.plain,
            ));
        }
        if let Some(error) = &self.export_error {
            return Some(plain(error.clone(), palette.error));
        }
        if let Some((copied, at)) = &self.copied {
            if let Some(left) = COPIED_FOR.checked_sub(at.elapsed()) {
                ctx.request_repaint_after(left);
                return Some(plain(format!("Copied {copied}"), palette.comment));
            }
        }
        self.update_status
            .as_ref()
            .map(|status| update_message(status, palette))
            .or_else(|| {
                let warning = self.load_warning.clone()?;
                Some(plain(warning, palette.error))
            })
    }

    fn high_precision_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let tooltip = if self.high_precision {
            "High precision is on: currencies keep every digit"
        } else {
            "High precision is off: currencies are rounded"
        };
        if symbol_toggle(
            ui,
            palette,
            "\u{b1}",
            self.high_precision,
            palette.result,
            tooltip,
        ) {
            self.high_precision = !self.high_precision;
        }
    }

    /// The glyph shows how a decimal reads now. Recalculates, since the
    /// text sent to the engine changes with it, though the text itself
    /// doesn't.
    fn decimal_comma_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let (glyph, tooltip) = if self.decimal_comma {
            (
                "0,1",
                "Decimal comma is on: 1.234,5. Click for a decimal point, 1,234.5",
            )
        } else {
            (
                "0.1",
                "Decimal point is on: 1,234.5. Click for a decimal comma, 1.234,5",
            )
        };
        if symbol_toggle(
            ui,
            palette,
            glyph,
            self.decimal_comma,
            palette.result,
            tooltip,
        ) {
            self.decimal_comma = !self.decimal_comma;
            self.force_recalc();
        }
    }

    fn always_on_top_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let tooltip = if self.always_on_top {
            "Always on top is on"
        } else {
            "Always on top is off"
        };
        if symbol_toggle(
            ui,
            palette,
            "\u{25b2}",
            self.always_on_top,
            palette.keyword,
            tooltip,
        ) {
            self.always_on_top = !self.always_on_top;
            self.apply_always_on_top(ui.ctx());
        }
    }

    pub(crate) fn apply_always_on_top(&self, ctx: &egui::Context) {
        let level = if self.always_on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
    }

    fn example_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let tooltip = if self.show_example {
            "Hide the example"
        } else {
            "Show an example of what Soos understands"
        };
        if symbol_toggle(
            ui,
            palette,
            "?",
            self.show_example,
            palette.keyword,
            tooltip,
        ) {
            self.show_example = !self.show_example;
            if self.show_example {
                // So the document doesn't take keystrokes behind the overlay.
                ui.ctx().memory_mut(|m| m.stop_text_input());
            }
        }
    }

    fn converters_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if symbol_toggle(
            ui,
            palette,
            "\u{2194}",
            self.show_converters,
            palette.keyword,
            "Converters: define your own units",
        ) {
            self.show_converters = !self.show_converters;
            if self.show_converters {
                // Only on opening: stopping text input every frame would
                // make the table impossible to type in.
                ui.ctx().memory_mut(|m| m.stop_text_input());
                self.focus_converter = Some(0);
            }
        }
    }
}

/// A status-bar symbol in `palette.comment`, fading to `on_color` while
/// `on`. Returns whether it was clicked.
fn symbol_toggle(
    ui: &mut egui::Ui,
    palette: Palette,
    glyph: &str,
    on: bool,
    on_color: Color32,
    tooltip: &str,
) -> bool {
    let t = ui
        .ctx()
        .animate_bool_with_time(ui.id().with(glyph), on, 0.12);
    let color = palette.comment.lerp_to_gamma(on_color, t);
    status_symbol(ui, glyph, color, palette)
        .on_hover_text(tooltip)
        .clicked()
}

/// Cycles the theme: follow the system, light, dark. egui persists the
/// choice.
fn theme_control(ui: &mut egui::Ui, palette: Palette) {
    use egui::ThemePreference;
    let preference = ui.ctx().options(|o| o.theme_preference);
    let effective = match ui.ctx().theme() {
        Theme::Dark => "dark",
        Theme::Light => "light",
    };
    let (glyph, current, next, next_name) = match preference {
        ThemePreference::System => (
            "\u{25cc}",
            format!("follows the system ({effective})"),
            ThemePreference::Light,
            "light",
        ),
        ThemePreference::Light => (
            "\u{25cb}",
            "light".to_string(),
            ThemePreference::Dark,
            "dark",
        ),
        ThemePreference::Dark => (
            "\u{25cf}",
            "dark".to_string(),
            ThemePreference::System,
            "the system's",
        ),
    };
    let tooltip = format!("Theme: {current}. Click for {next_name}.");
    if symbol_toggle(ui, palette, glyph, false, palette.keyword, &tooltip) {
        ui.ctx().set_theme(next);
    }
}

/// Paints `message` centred on `row`, within the free span `(left, right)`
/// between the version and the icons; a message too long for it is cut,
/// with the full text on hover. Painted over the row rather than laid out
/// in it, so the row doesn't change size as messages come and go.
fn paint_message(ui: &mut egui::Ui, row: egui::Rect, free: (f32, f32), message: StatusMessage) {
    const GAP: f32 = 16.0;
    let (left, right) = (free.0 + GAP, free.1 - GAP);
    let mut job = LayoutJob::simple(message.text.clone(), mono(), message.color, f32::INFINITY);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(0.0));
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let width = galley.rect.width();
    let x = (row.center().x - width / 2.0).clamp(left, (right - width).max(left));
    let pos = egui::pos2(
        x,
        row.center().y - galley.rect.height() / 2.0 + cap_center_offset(ui),
    );
    let rect = egui::Rect::from_min_size(pos, galley.rect.size());
    let elided = galley.elided;
    ui.painter().galley(pos, galley, message.color);
    let sense = if message.url.is_some() {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let mut response = ui.interact(rect, ui.id().with("message"), sense);
    if elided {
        response = response.on_hover_text(&message.text);
    }
    if let Some(url) = message.url {
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Open the release page");
        if response.clicked() {
            open_url(&url);
        }
    }
}
