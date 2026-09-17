//! Line rewriting for the natural-language surface fend doesn't speak natively.
//!
//! Against fend-core 1.5.8: fend already handles `in`/`to`/`as` conversions,
//! `k`/`million`/`thousand` scale words, implicit-multiplication stacking
//! (`6(3)`), `P% of X`, and even `×`/`−`/`÷` as operators. What's missing
//! and handled here: `into`/`times` as words, `tea spoon(s)` as a unit
//! alias, `P% on X` / `P% off X` / `P% of what is X` phrasing, a variable
//! already holding a percent used with `on`, leading `Label: expr` lines,
//! and `today`/`now` date arithmetic (fend's own date engine is
//! unreachable from the public API in 1.5.8: it recognizes the `today`
//! token but fails with "unable to get the current date", and has no
//! date-literal syntax at all -- `2026-09-14` alone just evaluates as
//! subtraction), and `now`/`<clock>` timezone conversion (fend has no
//! timezone support whatsoever -- no `DateTime` type, no tz database). All
//! computed here directly with `chrono`/`chrono-tz`.

use std::sync::LazyLock;

use chrono::{
    DateTime, Duration, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use regex::Regex;

/// What a raw document line turns out to be once comments/structure are stripped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    /// Nothing to evaluate (blank, or a `//` full-line comment).
    Blank,
    /// `# ...` heading — displayed, never evaluated. No payload: nothing
    /// downstream reads the heading text itself, only that the line is one.
    Header,
    /// A trailing-`:` label with no expression content — displayed, never
    /// evaluated. Same no-payload reasoning as `Header`.
    Label,
    /// An expression to hand to fend, comment/label suffix already stripped.
    Expr(String),
}

// pub(crate): also reused by `highlight.rs` to mark the same spans as comments/words.
pub(crate) static TRAILING_LINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//.*$").unwrap());
pub(crate) static INLINE_QUOTED_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[^"]*""#).unwrap());
pub(crate) static INTO_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\binto\b").unwrap());
pub(crate) static TIMES_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btimes\b").unwrap());
// fend's own `$<number>` literal only parses when it's the first token of
// the expression -- `2 * $840` fails with "'$2' is not a function" (fend
// reads `$` as a variable-lookup sigil there instead). Rewriting to the
// ISO-code suffix form fend accepts in any position sidesteps that
// entirely; `format::format_currency` already renders a USD result back
// with a leading `$`, so the displayed answer is identical either way.
static DOLLAR_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$(\d+(?:\.\d+)?)").unwrap());
static TEA_SPOON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btea\s+spoon(s?)\b").unwrap());
// A leading "Label: " with a non-empty expression after it -- the label is
// display-only annotation and is dropped before fend ever sees the line.
// Requires a space after the colon (so "http://" isn't mistaken for one).
static LEADING_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z ]*:\s+(?P<rest>\S.*)$").unwrap());
// "<pct>% on <expr>"  ->  add pct% on top of expr
static PERCENT_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+on\s+(?P<x>.+)$").unwrap());
// "<var> on <expr>" -- var already holds a percent value (e.g. `fee = 8%`).
// fend's `of` operator only accepts a literal `N%`, not a variable of
// percent type ("fee of price" -> "expected an object"), but plain
// multiplication does work and fend correctly resolves the percent unit
// back down to a plain number once it's added to something unitless
// ("price + fee * price" -> 108, not "800%").
static VAR_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>[A-Za-z_][A-Za-z0-9_]*)\s+on\s+(?P<x>.+)$").unwrap());
// "<pct>% off <expr>" ->  subtract pct% from expr
static PERCENT_OFF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+off\s+(?P<x>.+)$").unwrap());
// "<x> as a % of <y>" -> what percentage is x of y
static PERCENT_AS_OF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<x>.+?)\s+as\s+a\s*%\s+of\s+(?P<y>.+)$").unwrap());
// "<pct>% of what is <x>" -> the whole amount that <x> is <pct>% of
static PERCENT_OF_WHAT_IS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+of\s+what\s+is\s+(?P<x>.+)$").unwrap());
// "today"/"now", optionally offset by N days/weeks/months/years.
static DATE_OFFSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:today|now)\s*(?P<sign>[+-])\s*(?P<n>\d+)\s+(?P<unit>days?|weeks?|months?|years?)$")
        .unwrap()
});
static BARE_TODAY_OR_NOW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(today|now)$").unwrap());
// "<time-or-now> [<from-zone>] in|to <zone>" -- requiring `when` to be `now`
// or a clock literal (am/pm or a colon) is what keeps "20 inches in cm" and
// similar unit conversions out of this branch.
static TZ_CONVERT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)^(?P<when>now|\d{1,2}:\d{2}\s*(?:am|pm)?|\d{1,2}\s*(?:am|pm))",
        r"(?:\s+(?P<from>[A-Za-z][A-Za-z_ /]*?))?",
        r"\s+(?:in|to)\s+(?P<to>[A-Za-z][A-Za-z_ /]*)$",
    ))
    .unwrap()
});
// A clock literal on its own, for parsing the `when` capture above.
static TIME_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?P<h>\d{1,2})(?::(?P<m>\d{2}))?\s*(?P<ampm>am|pm)?$").unwrap()
});
// Abbreviations the IANA database doesn't index by name. Standard and
// daylight forms deliberately map to the same zone -- chrono-tz resolves the
// right offset for whichever one is actually in effect on the given date, so
// "3pm PST" in July still means PDT, not a wrong hour. IST is ambiguous
// (India/Israel/Ireland); India is chosen as the common expectation.
const ZONE_ABBREVIATIONS: &[(&str, &str)] = &[
    ("PST", "America/Los_Angeles"),
    ("PDT", "America/Los_Angeles"),
    ("MST", "America/Denver"),
    ("MDT", "America/Denver"),
    ("CST", "America/Chicago"),
    ("CDT", "America/Chicago"),
    ("EST", "America/New_York"),
    ("EDT", "America/New_York"),
    ("GMT", "Europe/London"),
    ("BST", "Europe/London"),
    ("CET", "Europe/Paris"),
    ("CEST", "Europe/Paris"),
    ("EET", "Europe/Bucharest"),
    ("EEST", "Europe/Bucharest"),
    ("JST", "Asia/Tokyo"),
    ("IST", "Asia/Kolkata"),
    ("AEST", "Australia/Sydney"),
    ("AEDT", "Australia/Sydney"),
];

/// Classify a raw line and strip any comment/label decoration from it.
/// Blank/header/label lines are returned as-is (nothing left to preprocess further).
pub fn classify(raw: &str) -> LineKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return LineKind::Blank;
    }
    if trimmed.starts_with('#') {
        return LineKind::Header;
    }
    // A label is a trailing `:` with nothing that looks like an expression
    // before it (no digits) -- e.g. "Costs:" vs "x: 5" (not a label, fend
    // will just error on `x:` and that's fine, this is a heuristic).
    if let Some(rest) = trimmed.strip_suffix(':') {
        if !rest.chars().any(|c| c.is_ascii_digit()) {
            return LineKind::Label;
        }
    }
    let trimmed = if let Some(caps) = LEADING_LABEL.captures(trimmed) {
        caps.name("rest").unwrap().as_str()
    } else {
        trimmed
    };

    let mut expr = TRAILING_LINE_COMMENT.replace(trimmed, "").into_owned();
    expr = INLINE_QUOTED_NOTE.replace_all(&expr, "").into_owned();
    let expr = expr.trim().to_string();
    if expr.is_empty() {
        return LineKind::Blank;
    }
    LineKind::Expr(expr)
}

/// Rewrite the natural-language phrasing fend doesn't parse into forms it does.
/// Returns the rewritten expression, plus whether the result should be
/// displayed with a trailing `%` (for "X as a % of Y").
pub fn rewrite(expr: &str) -> (String, bool) {
    let expr = DOLLAR_LITERAL.replace_all(expr, "$1 USD").into_owned();
    let expr = INTO_WORD.replace_all(&expr, "to").into_owned();
    let expr = TIMES_WORD.replace_all(&expr, "*").into_owned();
    let expr = TEA_SPOON.replace_all(&expr, "teaspoon$1").into_owned();

    if let Some(caps) = PERCENT_AS_OF.captures(&expr) {
        let x = caps["x"].trim();
        let y = caps["y"].trim();
        return (format!("({x}) / ({y}) * 100"), true);
    }
    if let Some(caps) = PERCENT_OF_WHAT_IS.captures(&expr) {
        let p = caps["p"].trim();
        let x = caps["x"].trim();
        return (format!("({x}) / (({p})/100)"), false);
    }
    if let Some(caps) = PERCENT_ON.captures(&expr) {
        let p = caps["p"].trim();
        let x = caps["x"].trim();
        return (format!("({x}) + ({p})% of ({x})"), false);
    }
    if let Some(caps) = VAR_ON.captures(&expr) {
        let p = caps["p"].trim();
        let x = caps["x"].trim();
        return (format!("({x}) + ({p}) * ({x})"), false);
    }
    if let Some(caps) = PERCENT_OFF.captures(&expr) {
        let p = caps["p"].trim();
        let x = caps["x"].trim();
        return (format!("({x}) - ({p})% of ({x})"), false);
    }
    (expr, false)
}

/// Evaluate `today`/`now`, with or without a `+ N days|weeks|months|years`
/// offset, entirely in Rust -- see the module doc comment for why fend can't
/// do this itself. Returns `None` for anything else, so the caller falls
/// through to fend as normal.
pub fn eval_date(expr: &str) -> Option<String> {
    let now = Local::now();
    eval_timezone_at(expr, now).or_else(|| eval_date_at(expr, now.naive_local()))
}

/// Resolve a zone name typed in a document: an abbreviation (`PST`, `JST`),
/// a full IANA name (`Asia/Tokyo`), or the city alone (`Tokyo`, `new york`).
/// `None` means "not a zone we recognise" -- the caller falls through to
/// fend as normal rather than guessing, so `3pm in cm` still errors sanely.
fn resolve_zone(name: &str) -> Option<Tz> {
    let name = name.trim();
    if let Some((_, iana)) = ZONE_ABBREVIATIONS
        .iter()
        .find(|(abbr, _)| abbr.eq_ignore_ascii_case(name))
    {
        return iana.parse().ok();
    }
    if let Ok(tz) = name.parse::<Tz>() {
        return Some(tz);
    }
    let normalized = name.to_ascii_lowercase().replace(' ', "_");
    chrono_tz::TZ_VARIANTS
        .iter()
        .find(|tz| {
            tz.name()
                .rsplit('/')
                .next()
                .is_some_and(|city| city.eq_ignore_ascii_case(&normalized))
        })
        .copied()
}

/// Evaluate `now`/`<clock literal>`, optionally `<from-zone>`, `in`/`to`
/// `<zone>` entirely in Rust -- see the module doc comment for why fend has
/// no timezone support at all to hand this to. Returns `None` for anything
/// else (including an unrecognised zone name), so the caller falls through
/// to fend as normal.
fn eval_timezone_at(expr: &str, now: DateTime<Local>) -> Option<String> {
    let caps = TZ_CONVERT.captures(expr.trim())?;
    let to = resolve_zone(&caps["to"])?;

    let when = &caps["when"];
    let source: DateTime<Utc> = if when.eq_ignore_ascii_case("now") {
        now.with_timezone(&Utc)
    } else {
        let time_caps = TIME_LITERAL.captures(when)?;
        let time = parse_clock(&time_caps)?;
        match caps.name("from") {
            Some(m) => {
                let from = resolve_zone(m.as_str())?;
                let naive = now.with_timezone(&from).date_naive().and_time(time);
                // .earliest(): on a DST fall-back, the local hour occurs
                // twice -- picking the first occurrence still gives an
                // answer instead of erroring on the ambiguity.
                from.from_local_datetime(&naive)
                    .earliest()?
                    .with_timezone(&Utc)
            }
            None => {
                let naive = now.date_naive().and_time(time);
                Local
                    .from_local_datetime(&naive)
                    .earliest()?
                    .with_timezone(&Utc)
            }
        }
    };

    let converted = source.with_timezone(&to);
    Some(format!(
        "{} {}",
        format_datetime(converted.naive_local()),
        converted.format("%Z")
    ))
}

fn parse_clock(caps: &regex::Captures) -> Option<NaiveTime> {
    let mut hour: u32 = caps["h"].parse().ok()?;
    let minute: u32 = caps.name("m").map_or(Ok(0), |m| m.as_str().parse()).ok()?;
    if let Some(ampm) = caps.name("ampm") {
        let is_pm = ampm.as_str().eq_ignore_ascii_case("pm");
        hour %= 12;
        if is_pm {
            hour += 12;
        }
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn eval_date_at(expr: &str, now: NaiveDateTime) -> Option<String> {
    let trimmed = expr.trim();
    if let Some(caps) = DATE_OFFSET.captures(trimmed) {
        let n: i64 = caps["n"].parse().ok()?;
        let n = if &caps["sign"] == "-" { -n } else { n };
        let shifted = shift_date(now.date(), n, &caps["unit"])?;
        return Some(format_date(shifted));
    }
    if BARE_TODAY_OR_NOW.is_match(trimmed) {
        return Some(if trimmed.eq_ignore_ascii_case("now") {
            format_datetime(now)
        } else {
            format_date(now.date())
        });
    }
    None
}

fn shift_date(date: NaiveDate, n: i64, unit: &str) -> Option<NaiveDate> {
    let months = |m: i64| {
        if m >= 0 {
            date.checked_add_months(Months::new(m as u32))
        } else {
            date.checked_sub_months(Months::new((-m) as u32))
        }
    };
    match unit.to_ascii_lowercase().trim_end_matches('s') {
        "day" => date.checked_add_signed(Duration::days(n)),
        "week" => date.checked_add_signed(Duration::weeks(n)),
        "month" => months(n),
        "year" => months(n * 12),
        _ => None,
    }
}

fn format_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn format_datetime(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_blank_and_comments() {
        assert_eq!(classify(""), LineKind::Blank);
        assert_eq!(classify("   "), LineKind::Blank);
        assert_eq!(classify("// just a comment"), LineKind::Blank);
    }

    #[test]
    fn classify_header_and_label() {
        assert_eq!(classify("# Totals"), LineKind::Header);
        assert_eq!(classify("Costs:"), LineKind::Label);
    }

    #[test]
    fn classify_strips_trailing_and_inline_comments() {
        assert_eq!(
            classify("1 + 1 // trailing"),
            LineKind::Expr("1 + 1".into())
        );
        assert_eq!(
            classify(r#"1 + 1 "inline note""#),
            LineKind::Expr("1 + 1".into())
        );
    }

    #[test]
    fn rewrite_into_as_to() {
        assert_eq!(rewrite("20 inches into cm").0, "20 inches to cm");
    }

    #[test]
    fn rewrite_percent_on_off() {
        assert_eq!(rewrite("5% on 30").0, "(30) + (5)% of (30)");
        assert_eq!(rewrite("6% off 40 EUR").0, "(40 EUR) - (6)% of (40 EUR)");
    }

    #[test]
    fn rewrite_percent_as_of() {
        let (out, is_pct) = rewrite("50 as a % of 100");
        assert_eq!(out, "(50) / (100) * 100");
        assert!(is_pct);
    }

    #[test]
    fn rewrite_percent_of_what_is() {
        assert_eq!(rewrite("20% of what is 30 cm").0, "(30 cm) / ((20)/100)");
    }

    #[test]
    fn rewrite_times_word() {
        assert_eq!(rewrite("$8 times 3").0, "8 USD * 3");
    }

    #[test]
    fn rewrite_dollar_literal_works_in_any_position() {
        // fend's own `$<number>` literal only parses as the first token of
        // an expression -- `2 * $840` otherwise fails with "'$2' is not a
        // function". Rewriting to the ISO-code suffix form sidesteps that.
        assert_eq!(rewrite("2 * $840").0, "2 * 840 USD");
        assert_eq!(rewrite("$5 * $2").0, "5 USD * 2 USD");
        assert_eq!(rewrite("$840 * 2").0, "840 USD * 2");
    }

    #[test]
    fn rewrite_tea_spoon() {
        assert_eq!(rewrite("20 ml in tea spoons").0, "20 ml in teaspoons");
        assert_eq!(rewrite("1 tea spoon").0, "1 teaspoon");
    }

    #[test]
    fn rewrite_var_on_percent_variable() {
        assert_eq!(rewrite("fee on price").0, "(price) + (fee) * (price)");
    }

    #[test]
    fn classify_strips_leading_label() {
        assert_eq!(classify("Price: $7 * 4"), LineKind::Expr("$7 * 4".into()));
    }

    #[test]
    fn eval_date_bare_today_and_now() {
        let now = NaiveDate::from_ymd_opt(2026, 9, 14)
            .unwrap()
            .and_hms_opt(10, 30, 0)
            .unwrap();
        assert_eq!(eval_date_at("today", now), Some("2026-09-14".to_string()));
        assert_eq!(
            eval_date_at("now", now),
            Some("2026-09-14 10:30".to_string())
        );
        assert_eq!(eval_date_at("1 + 1", now), None);
    }

    #[test]
    fn eval_date_offsets() {
        let now = NaiveDate::from_ymd_opt(2026, 9, 14)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(
            eval_date_at("today + 17 days", now),
            Some("2026-10-01".to_string())
        );
        assert_eq!(
            eval_date_at("today - 1 month", now),
            Some("2026-08-14".to_string())
        );
        assert_eq!(
            eval_date_at("today + 1 year", now),
            Some("2027-09-14".to_string())
        );
    }

    /// A fixed absolute instant, tagged `Local` only because the function
    /// signature requires it -- `from_utc_datetime` fixes the instant itself
    /// (machine-independent), unlike `from_local_datetime` which would tie
    /// it to whatever timezone the test machine happens to be set to.
    fn fixed_now() -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, 9, 14)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        Local.from_utc_datetime(&naive)
    }

    #[test]
    fn resolve_zone_variants() {
        assert_eq!(resolve_zone("Tokyo"), Some(Tz::Asia__Tokyo));
        assert_eq!(resolve_zone("new york"), Some(Tz::America__New_York));
        assert_eq!(resolve_zone("Asia/Tokyo"), Some(Tz::Asia__Tokyo));
        assert_eq!(resolve_zone("JST"), Some(Tz::Asia__Tokyo));
        assert_eq!(resolve_zone("UTC"), Some(Tz::UTC));
        assert_eq!(resolve_zone("cm"), None);
        assert_eq!(resolve_zone("hours"), None);
    }

    #[test]
    fn eval_timezone_explicit_from_and_to() {
        let out = eval_timezone_at("3pm PST in CET", fixed_now());
        assert!(out.is_some());
        assert!(out.unwrap().ends_with("CEST"));
    }

    #[test]
    fn eval_timezone_now_in_utc() {
        let out = eval_timezone_at("now in UTC", fixed_now());
        assert_eq!(out, Some("2026-09-14 12:00 UTC".to_string()));
    }

    #[test]
    fn eval_timezone_does_not_swallow_unit_conversions() {
        // The regex requires `when` to be `now` or a clock literal -- plain
        // unit conversions must fall through to fend untouched.
        assert_eq!(eval_timezone_at("20 inches in cm", fixed_now()), None);
        assert_eq!(eval_timezone_at("3 apples in a box", fixed_now()), None);
        assert_eq!(eval_timezone_at("12 in cm", fixed_now()), None);
        assert_eq!(eval_date("20 inches in cm"), None);
    }
}
