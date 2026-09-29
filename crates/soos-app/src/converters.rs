//! The converters table: the user's own units, kept apart from the document.

use super::*;

/// One row of the table, as typed: `unit` = `factor` × `base`, `aliases`
/// comma-separated. soos-core validates it on every recalculation.
#[derive(Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ConverterRow {
    pub(crate) unit: String,
    pub(crate) aliases: String,
    pub(crate) base: String,
    pub(crate) factor: String,
}

impl ConverterRow {
    pub(crate) fn to_raw(&self) -> soos_core::RawConverter {
        soos_core::RawConverter {
            unit: self.unit.trim().to_string(),
            aliases: self
                .aliases
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            base: self.base.trim().to_string(),
            factor: self.factor.trim().to_string(),
        }
    }
}

/// A text field's left plus right margin, added to each column width so the
/// widths below count visible characters.
pub(crate) const TEXT_EDIT_HPADDING: f32 = 8.0;
/// Larger than egui's default, so the row buttons are easy to hit.
pub(crate) const CONVERTER_BUTTON_PADDING: egui::Vec2 = egui::vec2(10.0, 6.0);
pub(crate) const CONVERTER_ITEM_GAP: f32 = 8.0;
pub(crate) const CONVERTER_GRID_SPACING: egui::Vec2 = egui::vec2(14.0, 10.0);

/// Column widths, each fitting a long but realistic entry.
pub(crate) const UNIT_COL_WIDTH: f32 = CHAR_WIDTH * 10.0 + TEXT_EDIT_HPADDING; // e.g. "gallon_us"
pub(crate) const ALIASES_COL_WIDTH: f32 = CHAR_WIDTH * 16.0 + TEXT_EDIT_HPADDING; // e.g. "ton, tonne, MT"
pub(crate) const BASE_COL_WIDTH: f32 = CHAR_WIDTH * 13.0 + TEXT_EDIT_HPADDING; // e.g. "nautical_mile"
pub(crate) const FACTOR_COL_WIDTH: f32 = CHAR_WIDTH * 12.0 + TEXT_EDIT_HPADDING; // e.g. "0.3937007874"
pub(crate) const BUTTON_WIDTH: f32 = CHAR_WIDTH + 2.0 * CONVERTER_BUTTON_PADDING.x;
/// The ↑ ↓ − buttons.
pub(crate) const CONTROLS_COL_WIDTH: f32 = 3.0 * BUTTON_WIDTH + 2.0 * CONVERTER_ITEM_GAP;
/// Fits every converter error (20 characters at most); a longer result is
/// cut, with the full text on hover.
pub(crate) const STATUS_COL_WIDTH: f32 = CHAR_WIDTH * 24.0;

pub(crate) const CONVERTERS_GRID_WIDTH: f32 = UNIT_COL_WIDTH
    + ALIASES_COL_WIDTH
    + BASE_COL_WIDTH
    + FACTOR_COL_WIDTH
    + CONTROLS_COL_WIDTH
    + STATUS_COL_WIDTH
    + 5.0 * CONVERTER_GRID_SPACING.x;

/// The rows scroll past this height, so `+ Add converter` stays in view.
pub(crate) const CONVERTER_ROWS_MAX_HEIGHT: f32 = 320.0;
/// The overlay's height outside the rows: header, add button, margins.
const CONVERTER_CHROME_HEIGHT: f32 = 110.0;

/// One grid cell at exactly `size`. Without `set_min_size` a cell reports
/// its content's size to the `Grid`, and rows and stripes come out uneven.
pub(crate) fn fixed_cell<R>(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(size);
            add_contents(ui)
        },
    )
    .inner
}

/// Truncated, with the full text on hover.
pub(crate) fn converter_status_cell(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    row_height: f32,
) {
    fixed_cell(ui, egui::vec2(STATUS_COL_WIDTH, row_height), |ui| {
        let resp =
            ui.add(egui::Label::new(RichText::new(text).color(color).font(mono())).truncate());
        if !text.is_empty() {
            resp.on_hover_text(text);
        }
    });
}

/// The converters table: one editable row per converter with its result
/// or error, then `+ Add converter`. `focus_row` puts the cursor in that
/// row's unit field (the first on opening, a new one when added).
pub(crate) fn converters_window(
    ui: &mut egui::Ui,
    palette: Palette,
    converters: &mut Vec<ConverterRow>,
    results: &[Result<String, String>],
    focus_row: &mut Option<usize>,
) {
    ui.style_mut().spacing.button_padding = CONVERTER_BUTTON_PADDING;
    ui.style_mut().spacing.item_spacing.x = CONVERTER_ITEM_GAP;
    let want_focus = focus_row.take();

    let mut remove = None;
    let mut swap = None;
    let len = converters.len();

    // Every cell is a button's height, and text fields get the same
    // vertical margin, so they fill the row.
    let row_height = ui
        .fonts_mut(|f| f.layout_no_wrap("\u{2191}".to_owned(), mono(), Color32::PLACEHOLDER))
        .size()
        .y
        + 2.0 * ui.spacing().button_padding.y;
    let text_edit_margin = egui::Margin::symmetric(
        (TEXT_EDIT_HPADDING / 2.0) as i8,
        ui.spacing().button_padding.y as i8,
    );

    let header = |ui: &mut egui::Ui, width: f32, label: &str| {
        fixed_cell(ui, egui::vec2(width, row_height), |ui| {
            ui.label(RichText::new(label).color(palette.comment).font(mono()));
        });
    };

    let rows_height = CONVERTER_ROWS_MAX_HEIGHT
        .min(modal_room(ui.ctx()).y - CONVERTER_CHROME_HEIGHT)
        .max(row_height * 2.0);
    // Both ways: in a narrow window the table scrolls sideways.
    egui::ScrollArea::both()
        .max_height(rows_height)
        .show(ui, |ui| {
            egui::Grid::new("converters-grid")
                .num_columns(6)
                .spacing(CONVERTER_GRID_SPACING)
                .striped(true)
                .show(ui, |ui| {
                    header(ui, UNIT_COL_WIDTH, "unit");
                    header(ui, ALIASES_COL_WIDTH, "aliases");
                    header(ui, BASE_COL_WIDTH, "base");
                    header(ui, FACTOR_COL_WIDTH, "factor");
                    header(ui, CONTROLS_COL_WIDTH, "");
                    header(ui, STATUS_COL_WIDTH, "1 unit =");
                    ui.end_row();

                    for (i, row) in converters.iter_mut().enumerate() {
                        fixed_cell(ui, egui::vec2(UNIT_COL_WIDTH, row_height), |ui| {
                            let unit_resp = ui.add(
                                egui::TextEdit::singleline(&mut row.unit)
                                    .hint_text(
                                        RichText::new("teu").color(palette.comment).font(mono()),
                                    )
                                    .font(mono())
                                    .margin(text_edit_margin)
                                    .desired_width(UNIT_COL_WIDTH),
                            );
                            if want_focus == Some(i) {
                                unit_resp.request_focus();
                            }
                        });
                        fixed_cell(ui, egui::vec2(ALIASES_COL_WIDTH, row_height), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut row.aliases)
                                    .hint_text(
                                        RichText::new("TEU, teus")
                                            .color(palette.comment)
                                            .font(mono()),
                                    )
                                    .font(mono())
                                    .margin(text_edit_margin)
                                    .desired_width(ALIASES_COL_WIDTH),
                            );
                        });
                        fixed_cell(ui, egui::vec2(BASE_COL_WIDTH, row_height), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut row.base)
                                    .hint_text(
                                        RichText::new("cbm").color(palette.comment).font(mono()),
                                    )
                                    .font(mono())
                                    .margin(text_edit_margin)
                                    .desired_width(BASE_COL_WIDTH),
                            );
                        });
                        fixed_cell(ui, egui::vec2(FACTOR_COL_WIDTH, row_height), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut row.factor)
                                    .hint_text(
                                        RichText::new("33.2").color(palette.comment).font(mono()),
                                    )
                                    .font(mono())
                                    .margin(text_edit_margin)
                                    .desired_width(FACTOR_COL_WIDTH),
                            );
                        });
                        fixed_cell(ui, egui::vec2(CONTROLS_COL_WIDTH, row_height), |ui| {
                            if ui
                                .add_enabled_ui(i > 0, |ui| centered_button(ui, "\u{2191}"))
                                .inner
                                .on_hover_text("Move up")
                                .clicked()
                            {
                                swap = Some((i, i - 1));
                            }
                            if ui
                                .add_enabled_ui(i + 1 < len, |ui| centered_button(ui, "\u{2193}"))
                                .inner
                                .on_hover_text("Move down")
                                .clicked()
                            {
                                swap = Some((i, i + 1));
                            }
                            if centered_button(ui, "\u{2212}")
                                .on_hover_text("Remove")
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });

                        // Grey, not red, while the row is still being filled in.
                        let missing: Vec<&str> = [
                            ("unit", &row.unit),
                            ("base", &row.base),
                            ("factor", &row.factor),
                        ]
                        .into_iter()
                        .filter(|(_, value)| value.trim().is_empty())
                        .map(|(name, _)| name)
                        .collect();
                        if !missing.is_empty() {
                            let text = format!("needs {}", missing.join(", "));
                            converter_status_cell(ui, &text, palette.comment, row_height);
                        } else {
                            match results.get(i) {
                                Some(Ok(d)) => {
                                    converter_status_cell(ui, d, palette.result, row_height)
                                }
                                Some(Err(e)) => {
                                    converter_status_cell(ui, e, palette.error, row_height)
                                }
                                None => converter_status_cell(ui, "", palette.comment, row_height),
                            }
                        }
                        ui.end_row();
                    }
                });
        });

    ui.add_space(MODAL_GAP);
    if len >= soos_core::MAX_CONVERTERS {
        ui.label(
            RichText::new(format!("{} converters max", soos_core::MAX_CONVERTERS))
                .color(palette.comment)
                .font(mono()),
        );
    } else if centered_button(ui, "+ Add converter").clicked() {
        converters.push(ConverterRow::default());
        *focus_row = Some(converters.len() - 1);
    }

    // Applied after the loop, which borrows `converters`.
    if let Some(i) = remove {
        converters.remove(i);
    }
    if let Some((a, b)) = swap {
        converters.swap(a, b);
    }
}
