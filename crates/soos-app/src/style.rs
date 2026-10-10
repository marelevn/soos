//! Fonts, colours, and the small drawing helpers every surface uses.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, TextFormat, Theme};

pub(crate) const FONT_SIZE: f32 = 16.0;

/// JetBrains Mono is monospaced at 600/1000 em, so widths meant to fit N
/// characters are computed from this.
pub(crate) const CHAR_WIDTH: f32 = FONT_SIZE * 0.6;

/// The one font every surface uses.
pub(crate) fn mono() -> FontId {
    FontId::monospace(FONT_SIZE)
}

/// Its bold weight, for headings and totals. Same advance width, so bold
/// text stays on the character grid.
pub(crate) fn mono_bold() -> FontId {
    FontId::new(FONT_SIZE, egui::FontFamily::Name(BOLD.into()))
}

const BOLD: &str = "bold";

pub(crate) const LINE_HEIGHT: f32 = FONT_SIZE * 1.75;
/// With a tall `LINE_HEIGHT`, epaint puts the extra leading below the
/// glyphs, so text sits high in its row. This moves the glyphs (not the row
/// or the caret) down. Set by eye for JetBrains Mono at `FONT_SIZE`.
pub(crate) const GLYPH_Y_OFFSET: f32 = 4.0;

pub(crate) const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
pub(crate) const JETBRAINS_MONO_BOLD: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf");
/// The currency signs JetBrains Mono lacks (`₹`, `₩`, `₺`, `฿` and nine
/// more), cut from DejaVu Sans Mono so the fallback is a few KB rather
/// than a whole font; see CONTRIBUTING.md, "Fonts". DejaVu has no `₼`,
/// `₾` or `﷼`, so those show as boxes.
pub(crate) const DEJAVU_CURRENCY: &[u8] =
    include_bytes!("../../../assets/fonts/DejaVuSansMono-Currency.ttf");

/// JetBrains Mono first, then [`DEJAVU_CURRENCY`]; the bold family falls
/// back the same way. Proportional text (tooltips) uses the same fonts, so
/// the whole app is in JetBrains Mono.
pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::empty();
    let tweak = egui::FontTweak {
        y_offset: GLYPH_Y_OFFSET,
        ..Default::default()
    };
    for (name, bytes) in [
        ("JetBrainsMono", JETBRAINS_MONO_REGULAR),
        ("JetBrainsMonoBold", JETBRAINS_MONO_BOLD),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes).tweak(tweak.clone())),
        );
    }
    fonts.font_data.insert(
        "DejaVuCurrency".to_owned(),
        Arc::new(egui::FontData::from_static(DEJAVU_CURRENCY)),
    );
    let family = |first: &str| vec![first.to_owned(), "DejaVuCurrency".to_owned()];
    fonts.families = [
        (egui::FontFamily::Monospace, family("JetBrainsMono")),
        (egui::FontFamily::Proportional, family("JetBrainsMono")),
        (
            egui::FontFamily::Name(BOLD.into()),
            family("JetBrainsMonoBold"),
        ),
    ]
    .into();
    ctx.set_fonts(fonts);
}

/// Every text colour has at least 4.5:1 contrast with `background` (WCAG
/// AA), and `comment` and `secondary` also with the tab strip's tint.
/// Colour is for the answers: what's typed is neutral in three steps --
/// `plain`, then `secondary` for labels and joining words, then `comment`
/// -- with one accent for the words that stand for a value (`sum`, `prev`,
/// `today`).
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) background: Color32,
    pub(crate) plain: Color32,
    pub(crate) result: Color32,
    pub(crate) error: Color32,
    pub(crate) keyword: Color32,
    /// Labels (`Rent:`) and joining words (`in`, `of`).
    pub(crate) secondary: Color32,
    pub(crate) comment: Color32,
    /// Behind an overlay. The same alpha barely shows on dark and smears
    /// on light, so each theme has its own.
    pub(crate) backdrop: Color32,
}

impl Palette {
    pub(crate) const DARK: Palette = Palette {
        background: Color32::from_rgb(0x22, 0x24, 0x28),
        plain: Color32::from_rgb(0xf0, 0xf0, 0xf0),
        result: Color32::from_rgb(0x96, 0xc8, 0x5a),
        error: Color32::from_rgb(0xd8, 0x6c, 0x6c),
        keyword: Color32::from_rgb(0x5b, 0xc8, 0xe8),
        secondary: Color32::from_rgb(0xb8, 0xbc, 0xc3),
        comment: Color32::from_rgb(0x95, 0x9a, 0xa2),
        backdrop: Color32::from_black_alpha(180),
    };

    pub(crate) const LIGHT: Palette = Palette {
        background: Color32::from_rgb(0xf7, 0xf8, 0xfa),
        plain: Color32::from_rgb(0x14, 0x16, 0x1a),
        result: Color32::from_rgb(0x38, 0x7f, 0x2a),
        error: Color32::from_rgb(0xc0, 0x20, 0x20),
        keyword: Color32::from_rgb(0x0b, 0x7a, 0x9e),
        secondary: Color32::from_rgb(0x4a, 0x4f, 0x57),
        comment: Color32::from_rgb(0x62, 0x69, 0x73),
        backdrop: Color32::from_black_alpha(90),
    };

    pub(crate) fn of(theme: Theme) -> Palette {
        match theme {
            Theme::Dark => Palette::DARK,
            Theme::Light => Palette::LIGHT,
        }
    }
}

pub(crate) fn app_visuals(theme: Theme) -> egui::Visuals {
    let palette = Palette::of(theme);
    let mut visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    visuals.panel_fill = palette.background;
    visuals.extreme_bg_color = palette.background;
    visuals.override_text_color = Some(palette.plain);
    // Blended toward the background, so it's a tint in both themes rather
    // than a dark smear in the light one.
    visuals.selection.bg_fill = palette.background.lerp_to_gamma(palette.keyword, 0.35);
    visuals.text_cursor.stroke.color = palette.plain;
    visuals
}

/// How far hovering blends a clickable symbol toward `palette.plain`.
pub(crate) const HOVER_BRIGHTEN: f32 = 0.5;

pub(crate) fn text_format(color: Color32) -> TextFormat {
    text_format_in(mono(), color)
}

pub(crate) fn text_format_in(font: FontId, color: Color32) -> TextFormat {
    TextFormat {
        line_height: Some(LINE_HEIGHT),
        ..TextFormat::simple(font, color)
    }
}

/// A clickable status-bar symbol or label that brightens on hover.
pub(crate) fn status_symbol(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    palette: Palette,
) -> egui::Response {
    let (rect, response, galley) = allocate_text(ui, text, egui::Sense::click());
    let hover_t = ui.ctx().animate_bool(response.id, response.hovered());
    let color = color.lerp_to_gamma(palette.plain, hover_t * HOVER_BRIGHTEN);
    paint_centred(ui, text, rect, galley, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Status-bar text that only shows a tooltip; it doesn't look clickable.
pub(crate) fn status_text(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let (rect, response, galley) = allocate_text(ui, text, egui::Sense::hover());
    paint_centred(ui, text, rect, galley, color);
    response
}

fn allocate_text(
    ui: &mut egui::Ui,
    text: &str,
    sense: egui::Sense,
) -> (egui::Rect, egui::Response, Arc<egui::Galley>) {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), mono(), Color32::PLACEHOLDER));
    let (rect, response) = ui.allocate_exact_size(galley.size(), sense);
    (rect, response, galley)
}

/// Paints `galley` with its ink centred in the row rather than on the
/// baseline: JetBrains Mono's symbols differ a lot in height (`±` is much
/// shorter than `?`), and baseline alignment makes the short ones sit low.
fn paint_centred(
    ui: &egui::Ui,
    text: &str,
    rect: egui::Rect,
    galley: Arc<egui::Galley>,
    color: Color32,
) {
    let nudge = if text.chars().count() <= 1 {
        ink_center_offset(&galley)
    } else {
        cap_center_offset(ui)
    };
    ui.painter()
        .galley_with_override_text_color(rect.min + egui::vec2(0.0, nudge), galley, color);
}

/// How far to move a one-glyph galley so its ink is centred in the galley
/// box; 0.0 for a glyph with no ink.
pub(crate) fn ink_center_offset(galley: &egui::Galley) -> f32 {
    let Some(glyph) = galley.rows.first().and_then(|row| row.row.glyphs.first()) else {
        return 0.0;
    };
    if glyph.uv_rect.is_nothing() {
        return 0.0;
    }
    let ink_top = glyph.pos.y + glyph.uv_rect.offset.y;
    let ink_center = ink_top + glyph.uv_rect.size.y / 2.0;
    galley.size().y / 2.0 - ink_center
}

/// [`ink_center_offset`] of a capital `H`, so every multi-character string
/// shares one baseline whatever its first letter.
pub(crate) fn cap_center_offset(ui: &egui::Ui) -> f32 {
    ink_center_offset(
        &ui.fonts_mut(|f| f.layout_no_wrap("H".to_owned(), mono(), Color32::PLACEHOLDER)),
    )
}

/// Like `ui.button`, with the glyph's ink centred: JetBrains Mono's `+` and
/// `-` sit visibly high in egui's own button.
pub(crate) fn centered_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), mono(), Color32::PLACEHOLDER));
    let padding = ui.spacing().button_padding;
    let size = galley.size() + 2.0 * padding;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect.expand(visuals.expansion),
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let text_pos = rect.left_top() + padding + egui::vec2(0.0, ink_center_offset(&galley));
        ui.painter()
            .galley_with_override_text_color(text_pos, galley, visuals.text_color());
    }
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Color32, FontId};

    use crate::tabs::tab_bar_fill;

    /// WCAG 2 contrast ratio between two sRGB colours.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let linear = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_text_colour_meets_wcag_aa() {
        for palette in [Palette::DARK, Palette::LIGHT] {
            let text = [
                palette.plain,
                palette.result,
                palette.error,
                palette.keyword,
                palette.secondary,
                palette.comment,
            ];
            for colour in text {
                let ratio = contrast(colour, palette.background);
                assert!(ratio >= 4.5, "{colour:?} is {ratio:.2}:1");
            }
            for colour in [palette.comment, palette.secondary] {
                let ratio = contrast(colour, tab_bar_fill(palette));
                assert!(ratio >= 4.5, "{colour:?} on the tab strip is {ratio:.2}:1");
            }
        }
    }

    /// Bold keeps JetBrains Mono's grid, so bold results stay aligned.
    #[test]
    fn bold_has_the_regular_advance_width() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        let width = |font: FontId| {
            ctx.fonts_mut(|f| f.layout_no_wrap("$1,234.56".to_owned(), font, Color32::WHITE))
                .size()
                .x
        };
        assert_eq!(width(mono()), width(mono_bold()));
    }

    /// JetBrains Mono has none of these; the DejaVu subset has them all.
    #[test]
    fn dejavu_covers_signs_jetbrains_mono_lacks() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&mono(), "₹₩₺₱₪₦₸₡₲₵₭฿₨")));
    }

    /// `±` and `?` have very different ink heights, so a working
    /// `ink_center_offset` nudges them differently.
    #[test]
    fn ink_center_offset_corrects_a_known_misaligned_glyph_and_ignores_space() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        ctx.begin_pass(egui::RawInput::default());

        let offset_of = |glyph: &str| {
            let galley =
                ctx.fonts_mut(|f| f.layout_no_wrap(glyph.to_owned(), mono(), Color32::WHITE));
            ink_center_offset(&galley)
        };

        let plus_minus = offset_of("\u{b1}");
        let question = offset_of("?");
        assert!(
            (plus_minus - question).abs() > 0.5,
            "expected a real correction: ±={plus_minus}, ?={question}"
        );
        assert_eq!(offset_of(" "), 0.0);
    }
}
