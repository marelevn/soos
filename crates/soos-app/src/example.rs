//! The read-only worked example behind the status bar's `?`.

use eframe::egui;

use crate::editor::build_layout_job;
use crate::style::{text_format, Palette};
use crate::tray::FULL_DESKTOP;
use crate::window::modal_room;

/// Each line and the result the app shows for it. A fixed list, so the
/// overlay never depends on the network or the clock; the test below checks
/// every result against the engine with fixed rates and a fixed clock.
/// The basics come first and fit without scrolling.
pub(crate) const EXAMPLE_LINES: &[(&str, Option<&str>)] = &[
    ("# Welcome to Soos", None),
    ("", None),
    ("// Each line's answer shows up beside it.", None),
    ("", None),
    ("12 + 8 * 2", Some("28")),
    ("", None),
    ("// A label names a line. sum adds up the", None),
    ("// block above it; a blank line ends a block.", None),
    ("Flights: $840 * 2", Some("$1,680.00")),
    ("Hotel: 6 * $120", Some("$720.00")),
    ("Visas: 3 * $45", Some("$135.00")),
    ("sum", Some("$2,535.00")),
    ("", None),
    ("# Variables carry down the page", None),
    ("price = 32", Some("32")),
    ("price * 8", Some("256")),
    ("prev + 100", Some("356")),
    ("", None),
    ("# More", None),
    ("", None),
    ("// avg works like sum.", None),
    ("18", Some("18")),
    ("24", Some("24")),
    ("avg", Some("21")),
    ("", None),
    ("// Natural-language phrasing", None),
    ("5% on 30", Some("31.5")),
    ("6% off 40 EUR", Some("\u{20ac}37.60")),
    ("20% of what is 30 cm", Some("150 cm")),
    ("10 as a % of 40", Some("25%")),
    ("$8 times 3", Some("$24.00")),
    ("20 ml in tea spoons", Some("\u{2248} 4.0577 teaspoons")),
    ("", None),
    ("// A variable can hold a percent.", None),
    ("fee = 8%", Some("8%")),
    ("cost = 200", Some("200")),
    ("fee on cost", Some("216")),
    ("", None),
    ("# Currency and units", None),
    ("// Rates update daily; this one is an example.", None),
    ("1 USD to VND", Some("26,125 \u{20ab}")),
    ("20 inches in cm", Some("50.8 cm")),
    ("16 px to pt", Some("12 pt")),
    ("2rem to px", Some("32 px")),
    ("// pt is points here; write pint for pints.", None),
    ("// Define your own units with \u{2194} below.", None),
    ("", None),
    ("# Dates and time zones", None),
    ("today + 17 days", Some("Friday, 2 October 2026")),
    ("@2026-12-25", Some("Friday, 25 December 2026")),
    ("now in Tokyo", Some("Tuesday, 15 September 2026 08:45 JST")),
    (
        "9AM PST to Tokyo",
        Some("Tuesday, 15 September 2026 01:00 JST"),
    ),
    ("", None),
    ("# The status bar", None),
    ("// \u{2194} converters   ? this example", None),
    // Linux has no ▲ or ⌨; see `FULL_DESKTOP`.
    if FULL_DESKTOP {
        ("// \u{25b2} always on top   \u{b1} high precision", None)
    } else {
        ("// \u{b1} high precision", None)
    },
    if FULL_DESKTOP {
        ("// \u{25cc} theme   \u{2328} global hotkey", None)
    } else {
        ("// \u{25cc} theme", None)
    },
    ("// Hover over one for what it does.", None),
    ("", None),
    ("// Cmd/Ctrl+T new tab, Cmd/Ctrl+W close it,", None),
    ("// Cmd/Ctrl + - 0 zoom the text.", None),
];

/// Where the answer column starts, in characters: the longest expression
/// with a result (`20% of what is 30 cm`) plus a gap.
pub(crate) const EXAMPLE_COLUMN: usize = 24;

/// Tall enough to show the basics, with `# More` just below the fold.
pub(crate) const EXAMPLE_ROWS_MAX_HEIGHT: f32 = 460.0;

/// [`EXAMPLE_LINES`], syntax-coloured, with the results in a column.
pub(crate) fn example_preview(ui: &mut egui::Ui, palette: Palette) {
    let max_height = EXAMPLE_ROWS_MAX_HEIGHT.min(modal_room(ui.ctx()).y);
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            for (line, result) in EXAMPLE_LINES {
                let mut job = build_layout_job(line, f32::INFINITY, palette);
                if let Some(result) = result {
                    let pad = EXAMPLE_COLUMN.saturating_sub(line.chars().count());
                    job.append(&" ".repeat(pad.max(2)), 0.0, text_format(palette.plain));
                    job.append(result, 0.0, text_format(palette.result));
                }
                ui.add(egui::Label::new(job).selectable(false));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use soos_core::currency::RateSource;

    /// The whole list runs as one document, as in the app, at 23:45 UTC on
    /// 2026-09-14, with 1 EUR = 1 USD = 26,125 VND. `today` lines depend on
    /// this machine's zone, so they're only illustrative.
    #[test]
    fn example_lines_match_the_engine() {
        let rates = RateSource::with_rates(&[("EUR", 1.0), ("USD", 1.0), ("VND", 26125.0)]);
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_789_429_500);

        let document: Vec<&str> = EXAMPLE_LINES.iter().map(|(line, _)| *line).collect();
        let (_, results) = soos_core::recalc_document_at(&document.join("\n"), &[], &rates, now);
        let mut mismatches = Vec::new();
        for ((line, expected), result) in EXAMPLE_LINES.iter().zip(&results) {
            let Some(expected) = expected else { continue };
            if line.starts_with("today") {
                continue;
            }
            let actual = soos_core::format::shown(result, false).map(|s| s.text);
            if actual.as_deref() != Some(*expected) {
                mismatches.push(format!(
                    "{line:?}: shows {actual:?}, example says {expected:?}"
                ));
            }
        }
        assert!(mismatches.is_empty(), "\n{}", mismatches.join("\n"));
    }
}
