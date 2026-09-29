//! The document editor: one multiline `TextEdit` (so selection, undo and
//! the caret work as usual), syntax colours from its layouter, and each
//! line's result painted in a column beside it. A wrapped line's result sits
//! beside its first row.

use std::time::Duration;

use egui::text::{CCursor, CCursorRange};

use super::*;

/// Shown in an empty document.
pub(crate) const PLACEHOLDER: &str = "Type a calculation, like $120 - 15%";

/// How far out-of-date results fade toward the background.
const STALE_DIM: f32 = 0.55;

/// An error on the line being typed waits this long after the last
/// keystroke: half a line is usually an error (`5 k` before `5 kg`).
pub(crate) const ERROR_HOLD: Duration = Duration::from_millis(900);

/// The whole buffer as a coloured `LayoutJob`, using soos-core's
/// highlighter so colours follow what each line actually does.
pub(crate) fn build_layout_job(text: &str, wrap_width: f32, palette: Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            job.append("\n", 0.0, text_format(palette.plain));
        }
        let mut cursor = 0usize;
        for (range, kind) in soos_core::highlight::tokens(line) {
            if range.start > cursor {
                job.append(&line[cursor..range.start], 0.0, text_format(palette.plain));
            }
            let span = &line[range.clone()];
            match kind {
                // The `#` marks a heading; the heading itself is the
                // strongest text on the page, not the faintest.
                TokenKind::Header => {
                    let marker = span.len() - span.trim_start_matches(['#', ' ']).len();
                    job.append(&span[..marker], 0.0, text_format(palette.comment));
                    let heading = text_format_in(mono_bold(), palette.plain);
                    job.append(&span[marker..], 0.0, heading);
                }
                TokenKind::Comment => job.append(span, 0.0, text_format(palette.comment)),
                TokenKind::Label | TokenKind::ConversionWord => {
                    job.append(span, 0.0, text_format(palette.secondary));
                }
                TokenKind::Keyword => job.append(span, 0.0, text_format(palette.keyword)),
            }
            cursor = range.end;
        }
        if cursor < line.len() {
            job.append(&line[cursor..], 0.0, text_format(palette.plain));
        }
    }
    job
}

/// One line's result, ready for the result column. Its text is painted as
/// is, flush against the column's right edge: padding it to line decimal
/// points up would pull some results off that edge.
pub(crate) struct Cell {
    pub(crate) shown: Shown,
    /// A `sum` or `avg` line, drawn bold.
    pub(crate) total: bool,
}

/// A line whose result is a total: `sum`, `total`, `avg` or `average`.
pub(crate) fn is_total(line: &str) -> bool {
    soos_core::highlight::tokens(line)
        .into_iter()
        .any(|(range, kind)| {
            kind == TokenKind::Keyword
                && ["sum", "total", "avg", "average"]
                    .iter()
                    .any(|word| line[range.clone()].eq_ignore_ascii_case(word))
        })
}

/// A cell per line of `text`, `None` where the line shows nothing.
pub(crate) fn result_cells(
    text: &str,
    results: &[LineResult],
    high_precision: bool,
) -> Vec<Option<Cell>> {
    text.split('\n')
        .enumerate()
        .map(|(i, line)| {
            Some(Cell {
                shown: soos_core::format::shown(results.get(i)?, high_precision)?,
                total: is_total(line),
            })
        })
        .collect()
}

pub(crate) const MIN_GUTTER: f32 = 90.0;
/// So a long result can't squeeze out the expressions.
pub(crate) const MAX_GUTTER_FRACTION: f32 = 0.45;
pub(crate) const GUTTER_PAD: f32 = 12.0;
/// Between the expressions and the result column.
pub(crate) const COLUMN_GAP: f32 = 24.0;
/// Between the document's scrollbar and the window's edge.
const SCROLL_BAR_INSET: f32 = 3.0;
/// Around the page: `TextEdit::margin()` is ignored once the `TextEdit`
/// has `Frame::NONE`.
pub(crate) const APP_PADDING: i8 = 28;

/// A cell's width on the character grid.
fn cell_width(cell: &Cell) -> f32 {
    cell.shown.text.chars().count() as f32 * CHAR_WIDTH
}

/// The result column's width: the widest result, within limits. Errors
/// don't count; a long one is cut instead (see [`error_budget`]).
pub(crate) fn gutter_width(cells: &[Option<Cell>], available_width: f32) -> f32 {
    let widest_value = cells
        .iter()
        .flatten()
        .filter(|cell| cell.shown.error.is_none())
        .map(cell_width)
        .fold(0.0_f32, f32::max);
    (widest_value + GUTTER_PAD * 2.0).clamp(MIN_GUTTER, available_width * MAX_GUTTER_FRACTION)
}

/// An error may use the free space to the left on its own line, but never
/// less than the result column.
pub(crate) fn error_budget(gutter_right: f32, row_right: f32, gutter_budget: f32) -> f32 {
    (gutter_right - row_right - COLUMN_GAP).max(gutter_budget)
}

/// The line the caret is on, counting from 0.
fn caret_line(text: &str, cursor: Option<CCursorRange>) -> Option<usize> {
    let index = cursor?.primary.index.0;
    Some(text.chars().take(index).filter(|&c| c == '\n').count())
}

impl SoosApp {
    /// The active tab's text and its results. Clicking a result copies it.
    pub(crate) fn document(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let stale = self.results_stale(ui.ctx());
        let tab_id = egui::Id::new(("doc", self.tabs[self.active].id));
        self.keep_focus(ui.ctx(), tab_id);

        // The scroll area spans the window between the tab strip and the
        // status bar, with the page's margins inside it: its scrollbar runs
        // along the window's edge, over the right margin, and text scrolls
        // up to the tab strip instead of stopping short of it.
        let page_margin = egui::Margin {
            left: APP_PADDING,
            right: APP_PADDING,
            top: APP_PADDING,
            bottom: APP_PADDING / 2,
        };
        ui.scope(|ui| {
            let scroll = &mut ui.spacing_mut().scroll;
            scroll.floating_allocated_width = 0.0;
            // A hair inside the window's edge, as macOS draws overlay bars.
            scroll.bar_outer_margin = SCROLL_BAR_INSET;
            // Not shrunk to the content, which ends short of the right
            // margin, so the bar is at the window's edge.
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    egui::Frame::new().inner_margin(page_margin).show(ui, |ui| {
                        let available_width = ui.available_width();
                        let cells =
                            result_cells(self.active_text(), &self.results, self.high_precision);
                        let gutter_width = gutter_width(&cells, available_width);
                        let text_width = (available_width - gutter_width - COLUMN_GAP).max(100.0);
                        let mut layouter =
                            |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                                ui.fonts_mut(|f| {
                                    f.layout_job(build_layout_job(
                                        buf.as_str(),
                                        wrap_width,
                                        palette,
                                    ))
                                })
                            };

                        let output = egui::TextEdit::multiline(&mut self.tabs[self.active].text)
                            // Per tab, so Ctrl+Z can't undo into another tab.
                            .id(tab_id)
                            .frame(egui::Frame::NONE)
                            .desired_width(text_width)
                            // A click below the text still places the caret.
                            .min_size(egui::vec2(text_width, ui.available_height()))
                            .font(mono())
                            .layouter(&mut layouter)
                            .show(ui);

                        if self.tabs[self.active].text.is_empty() {
                            let hint = LayoutJob::single_section(
                                PLACEHOLDER.to_owned(),
                                text_format(palette.comment),
                            );
                            let galley = ui.fonts_mut(|f| f.layout_job(hint));
                            ui.painter()
                                .galley(output.galley_pos, galley, palette.comment);
                        }

                        let typing_line = self
                            .last_edit
                            .and_then(|at| ERROR_HOLD.checked_sub(at.elapsed()))
                            .filter(|left| !left.is_zero())
                            .and_then(|left| {
                                ui.ctx().request_repaint_after(left);
                                caret_line(&self.tabs[self.active].text, output.cursor_range)
                            });

                        let gutter_right =
                            output.response.rect.right() + COLUMN_GAP + gutter_width - GUTTER_PAD;
                        let gutter_budget = gutter_width - GUTTER_PAD;
                        let mut line = 0usize;
                        let mut starts_line = true;
                        for row in &output.galley.rows {
                            let this_line = line;
                            let first_row =
                                std::mem::replace(&mut starts_line, row.ends_with_newline);
                            if row.ends_with_newline {
                                line += 1;
                            }
                            let Some(Some(cell)) = cells.get(this_line).filter(|_| first_row)
                            else {
                                continue;
                            };
                            let is_error = cell.shown.error.is_some();
                            if is_error && typing_line == Some(this_line) {
                                continue;
                            }
                            let color = if is_error {
                                palette.error
                            } else {
                                palette.result
                            };
                            let color = match stale {
                                true => color.lerp_to_gamma(palette.background, STALE_DIM),
                                false => color,
                            };
                            let row_rect = row.rect().translate(output.galley_pos.to_vec2());
                            let budget = if is_error {
                                error_budget(gutter_right, row_rect.right(), gutter_budget)
                            } else {
                                gutter_budget
                            };
                            let font = if cell.total && !is_error {
                                mono_bold()
                            } else {
                                mono()
                            };
                            let mut job = LayoutJob::simple(
                                cell.shown.text.clone(),
                                font,
                                color,
                                f32::INFINITY,
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(budget);
                            let galley = ui.fonts_mut(|f| f.layout_job(job));
                            let width = galley.rect.width();
                            let pos = egui::pos2(gutter_right - width, row_rect.top());
                            let elided = galley.elided;
                            ui.painter().galley(pos, galley, color);
                            let click_rect = egui::Rect::from_min_size(
                                egui::pos2(gutter_right - gutter_width, row_rect.top()),
                                egui::vec2(gutter_width, row_rect.height()),
                            );
                            // An error only has a tooltip: its label isn't worth copying.
                            let shown = &cell.shown;
                            let response = match &shown.error {
                                Some(full) => ui
                                    .allocate_rect(click_rect, egui::Sense::hover())
                                    .on_hover_text(full),
                                None => ui
                                    .allocate_rect(click_rect, egui::Sense::click())
                                    .on_hover_cursor(egui::CursorIcon::PointingHand),
                            };
                            let response = match (&shown.full, elided && shown.error.is_none()) {
                                (Some(full), _) => response.on_hover_text(full),
                                (None, true) => response.on_hover_text(&shown.text),
                                (None, false) => response,
                            };
                            if response.clicked() {
                                ui.ctx().copy_text(shown.copy.clone());
                                self.copied = Some((shown.copy.clone(), Instant::now()));
                            }
                        }
                    });
                });
        });
    }

    /// The editor keeps the keyboard, as in any notepad, except while an
    /// overlay is open or the hotkey is being set. A new or switched-to tab
    /// opens with the caret at its end.
    fn keep_focus(&mut self, ctx: &egui::Context, tab_id: egui::Id) {
        if self.show_converters || self.show_example || self.capturing_hotkey {
            ctx.memory_mut(|m| m.surrender_focus(tab_id));
            self.focused_tab = None;
            return;
        }
        let tab = self.tabs[self.active].id;
        if self.focused_tab != Some(tab) {
            let mut state = egui::TextEdit::load_state(ctx, tab_id).unwrap_or_default();
            if state.cursor.char_range().is_none() {
                let end = CCursor::new(self.active_text().chars().count());
                state.cursor.set_char_range(Some(CCursorRange::one(end)));
                state.store(ctx, tab_id);
            }
            self.focused_tab = Some(tab);
        }
        if ctx.memory(|m| m.focused()) != Some(tab_id) {
            ctx.memory_mut(|m| m.request_focus(tab_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(s: &str) -> LineResult {
        LineResult::Value(s.to_owned())
    }

    fn texts(text: &str, results: &[LineResult]) -> Vec<Option<String>> {
        result_cells(text, results, false)
            .into_iter()
            .map(|cell| cell.map(|cell| cell.shown.text))
            .collect()
    }

    /// Every result is painted as it's formatted, with no padding to line
    /// up decimal points, so `\u{a5}21,000`, `15` and `500` all end on the
    /// column's right edge.
    #[test]
    fn results_are_not_padded_so_they_end_on_the_right_edge() {
        let text = "a\nb\nc\nd\ne\nf";
        let results = [
            value("1680 USD"),
            value("21000 JPY"),
            value("\u{2248} 11.0231131092 lbs"),
            value("15"),
            value("500"),
            value("180.34 cm"),
        ];
        assert_eq!(
            texts(text, &results),
            [
                Some("$1,680.00".to_owned()),
                Some("\u{a5}21,000".to_owned()),
                Some("\u{2248} 11.0231 lbs".to_owned()),
                Some("15".to_owned()),
                Some("500".to_owned()),
                Some("180.34 cm".to_owned()),
            ]
        );
    }

    #[test]
    fn a_line_without_a_result_has_no_cell() {
        let text = "a\nb\n\nc\nd";
        let results = [
            value("1.5"),
            LineResult::Error("unknown identifier 'x'".to_owned()),
            LineResult::Blank,
            value("100"),
            LineResult::Date("2026-09-30".to_owned()),
        ];
        assert_eq!(
            texts(text, &results),
            [
                Some("1.5".to_owned()),
                Some("unknown x".to_owned()),
                None,
                Some("100".to_owned()),
                Some("2026-09-30".to_owned()),
            ]
        );
    }

    #[test]
    fn totals_are_sum_and_avg_lines() {
        assert!(is_total("sum"));
        assert!(is_total("Total: avg in EUR"));
        assert!(!is_total("prev + 1"));
        assert!(!is_total("// sum"));
    }

    #[test]
    fn caret_line_counts_newlines_before_the_caret() {
        let at = |i| Some(CCursorRange::one(CCursor::new(i)));
        assert_eq!(caret_line("1\n22\n3", at(0)), Some(0));
        assert_eq!(caret_line("1\n22\n3", at(4)), Some(1));
        assert_eq!(caret_line("1\n22\n3", at(5)), Some(2));
        assert_eq!(caret_line("1", None), None);
    }
}
