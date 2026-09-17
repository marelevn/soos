//! Soos: a document where every line is an expression and the answer shows
//! up beside it. One native multiline `TextEdit` for the source (so
//! selection, undo, copy/paste and the caret all just work), with results
//! painted into a right-hand gutter and syntax colours applied through
//! `TextEdit::layouter`. Long lines soft-wrap; the answer stays pinned
//! beside the wrapped line's first visual row (see `SoosApp::ui`).
//!
//! Theme follows the OS until the status bar's own toggle is clicked (egui's
//! `ThemePreference`, persisted by eframe automatically -- see
//! `SoosApp::theme_control`), light and dark both covered by `Palette`'s
//! two constants, `DARK`/`LIGHT`.
//!
// Wrapped continuation rows get no hanging indent. egui's LayoutJob has no
// per-row indent knob, and faking one means mutating the `Arc<Galley>` the
// layouter's cache hands back -- fragile, and the gutter already shows
// where each logical line starts (a result only ever paints beside a
// line's first row, see `SoosApp::ui` below).
//
// A release build is windowed, not console -- a debug build keeps its
// console so `cargo run`'s panics and eprintln! output still show up.
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
use soos_core::{currency::RateSource, highlight::TokenKind, LineResult};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

const FONT_SIZE: f32 = 16.0;

/// JetBrains Mono's own advance width, read directly from the bundled
/// assets/fonts/JetBrainsMono-Regular.ttf's hmtx table: exactly 600/1000em
/// for every glyph (it's monospace by design -- confirmed by decoding the
/// font's own hmtx entries, not assumed), so at `FONT_SIZE` this is exact,
/// not an estimate, for anything rendered via `mono()`. Every fixed pixel
/// width in this file that needs to fit "N characters" (the converter
/// grid's columns, the main window's own minimum) is computed from this
/// single number instead of picked by hand.
const CHAR_WIDTH: f32 = FONT_SIZE * 0.6;

/// The one text font every surface in the app renders through, so nothing
/// drifts onto egui's default proportional font by omission.
fn mono() -> FontId {
    FontId::monospace(FONT_SIZE)
}

const LINE_HEIGHT: f32 = 28.0; // Numi's airy row pitch, ~1.75x FONT_SIZE.
                               // FontTweak::y_offset is purely a rendering-time shift of each glyph's
                               // rasterized bitmap (baked into its atlas UV offset, see epaint's
                               // font.rs::allocate_glyph) -- it never touches ascent/row-height/layout, so
                               // the caret (sized from the row, see epaint's cursor_rect) and this offset
                               // can be tuned independently. Without it, every section shares LINE_HEIGHT
                               // as its line_height, so epaint's per-row valign term is always zero (see
                               // text_layout.rs) and all the extra leading above FONT_SIZE's natural row
                               // height lands below the glyphs -- text sits high, caret runs past it.
                               // Calibrated against JetBrainsMono-Regular's own metrics at 16pt --
                               // recalibrate by screenshot comparison if FONT_SIZE, LINE_HEIGHT or
                               // the font file change.
const GLYPH_Y_OFFSET: f32 = 4.0;

const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");

/// Install JetBrains Mono as the primary `Monospace` font, ahead of egui's
/// bundled Hack -- kept as a fallback for any glyph JetBrains Mono lacks
/// rather than dropped, the same pattern egui's own defaults use for emoji.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrainsMono".to_owned(),
        Arc::new(
            egui::FontData::from_static(JETBRAINS_MONO_REGULAR).tweak(egui::FontTweak {
                y_offset: GLYPH_Y_OFFSET,
                ..Default::default()
            }),
        ),
    );
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".to_owned());
    ctx.set_fonts(fonts);
}

/// The default show/hide hotkey. On Windows and Linux this is the OS's own
/// dedicated "Calculator" key (`Code::LaunchApp2`) -- registerable at all
/// only because of the local patch pinned in the workspace `Cargo.toml`'s
/// `[patch.crates-io]` (upstream's key-mapping tables omit it on every
/// platform; see that stanza's comment for the two-line diff). macOS has no
/// Carbon-level equivalent for a physical Calculator button, so it keeps the
/// modifier-based combo instead.
fn default_hotkey() -> HotKey {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        HotKey::new(None, Code::LaunchApp2)
    }
    // Everywhere else (macOS, and any other target): no physical Calculator
    // key to bind, so fall back to the modifier-based combo.
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space)
    }
}

/// Reconstruct the saved hotkey from `SavedState`'s two independently-
/// persisted parts (see that struct's doc comment for why not a round-trip
/// through `HotKey`'s own string parser). Falls back to `default_hotkey()`
/// if nothing was saved yet, or a saved `Code` name no longer parses (e.g.
/// this build's `keyboard-types` version dropped a variant).
fn load_hotkey(saved: &SavedState) -> HotKey {
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

/// Display text for the status bar's hotkey label -- `Code`'s own `Display`
/// prints the bare default binding as the technical-looking "LaunchApp2",
/// so that one case is special-cased to the name of the physical key it
/// actually is. Everything else (including a user-chosen rebind) falls
/// through to `HotKey`'s own `Display` (`"control+shift+Space"`-style).
fn hotkey_label(hk: &HotKey) -> String {
    if hk.mods.is_empty() && hk.key == Code::LaunchApp2 {
        "Calculator key".to_string()
    } else {
        hk.to_string()
    }
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

/// `https://api.github.com/repos/<owner>/<repo>/releases/latest` -- the
/// owner/repo comes from the crate's own `repository` field
/// (`repository.workspace = true` in `Cargo.toml`), not hardcoded a second
/// time. `None` if that field is ever missing or malformed.
fn latest_release_url() -> Option<String> {
    let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let (_, path) = repo.split_once("github.com/")?;
    Some(format!(
        "https://api.github.com/repos/{path}/releases/latest"
    ))
}

/// Parses `"1.2.3"` (an optional leading `v` stripped) into a numeric
/// tuple, so `0.9.0 < 0.10.0` compares correctly -- a plain string compare
/// would get that backwards.
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Runs on a background thread (see `SoosApp::check_for_update`) -- never
/// claims "up to date" on a network error, a non-2xx status or an
/// unparseable version, only on a genuine successful comparison.
fn check_latest_release() -> UpdateStatus {
    // GitHub's API 403s any request with no User-Agent header.
    let Some((latest, release)) = latest_release_url()
        .and_then(|url| ureq::get(&url).header("User-Agent", "soos-app").call().ok())
        .and_then(|mut r| r.body_mut().read_json::<GithubRelease>().ok())
        .and_then(|release| Some((parse_version(&release.tag_name)?, release)))
    else {
        return UpdateStatus::Failed;
    };
    let current = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
    if latest > current {
        UpdateStatus::Available {
            version: release.tag_name,
            url: release.html_url,
        }
    } else {
        UpdateStatus::UpToDate
    }
}

/// The centered hint for the current `UpdateStatus` -- a fallback shown
/// only when nothing else claimed the slot this frame, see `status_bar`.
fn update_hint(status: &UpdateStatus, palette: Palette) -> Hint {
    let (text, color, action) = match status {
        UpdateStatus::Checking => (
            "Checking for updates\u{2026}".to_string(),
            palette.comment,
            HintAction::None,
        ),
        UpdateStatus::UpToDate => (
            "You're up to date".to_string(),
            palette.comment,
            HintAction::None,
        ),
        UpdateStatus::Available { version, url } => (
            format!("New version {version} available"),
            palette.result,
            HintAction::OpenUrl(url.clone()),
        ),
        UpdateStatus::Failed => (
            "Couldn't check for updates".to_string(),
            palette.error,
            HintAction::None,
        ),
    };
    Hint {
        text,
        color,
        t: 1.0,
        action,
    }
}

/// Opens `url` in the user's default browser via a plain OS shell-out --
/// deliberately not eframe's `links` feature. That feature pulls in
/// `webbrowser`, which drags in the `url`/`idna`/full-ICU chain -- not
/// currently part of `soos-app`'s dependency graph at all (verified via
/// `cargo tree -p soos-app`), and shelling out means it stays that way.
fn open_url(url: &str) {
    // `explorer.exe` opens a URL via the registered default handler without
    // reinterpreting it as a command line the way `cmd /C` would -- `url`
    // comes from a deserialized GitHub API response, so it must never reach
    // an actual shell.
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Maps the keys a user could plausibly pick for a global shortcut --
/// letters, digits, F-keys, common navigation/editing keys and punctuation.
/// Deliberately excludes bare modifier keys (`ShiftLeft`, etc.) and numpad-
/// specific variants egui doesn't distinguish from their main-row twins.
/// `None` means "not offered as a hotkey key", not "invalid input" --
/// callers show a short message and let the user try again.
fn egui_key_to_hotkey_code(key: egui::Key) -> Option<Code> {
    if is_bare_modifier_key(key) {
        return None;
    }
    // `Code: FromStr` (keyboard-types 0.7) accepts exact physical-key-code
    // names like "Digit0"/"BracketLeft", and `egui::Key::name()` already
    // returns those names verbatim for most keys -- only letters/digits/
    // arrows need a prefix, and four names differ outright.
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

fn egui_modifiers_to_hotkey_modifiers(m: egui::Modifiers) -> Modifiers {
    let mut mods = Modifiers::empty();
    if m.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if m.shift {
        mods |= Modifiers::SHIFT;
    }
    if m.alt {
        mods |= Modifiers::ALT;
    }
    if m.mac_cmd {
        mods |= Modifiers::SUPER;
    }
    mods
}

/// True for egui's own physical modifier-key variants (`ShiftLeft`, etc.) --
/// these arrive as ordinary `Event::Key` presses, distinct from the
/// `modifiers` field every key event also carries. A hotkey capture must
/// skip them and wait for the actual key the modifiers are held alongside.
fn is_bare_modifier_key(key: egui::Key) -> bool {
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

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    plain: Color32,
    result: Color32,
    error: Color32,
    /// `sum`/`avg`/`today`/... and `in`/`of`/`on`/... render as the same
    /// blue in every reference screenshot -- see `highlight.rs`'s doc
    /// comment for why they're still two `TokenKind`s in the data model.
    keyword: Color32,
    label: Color32,
    comment: Color32,
    /// The example overlay's modal scrim (see `Modal::backdrop_color`).
    /// Needs its own per-theme value rather than one hardcoded black alpha:
    /// the same alpha barely darkens an already-dark background but reads
    /// as a heavy smear over a light one, so dark gets a stronger scrim and
    /// light a softer one.
    backdrop: Color32,
}

impl Palette {
    const DARK: Palette = Palette {
        background: Color32::from_rgb(0x22, 0x24, 0x28),
        plain: Color32::from_rgb(0xf0, 0xf0, 0xf0),
        result: Color32::from_rgb(0x96, 0xc8, 0x5a),
        error: Color32::from_rgb(0xd0, 0x50, 0x50),
        keyword: Color32::from_rgb(0x5b, 0xc8, 0xe8),
        label: Color32::from_rgb(0xe5, 0xc0, 0x7b),
        comment: Color32::from_rgb(0x5c, 0x63, 0x70),
        backdrop: Color32::from_black_alpha(180),
    };

    const LIGHT: Palette = Palette {
        background: Color32::from_rgb(0xf7, 0xf8, 0xfa),
        plain: Color32::from_rgb(0x14, 0x16, 0x1a),
        result: Color32::from_rgb(0x3f, 0x8f, 0x2f),
        error: Color32::from_rgb(0xc0, 0x20, 0x20),
        keyword: Color32::from_rgb(0x0b, 0x7e, 0xa3),
        label: Color32::from_rgb(0x9a, 0x6b, 0x00),
        comment: Color32::from_rgb(0x6b, 0x72, 0x7d),
        backdrop: Color32::from_black_alpha(90),
    };
}

fn main() -> eframe::Result<()> {
    // The window's *minimum* comes purely from the calculator itself
    // (`MAIN_MIN_WIDTH`) -- the converters modal is a secondary feature and
    // shouldn't force the main window any wider than the calculator alone
    // needs; a user who never opens it can drag the window down to that
    // floor. The *default* launch size (no persisted size yet) still ought
    // to show the converters modal cleanly without anyone needing to resize
    // first, so it takes whichever of the two is larger.
    let default_width = MAIN_MIN_WIDTH.max(CONVERTERS_MODAL_MIN_WINDOW_WIDTH);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([default_width, 680.0])
            // Below `MAIN_MIN_WIDTH` the calculator's own gutter+text
            // layout (see `gutter_width`, `MIN_GUTTER`) has nowhere left to
            // shrink without a calculation going illegible -- this stops
            // the user from *dragging* the window narrower than that
            // (winit enforces min_inner_size on interactive resize); a
            // persisted size already smaller than this restores as-is on
            // launch (a winit/DPI-scaling quirk, confirmed on a
            // 150%-scaled Windows display) but one manual resize snaps it
            // back. Deliberately independent of the converters modal (see
            // `MAIN_MIN_WIDTH`'s doc comment) -- if a window this narrow
            // then opens a popup, `SoosApp::request_wider_window` grows the
            // window to fit it instead of letting it crop.
            .with_min_inner_size([MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT])
            .with_icon(window_icon()),
        // Everything Soos saves lives beside the exe -- see
        // `currency::data_dir`'s doc comment.
        persistence_path: Some(soos_core::currency::data_dir().join("app.ron")),
        ..Default::default()
    };
    eframe::run_native(
        "Soos",
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            cc.egui_ctx
                .set_visuals_of(Theme::Dark, numi_visuals(Theme::Dark, Palette::DARK));
            cc.egui_ctx
                .set_visuals_of(Theme::Light, numi_visuals(Theme::Light, Palette::LIGHT));
            // egui's default floating scrollbar reserves no width
            // (ScrollStyle::floating's floating_allocated_width: 0.0), so
            // content lays out under it -- reserve the bar's own width
            // globally, once, here, rather than per-frame in a Ui:
            // `Ui::style_mut` clones the whole `Style` via `Arc::make_mut`
            // on every call, expensive enough to cause visible lag if done
            // every frame an overlay with a scrollbar is open.
            cc.egui_ctx.all_styles_mut(|style| {
                style.spacing.scroll.floating_allocated_width = style.spacing.scroll.bar_width;
            });
            Ok(Box::new(SoosApp::new(cc)))
        }),
    )
}

fn window_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icons/hicolor/256x256.png"))
        .expect("bundled window icon is a valid png")
}

/// Decode the bundled tray glyph into a `tray_icon::Icon` -- via `eframe`'s
/// PNG loader rather than pulling in `image` directly ourselves.
fn tray_icon_image() -> Icon {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/icons/hicolor/32x32.png"
    ))
    .expect("bundled tray icon is a valid png");
    Icon::from_rgba(icon.rgba, icon.width, icon.height)
        .expect("bundled tray icon has valid dimensions")
}

fn numi_visuals(theme: Theme, palette: Palette) -> egui::Visuals {
    let mut visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    visuals.panel_fill = palette.background;
    visuals.extreme_bg_color = palette.background;
    visuals.override_text_color = Some(palette.plain);
    // Blended toward `background`, not multiplied toward black: multiplying
    // the *light*-theme keyword toward black produced a dark smear behind
    // selected text instead of a highlight -- blending toward the theme's
    // own background instead gives a dim wash in dark mode and a light
    // tint in light mode, from the same formula.
    visuals.selection.bg_fill = palette.background.lerp_to_gamma(palette.keyword, 0.35);
    visuals.text_cursor.stroke.color = palette.plain;
    visuals
}

/// Messages from the tray icon / global hotkey OS callbacks (which run on
/// their own threads) into `logic()`, which runs on the egui/winit thread
/// and is the only place viewport commands may be sent from.
enum AppEvent {
    Toggle,
    Show,
    Quit,
    /// A background rate refresh (see `RateSource::refresh_in_background`)
    /// finished -- force a recalc even though `text` hasn't changed, so a
    /// currency line that errored while offline updates once rates land.
    RatesRefreshed,
    /// A background version check (see `SoosApp::check_for_update`) finished.
    UpdateChecked(UpdateStatus),
}

/// Result of a version check against GitHub Releases -- see
/// `check_latest_release`. Not persisted: a fresh launch always starts
/// unchecked (`SoosApp::update_status` is `None`).
enum UpdateStatus {
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed,
}

/// What persists across relaunch via eframe's own storage (see
/// `SoosApp::save`).
///
/// The hotkey is stored as its two independent parts, not round-tripped
/// through `HotKey`'s own string parser (`"control+shift+Space"`-style) --
/// that convenience parser has no token for `LaunchApp2` at all (it's not
/// in its keyword table), so it can't round-trip the default binding.
/// `Code`'s *own* `Display`/`FromStr` do support it, independently of that
/// higher-level parser -- see `load_hotkey`/`SoosApp::save`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SavedState {
    text: String,
    high_precision: bool,
    #[serde(default)]
    hotkey_code: Option<String>,
    #[serde(default)]
    hotkey_mods: u32,
    /// The user's own unit converters -- kept separate from `text` so
    /// clearing or editing the document can never lose them. See
    /// `converters_window`.
    #[serde(default)]
    converters: Vec<ConverterRow>,
}

/// One row of the converters table: `unit` = `factor` `base`, with
/// `aliases` extra names for the same unit. Kept as raw editable strings --
/// `aliases` as the comma-text the user types, not yet split -- and turned
/// into a `soos_core::RawConverter` only when recalculating (see
/// `SoosApp::force_recalc`), which is also where the real validation happens.
#[derive(Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct ConverterRow {
    unit: String,
    aliases: String,
    base: String,
    factor: String,
}

impl ConverterRow {
    fn to_raw(&self) -> soos_core::RawConverter {
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

struct SoosApp {
    text: String,
    results: Vec<LineResult>,
    converters: Vec<ConverterRow>,
    /// Last validation of `converters` (see `converters_window`), one entry
    /// per row in order -- recomputed alongside `results` whenever either
    /// the document or the converters change.
    converter_results: Vec<Result<String, String>>,
    rates: RateSource,
    last_text: String,
    last_converters: Vec<ConverterRow>,
    /// Whether the converters window (see `converters_window`) is showing.
    /// Not persisted -- always starts closed.
    show_converters: bool,
    /// Set on open, consumed by `converters_window` the same frame to focus
    /// the first row's unit field -- without this the modal needs a click
    /// before typing works, since `stop_text_input` (see
    /// `converters_control`) only clears focus, it doesn't claim it.
    focus_first_converter: bool,
    visible: bool,
    /// macOS only, see `logic`: set once the hidden window has actually
    /// lost focus, so focus coming back afterward means the Dock restored
    /// it, not a stale focus event from the frame we just hid in.
    hidden_unfocused: bool,
    quitting: bool,
    tray_init_done: bool,
    high_precision: bool,
    current_hotkey: HotKey,
    capturing_hotkey: bool,
    /// Whether the hotkey symbol's click has revealed the current binding --
    /// not persisted, always starts hidden. See `hotkey_control`.
    show_hotkey: bool,
    /// Set after a rebind attempt fails (or picks an unmappable key);
    /// shown next to the hotkey label until the next attempt or click.
    hotkey_message: Option<String>,
    /// Whether the example overlay (see `example_preview`, `EXAMPLE_LINES`)
    /// is showing. Not persisted -- always starts closed.
    show_example: bool,
    /// Result of the last version check (see `check_for_update`), or `None`
    /// before the version label has ever been clicked. Not persisted -- a
    /// fresh launch always starts unchecked.
    update_status: Option<UpdateStatus>,
    /// The one centered status-bar hint (see `status_bar`'s doc comment),
    /// scratch state rebuilt fresh every frame -- not meaningful between
    /// frames.
    hint: Option<Hint>,
    /// In-progress "grow the window to fit a popup" animation, if any --
    /// see `request_wider_window`/`step_window_resize`. Not persisted --
    /// nothing is mid-animation at launch.
    resize_animation: Option<ResizeAnimation>,
    /// Which popup to reveal once `resize_animation` finishes -- see
    /// `request_wider_window`. Not persisted -- nothing is pending at
    /// launch.
    pending_reveal: Option<PendingReveal>,
    /// The window's minimum width last sent via `MinInnerSize` -- see
    /// `sync_min_window_width`. Starts at `MAIN_MIN_WIDTH`, matching the
    /// floor the window actually launches with.
    current_min_width: f32,
    // Kept alive for as long as the app runs -- dropping either removes the
    // tray icon / unregisters the hotkey.
    tray: Option<TrayIcon>,
    hotkeys: Option<GlobalHotKeyManager>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

/// One control's fading description, on its way to the status bar's shared
/// centered slot -- see `SoosApp::set_hint`.
struct Hint {
    text: String,
    color: Color32,
    t: f32,
    action: HintAction,
}

/// What clicking a `Hint` does, if anything -- `None` for a hint that isn't
/// clickable, plus one variant per distinct click behaviour a hint can have
/// (starting a hotkey rebind, opening a URL).
enum HintAction {
    None,
    StartHotkeyCapture,
    OpenUrl(String),
}

/// An in-progress "grow the window to fit a popup" animation -- see
/// `SoosApp::request_wider_window`/`step_window_resize`. `from_width` is
/// captured fresh from the live window rect each time a grow is requested,
/// never trusted from a previous frame's state, since the actual OS window
/// width can change at any time via the user's own manual drag-resize --
/// unlike `ctx.animate_value_with_time` (used elsewhere in this file, e.g.
/// `symbol_toggle`, for pure-UI fades with no such external ground truth
/// to go stale against).
struct ResizeAnimation {
    from_width: f32,
    to_width: f32,
    height: f32,
    start: Instant,
}

/// Which popup `step_window_resize` should reveal once a `resize_animation`
/// it started finishes -- see `request_wider_window`. Showing the popup
/// only after the window has already reached its final width avoids the
/// popup (centered on the *current* screen rect every frame, since
/// `egui::Modal` has no open animation of its own) visibly re-centering
/// itself every frame while that width is still changing underneath it.
#[derive(Clone, Copy, PartialEq)]
enum PendingReveal {
    Example,
    Converters,
}

/// How long the main window's grow-to-fit-a-popup animation takes.
const RESIZE_ANIMATION_SECS: f32 = 0.2;

impl SoosApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rates = RateSource::new(soos_core::currency::default_cache_path());
        let saved = cc
            .storage
            .and_then(|s| eframe::get_value::<SavedState>(s, eframe::APP_KEY))
            .unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        let current_hotkey = load_hotkey(&saved);
        let mut app = Self {
            text: saved.text,
            results: Vec::new(),
            converters: saved.converters,
            converter_results: Vec::new(),
            rates,
            last_text: String::new(),
            last_converters: Vec::new(),
            show_converters: false,
            focus_first_converter: false,
            visible: true,
            hidden_unfocused: false,
            quitting: false,
            tray_init_done: false,
            high_precision: saved.high_precision,
            current_hotkey,
            capturing_hotkey: false,
            show_hotkey: false,
            hotkey_message: None,
            show_example: false,
            update_status: None,
            hint: None,
            resize_animation: None,
            pending_reveal: None,
            current_min_width: MAIN_MIN_WIDTH,
            tray: None,
            hotkeys: None,
            tx,
            rx,
        };
        app.recalc();
        app
    }

    fn recalc(&mut self) {
        if self.text == self.last_text && self.converters == self.last_converters {
            return;
        }
        self.force_recalc();
    }

    /// Recalculate unconditionally -- for when something other than the
    /// text/converters changed (rates finished refreshing) but stale
    /// results need updating.
    fn force_recalc(&mut self) {
        let raw: Vec<soos_core::RawConverter> =
            self.converters.iter().map(ConverterRow::to_raw).collect();
        (self.converter_results, self.results) =
            soos_core::recalc_document(&self.text, &raw, &self.rates);
        self.last_text = self.text.clone();
        self.last_converters = self.converters.clone();
    }

    /// Hide or restore the window -- the tray icon and global hotkey's
    /// "toggle" both funnel through this, so does closing to the tray.
    /// macOS gets `Minimized` rather than `Visible(false)`: an ordered-out
    /// window leaves no Dock target, and winit registers no
    /// `NSApplicationDelegate`, so `applicationShouldHandleReopen:` never
    /// reaches this app and the Dock icon has nothing to click. Minimizing
    /// leaves the window in the Dock, where AppKit's own default reopen
    /// handling restores it.
    ///
    /// That restore doesn't reach `visible`/`hidden_unfocused` on its own,
    /// though -- see `logic`'s macOS block, which detects it (via focus
    /// returning) and calls back into this same function so egui's
    /// `minimized` flag actually clears, not just AppKit's own window
    /// state.
    fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
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

    /// Kicks off a background version check against GitHub Releases -- a
    /// no-op while one is already running. Mirrors
    /// `RateSource::refresh_in_background`'s pattern (`crates/soos-core/src/currency.rs`):
    /// a plain `std::thread::spawn`, never blocking the UI thread, waking it
    /// via `ctx.request_repaint()` once the result is in.
    fn check_for_update(&mut self, ctx: &egui::Context) {
        if matches!(self.update_status, Some(UpdateStatus::Checking)) {
            return;
        }
        self.update_status = Some(UpdateStatus::Checking);
        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        std::thread::spawn(move || {
            let status = check_latest_release();
            let _ = tx.send(AppEvent::UpdateChecked(status));
            repaint_ctx.request_repaint();
        });
    }

    /// Version bottom-left, an offline indicator when there's no cached rate
    /// yet at all (see `RateSource::has_any_rate`), and a right-hand cluster
    /// of four symbols: example, high precision, theme, hotkey. Each symbol
    /// only ever shows its own glyph at rest -- hovering or activating one
    /// fades its description into the single centered slot shared by all
    /// four (see `set_hint`), painted after the row once every control has
    /// had a chance to claim it.
    fn status_bar(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if self.capturing_hotkey {
            self.handle_hotkey_capture(ui.ctx());
        }
        self.hint = None;
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .show(ui, |ui| {
                let row = ui
                    .horizontal(|ui| {
                        let version = ui.add(
                            egui::Label::new(
                                RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                    .color(palette.comment)
                                    .font(mono()),
                            )
                            .selectable(false)
                            .sense(egui::Sense::click()),
                        );
                        if version
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.check_for_update(ui.ctx());
                        }
                        if !self.rates.has_any_rate() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new("rates offline")
                                        .color(palette.comment)
                                        .font(mono()),
                                )
                                .selectable(false),
                            );
                        }
                        // right_to_left places the first-added item
                        // rightmost, so this order reads left-to-right as
                        // "↔ ? π ◐ ⌨".
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.hotkey_control(ui, palette);
                            self.theme_control(ui, palette);
                            self.high_precision_control(ui, palette);
                            self.example_control(ui, palette);
                            self.converters_control(ui, palette);
                        });
                    })
                    .response
                    .rect;
                // The update-check result is a fallback, not another
                // set_hint candidate: it only occupies the slot when
                // nothing else claimed it this frame, so hovering any of
                // the four symbols always shows that symbol's own hint
                // rather than racing it on `t`.
                if self.hint.is_none() {
                    if let Some(status) = &self.update_status {
                        self.hint = Some(update_hint(status, palette));
                    }
                }
                self.paint_hint(ui, row);
            });
    }

    /// The highest-`t` hint wins (ties go to whichever control ran first);
    /// see the field doc comments on `Hint` and `hint` for why one slot is
    /// enough -- a fully-revealed hotkey rebind or error message should hold
    /// the slot against a merely-hovered neighbour.
    fn set_hint(&mut self, text: impl Into<String>, color: Color32, t: f32, action: HintAction) {
        if t > 0.0 && self.hint.as_ref().is_none_or(|h| t > h.t) {
            self.hint = Some(Hint {
                text: text.into(),
                color,
                t,
                action,
            });
        }
    }

    /// Paints the frame's winning hint (if any) centered on `row`, and wires
    /// up its click per `Hint::action`. Painted manually rather than through
    /// a layout -- the text and its width aren't known until every symbol in
    /// the row has run, and `ui.allocate_rect` would grow the row and
    /// reintroduce the jitter a fixed-size fade is meant to avoid.
    fn paint_hint(&mut self, ui: &mut egui::Ui, row: egui::Rect) {
        let Some(hint) = self.hint.take() else {
            return;
        };
        let color = hint.color.gamma_multiply(hint.t);
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(hint.text, mono(), color));
        let pos = egui::pos2(
            row.center().x - galley.rect.width() / 2.0,
            row.center().y - galley.rect.height() / 2.0,
        );
        ui.painter().galley(pos, galley.clone(), color);
        if matches!(hint.action, HintAction::None) {
            return;
        }
        let rect = egui::Rect::from_min_size(pos, galley.rect.size());
        let response = ui
            .interact(rect, ui.id().with("hint"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if !response.clicked() {
            return;
        }
        match hint.action {
            HintAction::None => {}
            // Clicking the revealed hotkey binding starts a rebind.
            HintAction::StartHotkeyCapture => {
                self.capturing_hotkey = true;
                self.hotkey_message = None;
            }
            HintAction::OpenUrl(url) => open_url(&url),
        }
    }

    /// Grows the main window to `needed_width` over `RESIZE_ANIMATION_SECS`
    /// if it isn't already at least that wide -- call right before a popup
    /// that needs `needed_width` opens (see `example_control`,
    /// `converters_control`), so a window sitting at or near its own
    /// minimum never crops it. Never shrinks the window. Returns whether an
    /// animation was actually started: `false` means the window was already
    /// wide enough, so the caller's popup should open immediately; `true`
    /// means the caller should defer opening it (stash a `pending_reveal`
    /// instead) until `step_window_resize` finishes growing the window.
    fn request_wider_window(&mut self, ctx: &egui::Context, needed_width: f32) -> bool {
        let Some(inner_rect) = ctx.input(|i| i.viewport().inner_rect) else {
            return false; // platform can't report window geometry (Android/Wayland) -- no-op
        };
        if needed_width <= inner_rect.width() {
            return false;
        }
        self.resize_animation = Some(ResizeAnimation {
            from_width: inner_rect.width(),
            to_width: needed_width,
            height: inner_rect.height(),
            start: Instant::now(),
        });
        ctx.request_repaint();
        true
    }

    /// Steps any in-progress `resize_animation` -- called every frame from
    /// `logic`. Ease-out cubic: fast start, gentle settle, matching how a
    /// window growing to reveal something should feel. Only sends a resize
    /// command while actually animating, so it never fights a manual
    /// drag-resize once settled. Width is rounded to a whole pixel -- the OS
    /// would round it anyway, and re-sending a fractional size every frame
    /// is a plausible source of native resize jitter on its own.
    fn step_window_resize(&mut self, ctx: &egui::Context) {
        let Some(anim) = &self.resize_animation else {
            return;
        };
        let t = (anim.start.elapsed().as_secs_f32() / RESIZE_ANIMATION_SECS).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        let width = (anim.from_width + (anim.to_width - anim.from_width) * eased).round();
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(width, anim.height)));
        if t >= 1.0 {
            self.resize_animation = None;
            match self.pending_reveal.take() {
                Some(PendingReveal::Example) => self.show_example = true,
                Some(PendingReveal::Converters) => {
                    self.show_converters = true;
                    self.focus_first_converter = true;
                }
                None => {}
            }
        } else {
            ctx.request_repaint();
        }
    }

    /// Raises the window's minimum width to whichever popup is open needs,
    /// so it can't be dragged narrower than that and crop it -- and drops
    /// the floor back to `MAIN_MIN_WIDTH` once neither popup is open.
    /// Called once at the end of every frame from `logic`, by which point
    /// this frame's own open/close changes to `show_converters`/
    /// `show_example` have already landed. Unlike `step_window_resize` this
    /// is a discrete floor, not an animation, so it only sends a command
    /// when the target actually changes.
    fn sync_min_window_width(&mut self, ctx: &egui::Context) {
        let needed = if self.show_converters {
            CONVERTERS_MODAL_MIN_WINDOW_WIDTH
        } else if self.show_example {
            EXAMPLE_MODAL_MIN_WINDOW_WIDTH
        } else {
            MAIN_MIN_WIDTH
        };
        if needed != self.current_min_width {
            self.current_min_width = needed;
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(
                needed,
                MAIN_MIN_HEIGHT,
            )));
        }
    }

    /// `\u{2328}` symbol; click toggles `show_hotkey`, which -- along with an
    /// in-progress capture or a rebind failure -- reveals the current
    /// binding (or the failure message) in the shared centered hint.
    /// Merely hovering (not yet clicked) names the button instead, same as
    /// every other status-bar symbol.
    fn hotkey_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let reveal = self.show_hotkey || self.capturing_hotkey || self.hotkey_message.is_some();
        let id = ui.id().with("hotkey-symbol");
        let reveal_t = ui.ctx().animate_bool_with_time(id, reveal, 0.12);
        let symbol_color = palette.comment.lerp_to_gamma(palette.keyword, reveal_t);
        let symbol = symbol_label(ui, "\u{2328}", symbol_color);
        if symbol.clicked() {
            self.show_hotkey = !self.show_hotkey;
        }
        let hover_t = ui.ctx().animate_bool(id.with("hover"), symbol.hovered());
        if let Some(msg) = self.hotkey_message.clone() {
            self.set_hint(msg, palette.error, 1.0, HintAction::None);
        } else if reveal_t > 0.0 {
            let hotkey_text = if self.capturing_hotkey {
                "press a key\u{2026} (Esc to cancel)".to_string()
            } else {
                hotkey_label(&self.current_hotkey)
            };
            let action = if reveal_t > 0.5 && !self.capturing_hotkey {
                HintAction::StartHotkeyCapture
            } else {
                HintAction::None
            };
            self.set_hint(hotkey_text, palette.comment, reveal_t, action);
        } else {
            self.set_hint("Change hotkey", palette.comment, hover_t, HintAction::None);
        }
    }

    /// Shared shape behind the high-precision/theme/example status-bar
    /// symbols: animate toward `on`, lerp the glyph colour toward
    /// `on_color`, paint it, and fade `hint` in on hover. Returns whether
    /// the symbol was clicked this frame, so each caller only supplies what
    /// "on" means and what happens on click. `hotkey_control` doesn't fit
    /// this shape -- it reveals the live binding text instead of a fixed
    /// hint, so it stays its own function.
    fn symbol_toggle(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        glyph: &str,
        on: bool,
        on_color: Color32,
        hint: &str,
    ) -> bool {
        // `hint` is already a distinct label per caller, so it doubles as
        // the animation-id key -- one fewer parameter than a separate slug.
        let id = ui.id().with(hint);
        let t = ui.ctx().animate_bool_with_time(id, on, 0.12);
        let symbol_color = palette.comment.lerp_to_gamma(on_color, t);
        let symbol = symbol_label(ui, glyph, symbol_color);
        let hover_t = ui.ctx().animate_bool(id.with("label"), symbol.hovered());
        self.set_hint(hint, palette.comment, hover_t, HintAction::None);
        symbol.clicked()
    }

    /// `\u{03c0}` symbol; click toggles `high_precision` directly (its
    /// colour lerps to `palette.result` as the on/off effect).
    fn high_precision_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if self.symbol_toggle(
            ui,
            palette,
            "\u{3c0}",
            self.high_precision,
            palette.result,
            "High Precision",
        ) {
            self.high_precision = !self.high_precision;
        }
    }

    /// `\u{25CF}` symbol; click flips between light and dark via egui's own
    /// `ThemePreference` -- persisted automatically by eframe's own
    /// egui-memory persistence (see the `persistence` feature in
    /// `Cargo.toml`), so `SavedState` needs no field for it. Until the
    /// first click the window keeps following the OS: `ThemePreference`
    /// defaults to `System`, and nothing here touches it at startup.
    ///
    /// A filled circle, not a half-filled one -- `\u{25D0}` has no glyph in
    /// the bundled JetBrains Mono (verified against its own cmap table),
    /// so it silently fell back to a different bundled font that never gets
    /// `install_fonts`'s `GLYPH_Y_OFFSET` tweak, visibly misaligning it
    /// against the other status-bar symbols. `\u{25CF}` is in the font; the
    /// color lerp already carries the on/off meaning, so the glyph itself
    /// doesn't need to look different per theme.
    fn theme_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let is_dark = ui.ctx().theme() == Theme::Dark;
        if self.symbol_toggle(
            ui,
            palette,
            "\u{25cf}",
            is_dark,
            palette.keyword,
            "Light / Dark",
        ) {
            let next = match ui.ctx().theme() {
                Theme::Dark => egui::ThemePreference::Light,
                Theme::Light => egui::ThemePreference::Dark,
            };
            ui.ctx().set_theme(next);
        }
    }

    /// `?` symbol; click toggles the read-only example overlay (see
    /// `example_preview`, `EXAMPLE_LINES`).
    fn example_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if self.symbol_toggle(
            ui,
            palette,
            "?",
            self.show_example,
            palette.keyword,
            "Show example",
        ) {
            self.show_example = !self.show_example;
            // Only on the open transition, not every frame it stays open
            // (see this call's other two sites for why "every frame" is
            // wrong for an editable overlay) -- here it just keeps the live
            // document's TextEdit from eating keystrokes meant for the
            // overlay, which has nothing of its own to focus anyway.
            if self.show_example {
                ui.ctx().memory_mut(|m| m.stop_text_input());
                if self.request_wider_window(ui.ctx(), EXAMPLE_MODAL_MIN_WINDOW_WIDTH) {
                    // Window needs to grow first -- reveal once that
                    // finishes (see `PendingReveal`) instead of now.
                    self.show_example = false;
                    self.pending_reveal = Some(PendingReveal::Example);
                }
            }
        }
    }

    /// `\u{2194}` symbol; click toggles the converters window (see
    /// `converters_window`).
    fn converters_control(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if self.symbol_toggle(
            ui,
            palette,
            "\u{2194}",
            self.show_converters,
            palette.keyword,
            "Converters",
        ) {
            self.show_converters = !self.show_converters;
            // Clears the live document's focus once, on open, so the
            // overlay's own first field can be clicked into. Calling this
            // every frame instead would clear keyboard focus a fraction of
            // a second after a click sets it -- egui's `Memory::
            // stop_text_input` drops whatever widget currently holds focus
            // app-wide with no scoping, so a per-frame call would mean
            // "can never type in this modal at all".
            if self.show_converters {
                ui.ctx().memory_mut(|m| m.stop_text_input());
                // Claims the focus this clears -- see `focus_first_converter`.
                self.focus_first_converter = true;
                if self.request_wider_window(ui.ctx(), CONVERTERS_MODAL_MIN_WINDOW_WIDTH) {
                    // Window needs to grow first -- reveal once that
                    // finishes (see `PendingReveal`) instead of now.
                    self.show_converters = false;
                    self.focus_first_converter = false;
                    self.pending_reveal = Some(PendingReveal::Converters);
                }
            }
        }
    }

    /// Reads this frame's key events for the next non-repeat key-down while
    /// `capturing_hotkey` is set, and either adopts it as the new hotkey
    /// (via `apply_hotkey`), records why it couldn't be, or -- for Escape --
    /// leaves the current binding untouched. Exits capture mode either way,
    /// so a failed attempt requires clicking the label again to retry.
    fn handle_hotkey_capture(&mut self, ctx: &egui::Context) {
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
            // Holding a combo (Ctrl+Alt+K) generates its own `Event::Key`
            // for each modifier key first, so these must be skipped rather
            // than treated as "the" chosen key, or a combo's modifier press
            // itself gets grabbed before the real key ever arrives.
            if is_bare_modifier_key(key) {
                continue;
            }
            self.capturing_hotkey = false;
            if key == egui::Key::Escape {
                return;
            }
            match egui_key_to_hotkey_code(key) {
                Some(code) => {
                    let mods = egui_modifiers_to_hotkey_modifiers(modifiers);
                    self.hotkey_message = self.apply_hotkey(HotKey::new(Some(mods), code)).err();
                }
                None => self.hotkey_message = Some("can't use that key".to_string()),
            }
            return;
        }
        // No non-modifier key press yet this frame -- stay in capturing
        // mode and check again next frame.
    }

    /// Unregisters whatever's currently bound (ignoring that call's own
    /// error -- it firing just means nothing was actually registered yet,
    /// e.g. at startup) and registers `new` in its place. On failure,
    /// re-registers the previous binding so the app never ends up with no
    /// working hotkey at all, and returns a short message for the status
    /// bar. Must run on the same thread the `GlobalHotKeyManager` was
    /// created on -- true at both call sites (`init_tray_and_hotkey` and
    /// the rebind flow above), since eframe/egui itself is single-threaded.
    fn apply_hotkey(&mut self, new: HotKey) -> Result<(), String> {
        let Some(manager) = &self.hotkeys else {
            return Err("no hotkey manager available".to_string());
        };
        let _ = manager.unregister(self.current_hotkey);
        match manager.register(new) {
            Ok(()) => {
                self.current_hotkey = new;
                Ok(())
            }
            // AlreadyRegistered is only reliably distinguishable from other
            // failures on Windows and X11 -- on macOS the crate returns the
            // same FailedToRegister for a real collision and an unmappable
            // key, so macOS users just see the generic message below.
            Err(HotkeyError::AlreadyRegistered(_)) => {
                let _ = manager.register(self.current_hotkey);
                Err("hotkey occupied by other app".to_string())
            }
            Err(_) => {
                let _ = manager.register(self.current_hotkey);
                Err("can't use that as a shortcut".to_string())
            }
        }
    }

    /// Build the tray icon and register the global hotkey. Must run on the
    /// main thread with the event loop already pumping -- `logic()`'s first
    /// call, inside `eframe::App`, satisfies that on every platform.
    /// Failures (no tray host, hotkey already taken by something else) are
    /// logged and otherwise ignored: the app must keep running without them.
    fn init_tray_and_hotkey(&mut self, ctx: &egui::Context) {
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
            // The loaded/default binding, applied against a manager with
            // nothing registered yet -- apply_hotkey's own unregister(old)
            // is a harmless no-op here (see its doc comment).
            self.hotkey_message = self.apply_hotkey(self.current_hotkey).err();
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

fn text_format(color: Color32) -> TextFormat {
    TextFormat {
        line_height: Some(LINE_HEIGHT),
        ..TextFormat::simple(mono(), color)
    }
}

/// Build the coloured `LayoutJob` for the whole buffer -- called every
/// frame by `TextEdit::layouter`, using `soos_core::highlight::tokens` per
/// line so the app itself carries no lexing logic of its own.
fn build_layout_job(text: &str, wrap_width: f32, palette: Palette) -> LayoutJob {
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
            let color = match kind {
                TokenKind::Comment | TokenKind::Header => palette.comment,
                TokenKind::Label => palette.label,
                TokenKind::Keyword | TokenKind::ConversionWord => palette.keyword,
            };
            job.append(&line[range.clone()], 0.0, text_format(color));
            cursor = range.end;
        }
        if cursor < line.len() {
            job.append(&line[cursor..], 0.0, text_format(palette.plain));
        }
    }
    job
}

/// Numi-style display polish (currency symbols, shortened errors) applied
/// on top of the raw `LineResult` -- see `soos_core::format`. Returns an
/// owned `String` since both transforms may rewrite the text, not just
/// recolor it.
/// The `bool` is whether this is an error -- errors get a wider elision
/// budget than values (see `error_budget`), since unlike a value they don't
/// need to stay aligned to a shared column.
fn result_display(
    result: Option<&LineResult>,
    palette: Palette,
    high_precision: bool,
) -> Option<(String, Color32, bool)> {
    match result {
        Some(LineResult::Value(v)) => Some((
            soos_core::format::format_currency(v, high_precision),
            palette.result,
            false,
        )),
        Some(LineResult::Error(e)) => {
            Some((soos_core::format::shorten_error(e), palette.error, true))
        }
        _ => None,
    }
}

const MIN_GUTTER: f32 = 90.0;
const MAX_GUTTER_FRACTION: f32 = 0.45;
const GUTTER_PAD: f32 = 12.0;
const COLUMN_GAP: f32 = 24.0;
/// Inner margin for the whole central panel -- the only knob that actually
/// reaches every window edge. `TextEdit::margin()` looks like the natural
/// place to set this instead, but is silently discarded: `TextEdit` also
/// sets `.frame(Frame::NONE)`, and egui only applies `margin` as a fallback
/// when no explicit frame is given.
const APP_PADDING: i8 = 28;
/// Standing rule for every modal: elements never touch directly, always
/// this much breathing room between them.
const MODAL_GAP: f32 = 12.0;
/// Every modal's `Frame::inner_margin` -- named so the window-size math in
/// `main` can account for it without a second hardcoded copy of `18`
/// silently drifting from this one.
const MODAL_INNER_MARGIN: i8 = 18;

/// A `TextEdit`'s own default horizontal margin (egui's `TextEdit` builder
/// defaults to `Margin::symmetric(4, 2)`) -- added once here so each
/// converter-column width below reads as "N visible characters", not a
/// bare pixel count that has to be re-derived by eye.
const TEXT_EDIT_HPADDING: f32 = 8.0;

/// Converter grid column widths -- each sized to a realistic longest
/// example and computed from `CHAR_WIDTH`, not picked by feel. `TextEdit`'s
/// `desired_width` sets its *total* rendered footprint directly (confirmed
/// against egui's own `TextEdit` builder: the text area gets
/// `desired_width - margin`), so the same constant drives both the
/// `fixed_cell` wrapper and the widget's `desired_width` in
/// `converters_window` with no separate fudge factor between them.
const UNIT_COL_WIDTH: f32 = CHAR_WIDTH * 10.0 + TEXT_EDIT_HPADDING; // e.g. "gallon_us"
const ALIASES_COL_WIDTH: f32 = CHAR_WIDTH * 16.0 + TEXT_EDIT_HPADDING; // e.g. "ton, tonne, MT"
const BASE_COL_WIDTH: f32 = CHAR_WIDTH * 13.0 + TEXT_EDIT_HPADDING; // e.g. "nautical_mile"
const FACTOR_COL_WIDTH: f32 = CHAR_WIDTH * 12.0 + TEXT_EDIT_HPADDING; // e.g. "0.3937007874"
/// One `centered_button`'s own footprint: a single glyph plus its
/// `button_padding` (10, 6 -- see `converters_window`'s spacing override)
/// on each side.
const BUTTON_WIDTH: f32 = CHAR_WIDTH + 2.0 * 10.0;
/// Three buttons (\u{2191} \u{2193} \u{2212}) plus egui's default 8px
/// `item_spacing` between them.
const CONTROLS_COL_WIDTH: f32 = 3.0 * BUTTON_WIDTH + 2.0 * 8.0;
/// Fits every converter error message (see `document.rs`'s error sites,
/// all kept to 20 chars or under) with no extra margin -- a `Label` has
/// none, unlike `TextEdit`.
const STATUS_COL_WIDTH: f32 = CHAR_WIDTH * 20.0;

/// The converters grid's exact content width -- the *sum* of the six
/// column widths above plus the five 14px gaps between them (matching
/// `converters_window`'s `Grid::spacing`). Not a cap: `soos_modal` pins the
/// modal to exactly this, so if a column width above ever changes, the
/// modal resizes to match automatically instead of drifting out of sync
/// with it the way a second hand-picked number would.
const CONVERTERS_GRID_WIDTH: f32 = UNIT_COL_WIDTH
    + ALIASES_COL_WIDTH
    + BASE_COL_WIDTH
    + FACTOR_COL_WIDTH
    + CONTROLS_COL_WIDTH
    + STATUS_COL_WIDTH
    + 5.0 * 14.0;

/// The example overlay's own width (see `soos_modal`'s "example-doc" call
/// site) -- named here so `EXAMPLE_MODAL_MIN_WINDOW_WIDTH` below can't drift
/// from what that call site actually uses.
const EXAMPLE_MODAL_WIDTH: f32 = 480.0;

/// The narrowest main-window width that shows each popup without cropping
/// it: the popup's own pinned width plus its frame margin on each side
/// (see `soos_modal`), plus a little slack so nothing touches the window
/// edge. Used both by `main`'s default launch size and by
/// `SoosApp::request_wider_window` when a popup opens into a window
/// narrower than this.
const EXAMPLE_MODAL_MIN_WINDOW_WIDTH: f32 =
    EXAMPLE_MODAL_WIDTH + MODAL_INNER_MARGIN as f32 * 2.0 + 40.0;
const CONVERTERS_MODAL_MIN_WINDOW_WIDTH: f32 =
    CONVERTERS_GRID_WIDTH + MODAL_INNER_MARGIN as f32 * 2.0 + 40.0;

/// The main window's own minimum width: enough for a representative
/// expression (`EXAMPLE_COLUMN` -- the same 24-char budget the example
/// overlay already treats as "a typical line's width") plus the gutter's
/// own defined floor (`MIN_GUTTER`) -- independent of the converters
/// modal, which is a secondary feature and shouldn't dictate the main
/// window's minimum size (see `main`'s viewport setup for how the two are
/// reconciled for the *default*, non-minimum, launch size instead).
/// `MAIN_WIDTH_SLACK` covers the vertical scrollbar's reserved width (see
/// `main`'s `floating_allocated_width` setup) plus rounding.
const MAIN_WIDTH_SLACK: f32 = 20.0;
const MAIN_MIN_WIDTH: f32 = APP_PADDING as f32 * 2.0
    + CHAR_WIDTH * EXAMPLE_COLUMN as f32
    + COLUMN_GAP
    + MIN_GUTTER
    + MAIN_WIDTH_SLACK;

/// The main window's minimum height -- shared between `main`'s startup
/// `with_min_inner_size` and `SoosApp::sync_min_window_width`'s runtime
/// `MinInnerSize`, so the two can't drift apart.
const MAIN_MIN_HEIGHT: f32 = 480.0;

/// Adds one status-bar symbol (see `status_bar`'s four calls) with its
/// *ink* vertically centred in the row, rather than sitting on the shared
/// text baseline. JetBrains Mono's ink boxes differ per glyph (`\u{3c0}`
/// spans 550/1000 em, `?` spans 735 -- verified against
/// `assets/fonts/JetBrainsMono-Regular.ttf`'s own glyf table), so baseline
/// alignment alone makes the shorter ones read low and small. epaint
/// exposes the rasterised ink box per glyph (`Glyph::uv_rect`, offset/size
/// in points), so the nudge is derived at runtime instead of a per-glyph
/// table that would need updating whenever a symbol changes.
fn symbol_label(ui: &mut egui::Ui, glyph: &str, color: Color32) -> egui::Response {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(glyph.to_owned(), mono(), color));
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    let nudge = ink_center_offset(&galley);
    ui.painter()
        .galley(rect.min + egui::vec2(0.0, nudge), galley, color);
    response
}

/// How far to shift a single-glyph `galley` so its ink centre lands on the
/// galley box's own vertical centre. 0.0 for a galley with no ink (e.g. a
/// space), which is also what an empty galley falls back to.
fn ink_center_offset(galley: &egui::Galley) -> f32 {
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

/// A bordered, clickable button like `ui.button`, but with the same
/// ink-centering correction `symbol_label` applies. egui centers button
/// text using the font's overall line metrics; JetBrains Mono's `-`/`+`
/// glyphs sit visibly high within that box (see `ink_center_offset`'s doc
/// comment) the same way `?`/`\u{3c0}` did for the status bar.
fn centered_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
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
    // Hand-painted, so it needs its own accessibility label -- `ui.button`
    // sets this itself (see its `WidgetInfo::labeled` call), but nothing
    // does it automatically for a manually allocated+painted response.
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Widest current value result (errors are elided instead, see the gutter-
/// painting loop in `ui()`, so they never drive the gutter's width up and
/// crowd the expression column) -- the gutter sizes itself to that, clamped
/// so it can neither vanish nor swallow the editor.
fn gutter_width(
    ui: &egui::Ui,
    results: &[LineResult],
    palette: Palette,
    available_width: f32,
    high_precision: bool,
) -> f32 {
    let font_id = mono();
    let widest_value = results
        .iter()
        .filter_map(|r| match r {
            LineResult::Value(v) => Some(soos_core::format::format_currency(v, high_precision)),
            _ => None,
        })
        .map(|v| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(v, font_id.clone(), palette.result)
                    .rect
                    .width()
            })
        })
        .fold(0.0_f32, f32::max);
    (widest_value + GUTTER_PAD * 2.0).clamp(MIN_GUTTER, available_width * MAX_GUTTER_FRACTION)
}

/// An error may run left past the value column into whatever space its own
/// line leaves free -- unlike a value, it never drives `gutter_width` (see
/// that function's doc comment), so widening its budget here can't push the
/// expression column around. `row_right` is that row's own text extent;
/// `gutter_budget` (`gutter_width - GUTTER_PAD`) is the floor, so a long
/// expression with no free space still gets what a value would.
fn error_budget(gutter_right: f32, row_right: f32, gutter_budget: f32) -> f32 {
    (gutter_right - row_right - COLUMN_GAP).max(gutter_budget)
}

/// (expression text, a hand-written illustrative result) -- the showcase
/// behind the `?` symbol (see `example_preview`). A fixed list rather than
/// the real engine: it never depends on a cached exchange rate, and it
/// renders through ordinary allocated `Label`s (see `example_preview`), so
/// the surrounding `Frame`/`Modal` sizes itself correctly with no custom
/// positioning.
///
/// Basic first, Advanced below -- `example_preview`'s `ScrollArea` caps its
/// own height (`EXAMPLE_ROWS_MAX_HEIGHT`) so Basic is what a first-time user
/// sees on open, and Advanced only appears once they actually scroll to it.
/// Its natural-language phrasings (percent, `times`/`into`, `tea spoon`) are
/// kept matching the examples in `README.md`'s `## Usage` bullets.
const EXAMPLE_LINES: &[(&str, Option<&str>)] = &[
    ("# Welcome to Soos", None),
    ("", None),
    ("// Every line is an expression -- the answer shows up beside it.", None),
    ("", None),
    ("12 + 8 * 2", Some("28")),
    ("", None),
    (
        "// A label names a line. A blank line ends a block, and sum calculates the lines in the block.",
        None,
    ),
    ("Flights: $840 * 2", Some("$1680.00")),
    ("Hotel: 6 * $120", Some("$720.00")),
    ("Visas: 3 * $45", Some("$135.00")),
    ("sum", Some("$2535.00")),
    ("", None),
    ("# Variables carry down the page", None),
    ("price = 32", Some("32")),
    ("price * 8", Some("256")),
    ("prev + 100", Some("356")),
    ("", None),
    ("# Advanced", None),
    ("", None),
    ("// avg follows the same rule as sum", None),
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
    ("20 ml in tea spoons", Some("\u{2248} 4.06 teaspoons")),
    ("", None),
    ("// A variable can be a percent too", None),
    ("fee = 8%", Some("8%")),
    ("cost = 200", Some("200")),
    ("fee on cost", Some("216")),
    ("", None),
    ("# Live currency & units", None),
    ("1 USD to VND", Some("26124.50 \u{20ab}")),
    ("20 inches in cm", Some("50.8 cm")),
    ("16 px to pt", Some("12 pt")),
    ("2rem to px", Some("32 px")),
    (
        "// Your own converters live behind the \u{2194} symbol below.",
        None,
    ),
    ("", None),
    ("# Dates and timezones", None),
    ("today + 17 days", Some("2026-10-02")),
    ("now in Tokyo", Some("2026-09-15 08:45 JST")),
    ("9am PST to Tokyo", Some("2026-09-16 02:00 JST")),
    ("", None),
    (
        "// The precision toggle and the global show/hide hotkey live behind their own icons in the status bar too.",
        None,
    ),
];

/// Where `example_preview`'s answer column starts, in characters -- wide
/// enough for the longest *expression that has a result*
/// (`"20% of what is 30 cm"`, 20 chars) plus a few spaces of gap. Comment/
/// header lines run longer than this but never have a result to pad, so
/// they don't count. `saturating_sub` in the caller means a future line
/// longer than this just loses the column alignment for that one row
/// instead of underflowing.
const EXAMPLE_COLUMN: usize = 24;

/// Caps `example_preview`'s own `ScrollArea` -- same idea as
/// `CONVERTER_ROWS_MAX_HEIGHT` for the converters grid. Sized so opening the
/// popup shows the Basic section (see `EXAMPLE_LINES`) in full, with
/// `# Advanced` just past the fold rather than everything at once.
const EXAMPLE_ROWS_MAX_HEIGHT: f32 = 460.0;

/// Renders `EXAMPLE_LINES` as syntax-coloured, right-ish-aligned static
/// text -- one real `Label` per line (a normal widget allocation, unlike
/// the live document's hand-painted gutter), so a `Frame`/`Modal` wrapped
/// around this sizes itself correctly with no cropping. See `EXAMPLE_LINES`
/// for why this doesn't call into the engine at all.
fn example_preview(ui: &mut egui::Ui, palette: Palette) {
    // The floating scrollbar's width is reserved globally in main() (see
    // its comment) so this content wraps clear of the bar without needing
    // to touch this Ui's own style per frame.
    egui::ScrollArea::vertical()
        .max_height(EXAMPLE_ROWS_MAX_HEIGHT)
        .show(ui, |ui| {
            for (line, result) in EXAMPLE_LINES {
                // `build_layout_job` handles multi-line text, but a
                // newline-free `line` (every entry here is one) makes that a
                // no-op, so this is just the per-line tokenize/colour pass.
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

/// Caps the converter grid's own scroll area -- a fixed number rather than
/// derived from the window/screen size (no `Context::screen_rect` in this
/// egui version, and the window already has its own `with_min_inner_size`
/// floor) -- so past a handful of rows the header stays put and the rows
/// scroll under it instead of pushing `+ Add converter` off-screen.
const CONVERTER_ROWS_MAX_HEIGHT: f32 = 320.0;

/// Lays out one grid cell at an exact size -- so no column or row (not
/// just status-column *width*, every cell's width *and* height) can ever
/// grow or shrink with its content. `allocate_ui_with_layout` alone
/// doesn't do this: it reports the child `Ui`'s own shrink-wrapped
/// `min_rect` back to the Grid (confirmed in egui's own `scope_dyn`), so a
/// cell whose content is smaller than `size` -- a short status message, an
/// empty field, a plain header label -- under-reports its own footprint.
/// Since Grid only ever remembers the *widest/tallest a column or row has
/// been asked to be*, a cell that's never forced to fill its declared size
/// never grows the grid to it: that's what let the striped-row background
/// stop short of the actual (long) status text, and let a shorter
/// `TextEdit` sit top-aligned in a row a taller button cell had already
/// stretched. `set_min_size` (which only grows, never shrinks) closes that
/// gap -- every cell in the grid, header and data rows alike, goes through
/// this.
fn fixed_cell<R>(
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

/// One row's status cell: fixed-size (`STATUS_COL_WIDTH` x `row_height`),
/// truncated, with the full text always on hover so truncation never loses
/// information.
fn converter_status_cell(ui: &mut egui::Ui, text: &str, color: Color32, row_height: f32) {
    fixed_cell(ui, egui::vec2(STATUS_COL_WIDTH, row_height), |ui| {
        let resp =
            ui.add(egui::Label::new(RichText::new(text).color(color).font(mono())).truncate());
        if !text.is_empty() {
            resp.on_hover_text(text);
        }
    });
}

/// The converters table: one editable row per `converters` entry (unit,
/// aliases, base, factor, reorder/remove controls, and its own validation
/// status from `results`), scrolling once there are more rows than fit, plus
/// an "add" button (below the scroll area, always visible without
/// scrolling to it) that hides past `soos_core::MAX_CONVERTERS`. Removal
/// and reordering are both deferred until after the grid so mutating
/// `converters` mid-iteration can't shift indices out from under the loop.
/// `focus_first` claims keyboard focus for the first row's unit field once,
/// the frame the modal opens (see `SoosApp::converters_control`).
fn converters_window(
    ui: &mut egui::Ui,
    palette: Palette,
    converters: &mut Vec<ConverterRow>,
    results: &[Result<String, String>],
    focus_first: &mut bool,
) {
    ui.style_mut().spacing.button_padding = egui::vec2(10.0, 6.0);
    let want_focus = *focus_first;
    *focus_first = false;

    let mut remove = None;
    let mut swap = None;
    let len = converters.len();

    // One height for every cell in the grid, header row included -- a
    // `centered_button` (a glyph plus this `button_padding` on each side)
    // is taller than a `TextEdit` (a glyph plus its own smaller default
    // margin), so measuring the button's own footprint, live via the same
    // layout call `centered_button` itself uses, is what the row actually
    // needs to fit everything without a shorter cell sitting top-aligned
    // in a taller one.
    let row_height = ui
        .fonts_mut(|f| f.layout_no_wrap("\u{2191}".to_owned(), mono(), Color32::PLACEHOLDER))
        .size()
        .y
        + 2.0 * ui.spacing().button_padding.y;

    let header = |ui: &mut egui::Ui, width: f32, label: &str| {
        fixed_cell(ui, egui::vec2(width, row_height), |ui| {
            ui.label(RichText::new(label).color(palette.comment).font(mono()));
        });
    };

    egui::ScrollArea::vertical()
        .max_height(CONVERTER_ROWS_MAX_HEIGHT)
        .show(ui, |ui| {
            egui::Grid::new("converters-grid")
                .num_columns(6)
                .spacing([14.0, 10.0])
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
                                        RichText::new("mt").color(palette.comment).font(mono()),
                                    )
                                    .font(mono())
                                    .desired_width(UNIT_COL_WIDTH),
                            );
                            if i == 0 && want_focus {
                                unit_resp.request_focus();
                            }
                        });
                        fixed_cell(ui, egui::vec2(ALIASES_COL_WIDTH, row_height), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut row.aliases)
                                    .hint_text(
                                        RichText::new("ton, tonne")
                                            .color(palette.comment)
                                            .font(mono()),
                                    )
                                    .font(mono())
                                    .desired_width(ALIASES_COL_WIDTH),
                            );
                        });
                        // Compound units like `kg/m^3` are legal here (see
                        // `document::define_converter`'s charset check), but
                        // nothing else says so -- the hint is the one place
                        // that's discoverable.
                        fixed_cell(ui, egui::vec2(BASE_COL_WIDTH, row_height), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut row.base)
                                    .hint_text(
                                        RichText::new("kg").color(palette.comment).font(mono()),
                                    )
                                    .font(mono())
                                    .desired_width(BASE_COL_WIDTH),
                            );
                        });
                        fixed_cell(ui, egui::vec2(FACTOR_COL_WIDTH, row_height), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut row.factor)
                                    .hint_text(
                                        RichText::new("1000").color(palette.comment).font(mono()),
                                    )
                                    .font(mono())
                                    .desired_width(FACTOR_COL_WIDTH),
                            );
                        });
                        // No nested `ui.horizontal` here -- `fixed_cell`'s own
                        // layout is already a centered horizontal one, and an
                        // extra layout level on top of it was leaving these
                        // buttons a couple pixels off the text-edit cells'
                        // vertical center in this same row.
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

                        // Neutral, not red, until the row is actually
                        // complete -- a freshly-added row (or one the user
                        // is still typing into) names what's left rather
                        // than scolding about the field under the cursor.
                        // `aliases` is deliberately not in this list, which
                        // is also the only place that says it's optional.
                        let missing: Vec<&str> = [
                            ("unit", row.unit.trim()),
                            ("base", row.base.trim()),
                            ("factor", row.factor.trim()),
                        ]
                        .into_iter()
                        .filter(|(_, v)| v.is_empty())
                        .map(|(name, _)| name)
                        .collect();
                        if !missing.is_empty() {
                            converter_status_cell(
                                ui,
                                &format!("needs {}", missing.join(", ")),
                                palette.comment,
                                row_height,
                            );
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

    // Outside the scroll area (unlike the grid above it) so it's always
    // visible at a fixed position, never needing a scroll to reach and
    // never moving as rows are added or removed.
    ui.add_space(MODAL_GAP);
    if len >= soos_core::MAX_CONVERTERS {
        ui.label(
            RichText::new(format!("{} converters max", soos_core::MAX_CONVERTERS))
                .color(palette.comment)
                .font(mono()),
        );
    } else if centered_button(ui, "+ Add converter").clicked() {
        converters.push(ConverterRow::default());
    }

    if let Some(i) = remove {
        converters.remove(i);
    }
    if let Some((a, b)) = swap {
        converters.swap(a, b);
    }
}

impl eframe::App for SoosApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let saved = SavedState {
            text: self.text.clone(),
            high_precision: self.high_precision,
            hotkey_code: Some(self.current_hotkey.key.to_string()),
            hotkey_mods: self.current_hotkey.mods.bits(),
            converters: self.converters.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }

    /// Runs before every `ui()`, and -- critically -- also while the window
    /// is hidden (eframe skips `ui()` entirely then). Tray/hotkey/close
    /// handling all lives here so it keeps working while hidden in the tray.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.tray_init_done {
            self.tray_init_done = true;
            self.init_tray_and_hotkey(ctx);
        }

        self.step_window_resize(ctx);

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

        if !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.set_visible(ctx, false);
        }

        // macOS only: egui-winit never refreshes the `minimized` viewport
        // flag at runtime there (an upstream deadlock workaround, see
        // `set_visible`'s doc comment), so the flag `set_visible` set to
        // hide the window stays set even after the Dock has restored it --
        // eframe keeps skipping `ui()` and the last painted frame just sits
        // there, unclickable. Focus returning is the only restore signal
        // available; latch on an observed unfocus first so a stale `true`
        // from the very frame we hid in can't immediately undo it.
        if cfg!(target_os = "macos") && !self.visible {
            match ctx.input(|i| i.viewport().focused) {
                Some(false) => self.hidden_unfocused = true,
                Some(true) if self.hidden_unfocused => self.set_visible(ctx, true),
                _ => {}
            }
        }

        // Cheap no-op almost always (fresh cache, or a refresh already in
        // flight) -- see RateSource::refresh_in_background. Runs here rather
        // than in ui() so rates keep refreshing while hidden in the tray.
        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        self.rates.refresh_in_background(move || {
            let _ = tx.send(AppEvent::RatesRefreshed);
            repaint_ctx.request_repaint();
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let palette = match ui.ctx().theme() {
            Theme::Dark => Palette::DARK,
            Theme::Light => Palette::LIGHT,
        };
        egui::Frame::central_panel(&ui.style().clone())
            .fill(palette.background)
            .inner_margin(APP_PADDING)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                self.status_bar(ui, palette);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let available_width = ui.available_width();
                    let gutter_width = gutter_width(
                        ui,
                        &self.results,
                        palette,
                        available_width,
                        self.high_precision,
                    );
                    let text_width = (available_width - gutter_width - COLUMN_GAP).max(100.0);
                    let mut layouter =
                        |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                            ui.fonts_mut(|f| {
                                f.layout_job(build_layout_job(buf.as_str(), wrap_width, palette))
                            })
                        };

                    let output = egui::TextEdit::multiline(&mut self.text)
                        .frame(egui::Frame::NONE)
                        .desired_width(text_width)
                        .font(mono())
                        .layouter(&mut layouter)
                        .show(ui);

                    // Paint each result beside the first visual (post-wrap)
                    // row of its logical line -- see the module doc comment.
                    // Right-aligned at gutter_right, GUTTER_PAD in from the
                    // panel's own edge, with COLUMN_GAP of clear space
                    // before it so a wide result never crowds the text.
                    let gutter_right =
                        output.response.rect.right() + COLUMN_GAP + gutter_width - GUTTER_PAD;
                    let font_id = mono();
                    let mut line = 0usize;
                    let mut starts_line = true;
                    let gutter_budget = gutter_width - GUTTER_PAD;
                    for prow in &output.galley.rows {
                        if starts_line {
                            if let Some((text, color, is_error)) =
                                result_display(self.results.get(line), palette, self.high_precision)
                            {
                                let row_rect = prow.rect().translate(output.galley_pos.to_vec2());
                                let budget = if is_error {
                                    error_budget(gutter_right, row_rect.right(), gutter_budget)
                                } else {
                                    gutter_budget
                                };
                                // epaint's own truncate-with-ellipsis, rather
                                // than hand-shrinking the string and
                                // re-measuring it one character at a time.
                                let mut job = LayoutJob::simple(
                                    text.clone(),
                                    font_id.clone(),
                                    color,
                                    f32::INFINITY,
                                );
                                job.wrap = egui::text::TextWrapping::truncate_at_width(budget);
                                let galley = ui.fonts_mut(|f| f.layout_job(job));
                                let pos =
                                    egui::pos2(gutter_right - galley.rect.width(), row_rect.top());
                                let elided = galley.elided;
                                ui.painter().galley(pos, galley, color);
                                let click_rect = egui::Rect::from_min_size(
                                    egui::pos2(gutter_right - gutter_width, row_rect.top()),
                                    egui::vec2(gutter_width, row_rect.height()),
                                );
                                let response = ui
                                    .allocate_rect(click_rect, egui::Sense::click())
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if elided {
                                    response.clone().on_hover_text(&text);
                                }
                                if response.clicked() {
                                    ui.ctx().copy_text(text);
                                }
                            }
                        }
                        starts_line = prow.ends_with_newline;
                        if prow.ends_with_newline {
                            line += 1;
                        }
                    }
                });
            });

        if self.show_example {
            // The live document's focus was already cleared once on open
            // (see `example_control`) -- clearing it here too, every frame,
            // would fight any focus the overlay's own content tries to take.
            let modal = soos_modal(ui, palette, "example-doc", EXAMPLE_MODAL_WIDTH, |ui| {
                example_preview(ui, palette);
                // Nothing in this static preview senses clicks on its own,
                // so without this, a click anywhere in the body would fall
                // through the modal's own foreground layer instead of
                // dismissing it.
                ui.interact(ui.min_rect(), ui.id().with("dismiss"), egui::Sense::click())
                    .clicked()
            });
            if modal.should_close() || modal.inner {
                self.show_example = false;
            }
        }

        if self.show_converters {
            let modal = soos_modal(ui, palette, "converters", CONVERTERS_GRID_WIDTH, |ui| {
                converters_window(
                    ui,
                    palette,
                    &mut self.converters,
                    &self.converter_results,
                    &mut self.focus_first_converter,
                );
            });
            // Unlike the read-only example overlay, clicking inside this one
            // edits a field rather than dismissing it -- only backdrop-click
            // or Escape (both already covered by `should_close`) closes it.
            if modal.should_close() {
                self.show_converters = false;
            }
        }

        self.sync_min_window_width(ui.ctx());
        self.recalc();
    }
}

/// The shared chrome for every status-bar overlay (example, converters) --
/// same frame/background/backdrop every time, only the id, width and body
/// differ. Each caller still reads the returned `ModalResponse` to decide
/// its own close condition (a click-anywhere dismiss for the read-only
/// example; backdrop-click or Escape only, covered by `should_close`, for
/// the one that holds editable state). `width` is exact, not a cap:
/// `Ui::set_width` sets both the minimum and maximum, so the modal is
/// always exactly this size regardless of what its body measures out to --
/// nothing inside it (a status message, a row count) can move its edges.
fn soos_modal<R>(
    ui: &egui::Ui,
    palette: Palette,
    id: &str,
    width: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::ModalResponse<R> {
    egui::Modal::new(egui::Id::new(id))
        .frame(
            egui::Frame::popup(&ui.style().clone())
                .fill(palette.background)
                .inner_margin(MODAL_INNER_MARGIN),
        )
        .backdrop_color(palette.backdrop)
        .show(ui.ctx(), |ui| {
            ui.set_width(width);
            body(ui)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the status-bar symbol alignment fix: measured
    /// against JetBrainsMono-Regular's own glyf table, `\u{3c0}`'s ink box
    /// (550/1000 em) sits well above `?`'s (735/1000 em), so it needs a
    /// real, non-negligible nudge to line up -- a gutted `ink_center_offset`
    /// that always returns 0.0 would pass a same-value comparison but fail
    /// this one.
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

        let pi_nudge = offset_of("\u{3c0}");
        let question_nudge = offset_of("?");
        assert!(
            (pi_nudge - question_nudge).abs() > 0.5,
            "expected a real correction: pi={pi_nudge}, ?={question_nudge}"
        );
        // No ink at all -> no nudge, rather than an arbitrary shift.
        assert_eq!(offset_of(" "), 0.0);
    }

    #[test]
    fn tray_and_window_icons_decode() {
        let _ = window_icon();
        let _ = tray_icon_image();
    }

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
        // The four names that don't match egui::Key::name() verbatim.
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
        // Bare modifier keys are deliberately not offered as hotkey keys --
        // a global shortcut needs a "real" key, modifiers layer on top of it.
        assert_eq!(egui_key_to_hotkey_code(egui::Key::ShiftLeft), None);
    }

    /// A held modifier emits its own key-down as a distinct `Event::Key`
    /// before the actual key's -- `handle_hotkey_capture` must skip these
    /// rather than treat the modifier press itself as the chosen key.
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

    /// Round-trips a hotkey through `SavedState`'s two persisted fields and
    /// `load_hotkey` -- this is the path that must *not* go through
    /// `HotKey`'s own string parser (see `SavedState`'s doc comment), so a
    /// regression there (e.g. someone "simplifying" it back to one string)
    /// would silently break persisting the default Calculator-key binding.
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

    /// Numeric comparison, not string comparison -- `"0.9.0" < "0.10.0"`
    /// alphabetically (wrong), but `(0, 9, 0) < (0, 10, 0)` numerically
    /// (right). This is the whole reason `check_latest_release` parses
    /// before comparing instead of comparing tag strings directly.
    #[test]
    fn parse_version_compares_numerically_not_lexically() {
        assert!(parse_version("0.9.0") < parse_version("0.10.0"));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("not-a-version"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn latest_release_url_derives_from_cargo_repository() {
        // Whatever CARGO_PKG_REPOSITORY actually is at build time -- this
        // just guards the github.com/<owner>/<repo> extraction logic, not
        // the specific repo name.
        let url = latest_release_url().expect("repository field is a github.com URL");
        assert!(url.starts_with("https://api.github.com/repos/"));
        assert!(url.ends_with("/releases/latest"));
    }
}
