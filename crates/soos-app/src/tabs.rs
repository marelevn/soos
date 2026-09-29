//! Tabs: one document each, the tab strip, and its shortcuts.

use super::*;

/// One tab's document. `id` is never reused, so the editor keeps each tab's
/// cursor and undo history apart however tabs are closed or reopened.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Tab {
    pub(crate) id: u64,
    pub(crate) text: String,
}

/// Cmd/Ctrl+1 to 9 reach every tab.
pub(crate) const MAX_TABS: usize = 9;
/// How many closed tabs Cmd/Ctrl+Shift+T can bring back.
pub(crate) const MAX_CLOSED_TABS: usize = 10;
pub(crate) const TAB_BAR_H_PAD: i8 = 10;
/// Between the window's top edge and the tabs; the tabs reach the strip's
/// bottom edge, where the active one meets its page.
const STRIP_TOP_PAD: i8 = 8;
const TAB_HEIGHT: f32 = 30.0;
/// The top corners of the active and hovered tab.
const TAB_RADIUS: u8 = 8;
/// Inside a tab, beside its title.
const TAB_H_PAD: f32 = 12.0;
/// Between a tab's edge and its `×`, which needs less room than text.
const CLOSE_EDGE_PAD: f32 = 6.0;
/// The `×` button's square.
const CLOSE_SIZE: f32 = 18.0;
/// Between the `×` and the title.
const CLOSE_GAP: f32 = 4.0;
/// Between the last tab and `+`.
const PLUS_GAP: f32 = 4.0;
/// The strip is the background tinted this far toward the text colour.
pub(crate) const TAB_BAR_CONTRAST: f32 = 0.06;
/// A hovered tab's fill, from the strip toward the page.
const HOVER_FILL: f32 = 0.5;

pub(crate) fn tab_bar_fill(palette: Palette) -> Color32 {
    palette
        .background
        .lerp_to_gamma(palette.plain, TAB_BAR_CONTRAST)
}

/// The saved tabs (one empty tab if there are none) and the next free id.
/// Every saved tab is kept, even past [`MAX_TABS`].
pub(crate) fn load_tabs(mut tabs: Vec<Tab>) -> (Vec<Tab>, u64) {
    if tabs.is_empty() {
        tabs.push(Tab {
            id: 0,
            text: String::new(),
        });
    }
    let next_tab_id = tabs.iter().map(|t| t.id).max().map_or(0, |id| id + 1);
    (tabs, next_tab_id)
}

/// A tab's title: its first non-empty line without a leading `#` or `//`,
/// or "Tab N" while it has none.
pub(crate) fn tab_title(text: &str, position: usize) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| {
            line.trim_start_matches(['#'])
                .trim_start_matches("//")
                .trim()
        })
        .filter(|line| !line.is_empty())
        .map_or_else(|| format!("Tab {}", position + 1), str::to_string)
}

/// Every tab's width: the strip shared equally, as in Safari, so a tab
/// never changes width while its title does. `plus` is whether `+` takes
/// its square at the end.
pub(crate) fn tab_width(strip_width: f32, tabs: usize, plus: bool) -> f32 {
    let plus_width = if plus { TAB_HEIGHT + PLUS_GAP } else { 0.0 };
    ((strip_width - plus_width) / tabs.max(1) as f32).max(0.0)
}

/// Where a tab's title goes: all of the tab but its padding and the
/// `×`'s place, kept free whether the `×` shows or not, so the title never
/// moves when it appears.
pub(crate) fn title_room(tab: egui::Rect) -> egui::Rect {
    let close_side = CLOSE_EDGE_PAD + CLOSE_SIZE + CLOSE_GAP;
    let (left, right) = match CLOSE_ON_LEADING_EDGE {
        true => (close_side, TAB_H_PAD),
        false => (TAB_H_PAD, close_side),
    };
    let min_x = tab.left() + left;
    let max_x = (tab.right() - right).max(min_x);
    egui::Rect::from_x_y_ranges(min_x..=max_x, tab.y_range())
}

/// `title` cut to fit `width`, ending in `…` if it was cut; true if cut.
/// Every glyph is [`CHAR_WIDTH`] wide, so this counts characters.
pub(crate) fn fit_title(title: &str, width: f32) -> (String, bool) {
    let room = (width / CHAR_WIDTH).floor() as usize;
    if title.chars().count() <= room {
        return (title.to_owned(), false);
    }
    let kept: String = title.chars().take(room.saturating_sub(1)).collect();
    (format!("{}\u{2026}", kept.trim_end()), true)
}

/// The `×` sits where each platform puts it: on a tab's leading edge on
/// macOS, as in Safari and Finder, and its trailing edge elsewhere, as in
/// Chrome and Firefox.
const CLOSE_ON_LEADING_EDGE: bool = cfg!(target_os = "macos");

/// Whether a divider goes between tab `i` and the next: only between two
/// tabs at rest, as a highlighted tab's own shape already separates it.
pub(crate) fn divider_after(i: usize, tabs: usize, highlighted: &[usize]) -> bool {
    i + 1 < tabs && !highlighted.contains(&i) && !highlighted.contains(&(i + 1))
}

/// What the user did to one tab this frame.
pub(crate) struct TabClicks {
    pub(crate) select: bool,
    pub(crate) close: bool,
    pub(crate) hovered: bool,
}

/// A tab's top corners rounded, its bottom square where it meets the page.
fn tab_shape(ui: &egui::Ui, rect: egui::Rect, fill: Color32) {
    let radius = egui::CornerRadius {
        nw: TAB_RADIUS,
        ne: TAB_RADIUS,
        sw: 0,
        se: 0,
    };
    ui.painter().rect_filled(rect, radius, fill);
}

/// One tab in `rect`: its title centred beside the `×`'s place, and the
/// `×` while the tab is active or hovered.
pub(crate) fn tab_label(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    palette: Palette,
    active: bool,
    title: &str,
    close_shortcut: &str,
) -> TabClicks {
    let response = ui.interact(rect, id, egui::Sense::click());
    let close_centre = match CLOSE_ON_LEADING_EDGE {
        true => rect.left() + CLOSE_EDGE_PAD + CLOSE_SIZE / 2.0,
        false => rect.right() - CLOSE_EDGE_PAD - CLOSE_SIZE / 2.0,
    };
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(close_centre, rect.center().y),
        egui::vec2(CLOSE_SIZE, CLOSE_SIZE),
    );
    let close = ui
        .interact(close_rect, response.id.with("close"), egui::Sense::click())
        .on_hover_text(format!("Close tab ({close_shortcut})"))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close tab"));
    let hovered = response.contains_pointer();
    let hover_t = ui.ctx().animate_bool(response.id, hovered && !active);

    let strip = tab_bar_fill(palette);
    if active {
        tab_shape(ui, rect, palette.background);
    } else if hover_t > 0.0 {
        let fill = strip.lerp_to_gamma(palette.background, HOVER_FILL * hover_t);
        tab_shape(ui, rect, fill);
    }

    let room = title_room(rect);
    let (shown, elided) = fit_title(title, room.width());
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(shown, mono(), Color32::PLACEHOLDER));
    let color = match active {
        true => palette.plain,
        false => palette
            .secondary
            .lerp_to_gamma(palette.plain, hover_t * HOVER_BRIGHTEN),
    };
    let text_pos = egui::pos2(
        room.center().x - galley.size().x / 2.0,
        rect.center().y - galley.size().y / 2.0 + cap_center_offset(ui),
    );
    ui.painter()
        .with_clip_rect(rect)
        .galley_with_override_text_color(text_pos, galley, color);

    if active || hovered {
        if close.hovered() {
            ui.painter().rect_filled(
                close_rect,
                egui::CornerRadius::same(4),
                strip.lerp_to_gamma(palette.plain, 0.12),
            );
        }
        let close_galley =
            ui.fonts_mut(|f| f.layout_no_wrap("\u{d7}".to_owned(), mono(), Color32::PLACEHOLDER));
        let close_color = match close.hovered() {
            true => palette.plain,
            false => palette.comment,
        };
        let close_pos = close_rect.center() - close_galley.size() / 2.0
            + egui::vec2(0.0, ink_center_offset(&close_galley));
        ui.painter()
            .galley_with_override_text_color(close_pos, close_galley, close_color);
    }

    let response = match elided {
        true => response.on_hover_text(title),
        false => response,
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, title)
    });
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    TabClicks {
        select: response.clicked() && !close.clicked(),
        close: close.clicked() || response.middle_clicked(),
        hovered,
    }
}

/// `+` as a bare glyph in a tab-high square, with a hover fill like a tab's.
fn plus_button(ui: &mut egui::Ui, rect: egui::Rect, palette: Palette) -> egui::Response {
    let response = ui.interact(rect, ui.id().with("new-tab"), egui::Sense::click());
    let hover_t = ui.ctx().animate_bool(response.id, response.hovered());
    if hover_t > 0.0 {
        let fill = tab_bar_fill(palette).lerp_to_gamma(palette.background, HOVER_FILL * hover_t);
        ui.painter()
            .rect_filled(rect.shrink(3.0), egui::CornerRadius::same(6), fill);
    }
    let galley = ui.fonts_mut(|f| f.layout_no_wrap("+".to_owned(), mono(), Color32::PLACEHOLDER));
    let pos = rect.center() - galley.size() / 2.0 + egui::vec2(0.0, ink_center_offset(&galley));
    let color = palette
        .comment
        .lerp_to_gamma(palette.plain, hover_t * HOVER_BRIGHTEN);
    ui.painter()
        .galley_with_override_text_color(pos, galley, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New tab"));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl SoosApp {
    pub(crate) fn next_id(&mut self) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        id
    }

    /// Add an empty tab and switch to it, unless there are [`MAX_TABS`].
    pub(crate) fn new_tab(&mut self) {
        if self.tabs.len() >= MAX_TABS {
            return;
        }
        let id = self.next_id();
        self.tabs.push(Tab {
            id,
            text: String::new(),
        });
        self.active = self.tabs.len() - 1;
    }

    /// Close tab `idx` without asking: Cmd/Ctrl+Shift+T brings it back.
    /// Closing the last tab leaves one empty tab.
    pub(crate) fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let closed = self.tabs.remove(idx);
        self.closed.push(closed);
        if self.closed.len() > MAX_CLOSED_TABS {
            self.closed.remove(0);
        }
        if self.tabs.is_empty() {
            let id = self.next_id();
            self.tabs.push(Tab {
                id,
                text: String::new(),
            });
        }
        // Keep the same tab selected; if it was the one closed, select the
        // one that took its place (or the new last tab).
        if idx < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len() - 1);
    }

    /// Bring back the most recently closed tab, if there's room.
    pub(crate) fn undo_close_tab(&mut self) {
        if self.tabs.len() >= MAX_TABS {
            return;
        }
        let Some(tab) = self.closed.pop() else {
            return;
        };
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    /// Consumed before the editor runs, so a shortcut never also types into
    /// the document. Cmd/Ctrl+W takes over the editor's delete-previous-word
    /// where a platform binds it.
    pub(crate) fn handle_tab_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let consume = |ctx: &egui::Context, mods: Modifiers, key: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(mods, key)))
        };
        // First, because `consume_shortcut` would also match Cmd/Ctrl+T.
        if consume(ctx, Modifiers::COMMAND | Modifiers::SHIFT, Key::T) {
            self.undo_close_tab();
        } else if consume(ctx, Modifiers::COMMAND, Key::T) {
            self.new_tab();
        }
        if consume(ctx, Modifiers::COMMAND, Key::W) {
            self.close_tab(self.active);
        }
        const DIGIT_KEYS: [Key; MAX_TABS] = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, key) in DIGIT_KEYS.into_iter().enumerate() {
            if consume(ctx, Modifiers::COMMAND, key) && i < self.tabs.len() {
                self.active = i;
            }
        }
        if consume(ctx, Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        } else if consume(ctx, Modifiers::CTRL, Key::Tab) {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    /// The tab strip across the top of the window: the tabs sharing its
    /// width, then `+` at the end.
    pub(crate) fn tab_bar(&mut self, ui: &mut egui::Ui, palette: Palette) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let shortcut = |key| {
            ui.ctx()
                .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key))
        };
        let (new_shortcut, close_shortcut) = (shortcut(Key::T), shortcut(Key::W));
        egui::Panel::top("tabs")
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: TAB_BAR_H_PAD,
                        right: TAB_BAR_H_PAD,
                        top: STRIP_TOP_PAD,
                        bottom: 0,
                    })
                    .fill(tab_bar_fill(palette)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                let plus = self.tabs.len() < MAX_TABS;
                let (strip, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), TAB_HEIGHT),
                    egui::Sense::hover(),
                );
                let width = tab_width(strip.width(), self.tabs.len(), plus);
                let tab_rect = |i: usize| {
                    egui::Rect::from_min_size(
                        strip.min + egui::vec2(width * i as f32, 0.0),
                        egui::vec2(width, TAB_HEIGHT),
                    )
                };

                let mut switch_to = None;
                let mut close_idx = None;
                let mut highlighted = vec![self.active];
                for (i, tab) in self.tabs.iter().enumerate() {
                    let clicks = tab_label(
                        ui,
                        egui::Id::new(("tab", tab.id)),
                        tab_rect(i),
                        palette,
                        i == self.active,
                        &tab_title(&tab.text, i),
                        &close_shortcut,
                    );
                    if clicks.select {
                        switch_to = Some(i);
                    }
                    if clicks.close {
                        close_idx = Some(i);
                    }
                    if clicks.hovered {
                        highlighted.push(i);
                    }
                }
                let divider = palette.comment.gamma_multiply(0.4);
                for i in 0..self.tabs.len() {
                    if divider_after(i, self.tabs.len(), &highlighted) {
                        let rect = tab_rect(i);
                        let inset = rect.height() * 0.3;
                        ui.painter().vline(
                            rect.right(),
                            (rect.top() + inset)..=(rect.bottom() - inset),
                            egui::Stroke::new(1.0, divider),
                        );
                    }
                }
                if plus {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(strip.right() - TAB_HEIGHT, strip.top()),
                        egui::vec2(TAB_HEIGHT, TAB_HEIGHT),
                    );
                    let add = plus_button(ui, rect, palette)
                        .on_hover_text(format!("New tab ({new_shortcut})"));
                    if add.clicked() {
                        self.new_tab();
                    }
                }
                if let Some(i) = switch_to {
                    self.active = i;
                }
                if let Some(i) = close_idx {
                    self.close_tab(i);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(tabs: Vec<Tab>, active: usize) -> SoosApp {
        let saved = SavedState {
            tabs,
            active,
            ..Default::default()
        };
        SoosApp::from_saved(
            saved,
            RateSource::new(std::path::PathBuf::new()),
            Recalculator::spawn(egui::Context::default()),
        )
    }

    fn tab(id: u64, text: &str) -> Tab {
        Tab {
            id,
            text: text.to_string(),
        }
    }

    #[test]
    fn tab_title_is_the_first_line_without_its_marker() {
        assert_eq!(tab_title("", 0), "Tab 1");
        assert_eq!(tab_title("\n\n   \n", 2), "Tab 3");
        assert_eq!(tab_title("# Budget\n1 + 1", 0), "Budget");
        assert_eq!(tab_title("// a note\n1 + 1", 0), "a note");
    }

    /// Tabs share the strip, so a tab's width depends on how many there
    /// are, never on its title.
    #[test]
    fn tabs_share_the_strip_equally() {
        let plus = TAB_HEIGHT + PLUS_GAP;
        assert_eq!(tab_width(500.0 + plus, 2, true), 250.0);
        assert_eq!(tab_width(540.0, 9, false), 60.0);
        assert_eq!(tab_width(10.0, 3, true), 0.0);
    }

    /// The `×`'s place is kept whether or not it shows, so the title never
    /// moves when it appears; a tab too narrow for a title gets no room.
    #[test]
    fn title_room_keeps_the_close_buttons_place() {
        let tab =
            |width| egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(width, 30.0));
        let reserved = TAB_H_PAD + CLOSE_EDGE_PAD + CLOSE_SIZE + CLOSE_GAP;
        let room = title_room(tab(200.0));
        assert_eq!(room.width(), 200.0 - reserved);
        let room_left = match CLOSE_ON_LEADING_EDGE {
            true => 100.0 + CLOSE_EDGE_PAD + CLOSE_SIZE + CLOSE_GAP,
            false => 100.0 + TAB_H_PAD,
        };
        assert_eq!(room.left(), room_left);
        assert_eq!(title_room(tab(10.0)).width(), 0.0);
    }

    /// At the default window width, three tabs each fit a 13-character
    /// title like "Trip to Tokyo".
    #[test]
    fn three_tabs_fit_a_short_title_at_the_default_width() {
        let strip = DEFAULT_WIDTH - 2.0 * f32::from(TAB_BAR_H_PAD);
        let width = tab_width(strip, 3, true);
        let tab = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, TAB_HEIGHT));
        let (title, cut) = fit_title("Trip to Tokyo", title_room(tab).width());
        assert!(!cut, "{title}");
    }

    /// Cut by the room the tab has, not a fixed length, and never leaving a
    /// space before the `…`.
    #[test]
    fn fit_title_cuts_by_width() {
        let chars = |n: usize| n as f32 * CHAR_WIDTH;
        assert_eq!(
            fit_title("Trip to Tokyo", chars(13)),
            ("Trip to Tokyo".to_owned(), false)
        );
        assert_eq!(
            fit_title("Trip to Tokyo", chars(20)),
            ("Trip to Tokyo".to_owned(), false)
        );
        assert_eq!(
            fit_title("Trip to Tokyo", chars(9)),
            ("Trip to\u{2026}".to_owned(), true)
        );
        assert_eq!(
            fit_title("Groceries this week", chars(11)),
            ("Groceries\u{2026}".to_owned(), true)
        );
        assert_eq!(fit_title("Rent", 0.0), ("\u{2026}".to_owned(), true));
    }

    #[test]
    fn dividers_only_separate_two_tabs_at_rest() {
        // Four tabs, the second active: no divider touches it, and none
        // after the last tab.
        let dividers: Vec<bool> = (0..4).map(|i| divider_after(i, 4, &[1])).collect();
        assert_eq!(dividers, [false, false, true, false]);
        // Hovering the third takes its dividers away too.
        let dividers: Vec<bool> = (0..4).map(|i| divider_after(i, 4, &[1, 2])).collect();
        assert_eq!(dividers, [false, false, false, false]);
    }

    #[test]
    fn load_tabs_keeps_every_saved_tab_and_finds_the_next_id() {
        let (tabs, next_id) = load_tabs(Vec::new());
        assert_eq!((tabs.len(), next_id), (1, 1));

        let saved: Vec<Tab> = (0..20).map(|i| tab(i * 2, "")).collect();
        let (tabs, next_id) = load_tabs(saved);
        assert_eq!((tabs.len(), next_id), (20, 39));
    }

    #[test]
    fn new_tab_adds_and_switches_and_stops_at_cap() {
        let mut app = test_app(vec![tab(0, "")], 0);
        app.new_tab();
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1);

        for _ in 0..(MAX_TABS - 2) {
            app.new_tab();
        }
        assert_eq!(app.tabs.len(), MAX_TABS);
        app.new_tab();
        assert_eq!(app.tabs.len(), MAX_TABS);
    }

    #[test]
    fn close_tab_before_active_keeps_the_same_tab_selected() {
        let mut app = test_app(vec![tab(0, "a"), tab(1, "b"), tab(2, "c")], 2);
        app.close_tab(0);
        assert_eq!(app.tabs[app.active].text, "c");
    }

    #[test]
    fn close_active_tab_selects_the_next_one() {
        let mut app = test_app(vec![tab(0, "a"), tab(1, "b"), tab(2, "c")], 0);
        app.close_tab(0);
        assert_eq!(app.tabs[app.active].text, "b");
    }

    #[test]
    fn close_last_tab_selects_the_new_last_tab() {
        let mut app = test_app(vec![tab(0, "a"), tab(1, "b")], 1);
        app.close_tab(1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn closing_the_only_tab_leaves_an_empty_one() {
        let mut app = test_app(vec![tab(0, "keep me")], 0);
        app.close_tab(0);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].text, "");
        assert_eq!(app.closed[0].text, "keep me");
    }

    #[test]
    fn undo_close_tab_restores_the_most_recently_closed() {
        let mut app = test_app(vec![tab(0, "a"), tab(1, "b")], 0);
        app.close_tab(1);
        app.undo_close_tab();
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[1].text, "b");
        assert_eq!(app.active, 1);
    }

    #[test]
    fn undo_close_tab_does_nothing_with_nothing_closed_or_at_the_cap() {
        let mut app = test_app(vec![tab(0, "a")], 0);
        app.undo_close_tab();
        assert_eq!(app.tabs.len(), 1);

        let mut full = test_app((0..MAX_TABS as u64).map(|i| tab(i, "")).collect(), 0);
        full.closed.push(tab(999, "waiting"));
        full.undo_close_tab();
        assert_eq!(full.tabs.len(), MAX_TABS);
        assert_eq!(full.closed.len(), 1);
    }

    #[test]
    fn closed_tabs_list_drops_the_oldest() {
        let tabs: Vec<Tab> = (0..(MAX_CLOSED_TABS as u64 + 5))
            .map(|i| tab(i, ""))
            .chain(std::iter::once(tab(9000, "")))
            .collect();
        let mut app = test_app(tabs, 0);
        while app.tabs.len() > 1 {
            app.close_tab(0);
        }
        assert_eq!(app.closed.len(), MAX_CLOSED_TABS);
        assert!(app.closed.iter().all(|t| t.id != 0));
    }
}
