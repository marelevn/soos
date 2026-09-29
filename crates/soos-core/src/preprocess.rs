//! Rewrites the natural-language phrasing fend doesn't parse, and answers
//! the date and time-zone lines fend can't.
//!
//! fend-core 1.5.8 already handles `in`/`to`/`as` conversions, scale words
//! (`k`, `million`), implicit multiplication (`6(3)`), `P% of X`,
//! `@2026-12-25` date literals and `×`/`−`/`÷`. Handled here instead:
//!  - `into`, `times`, and `tea spoon(s)`;
//!  - `sin 30 deg`, which fend reads as `(sin 30) deg`;
//!  - `5 ft 11 in` and `3 in`, where fend reads `in` as a conversion;
//!  - `100 * 15%` and `100 / 20%`: fend applies `%` after the product or
//!    quotient, giving `1500%` and `5%`;
//!  - `P% on X`, `P% off X`, `P% of what is X`, `X as a % of Y`, `var on X`,
//!    and `X - P%`/`X + P%`;
//!  - currency symbols anywhere in a line (`$840`, `€5`, `A$3`, `26125 ₫`):
//!    fend only reads `$` as the first token, and no `€` or `₫` at all;
//!  - leading `Label: ` prefixes, `//` comments and `"quoted notes"`;
//!  - `today`/`tomorrow`/`yesterday`/`now` and offsets from them: fend's
//!    own `today` fails with "unable to get the current date";
//!  - time-zone conversion, which fend has no notion of.

use std::sync::LazyLock;

use chrono::{
    DateTime, Duration, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use regex::{Captures, Regex};

use crate::format::CURRENCY_STYLES;

/// What a raw document line is once comments and labels are stripped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    /// Empty, or a `//` comment.
    Blank,
    /// `# heading`.
    Header,
    /// `Label:` with nothing after it.
    Label,
    /// An expression for the engine, label and comments removed.
    Expr(String),
}

/// A `//` comment, to the end of the line. This and the next few are
/// `pub(crate)` so [`crate::highlight`] colours exactly what this module
/// strips or rewrites.
pub(crate) static TRAILING_LINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//.*$").unwrap());
pub(crate) static INLINE_QUOTED_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[^"]*""#).unwrap());
pub(crate) static INTO_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\binto\b").unwrap());
pub(crate) static TIMES_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btimes\b").unwrap());
/// An amount with a symbol before it (`$840`, `A$1,234.56`) or after it
/// (`26125 ₫`), for every symbol in [`CURRENCY_STYLES`], so each result Soos
/// shows reads back in. fend reads `,` between digits as a thousands
/// separator, so the number keeps them.
static SYMBOL_BEFORE: LazyLock<Regex> = LazyLock::new(|| symbol_regex(true));
static SYMBOL_AFTER: LazyLock<Regex> = LazyLock::new(|| symbol_regex(false));
static TEA_SPOON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btea\s+spoon(s?)\b").unwrap());
/// A leading `Label: ` followed by an expression. The colon must be
/// followed by whitespace, so `http://` isn't a label.
pub(crate) static LEADING_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{L}[\p{L}\p{M} ]*:\s+(?P<rest>\S.*)$").unwrap());
static PERCENT_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+on\s+(?P<x>.+)$").unwrap());
/// `fee on price`, for a variable holding a percent (`fee = 8%`). fend's
/// `of` only takes a literal `N%`, so this becomes `price + fee * price`,
/// and [`crate::engine`] checks that `fee` really is a percent first.
static VAR_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>[A-Za-z_][A-Za-z0-9_]*)\s+on\s+(?P<x>.+)$").unwrap());
static PERCENT_OFF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+off\s+(?P<x>.+)$").unwrap());
static PERCENT_AS_OF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<x>.+?)\s+as\s+a\s*%\s+of\s+(?P<y>.+)$").unwrap());
static PERCENT_OF_WHAT_IS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+of\s+what\s+is\s+(?P<x>.+)$").unwrap());
/// `X - P%` or `X + P%` ending a line: P% of X, as on a desk calculator.
/// fend reads `%` as 1/100, so `100 - 15%` alone is a silent 99.85 and
/// `$100 - 15%` a unit mismatch. The greedy `x` takes the last `+`/`-`
/// (`100 - 10 - 5%` is 5% off 90); an `x` with a `%` of its own
/// (`20% + 10%`) is left to fend.
static PERCENT_ADD_SUB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<x>[^%]*[^%\s])\s*(?P<op>[+-])\s*(?P<p>\d+(?:\.\d+)?)\s*%$").unwrap()
});
/// A [`PERCENT_ADD_SUB`] `x` ending like this makes the `+`/`-` a sign:
/// `2 * -5%`, `x = -5%`, `1e-5%`.
static ENDS_BEFORE_A_SIGN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[-+*/^(=<>,\u{d7}\u{f7}]|\d[eE])$").unwrap());
/// `name = rhs`, kept out of a percent phrase so the variable gets the
/// phrase's result: inside the parentheses, `price = $100 - 15%` would show
/// 85 but assign 100. `==` is a comparison.
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<lhs>[A-Za-z_][A-Za-z0-9_]*\s*=\s*)(?P<rhs>[^=\s].*)$").unwrap()
});
/// `sin 30 deg`: fend applies the function to the bare number and the unit
/// to the result, `(sin 30 rad) deg` -- a silent -0.988. The angle goes
/// inside, as in `sin(30 deg)`.
static TRIG_WITH_UNIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?P<f>sin|cos|tan)\s+(?P<n>-?\d+(?:\.\d+)?)\s*",
        r"(?P<u>\u{b0}|(?:deg|degs|degrees?|rad|rads|radians?)\b)",
    ))
    .unwrap()
});
/// What may follow a length for its `in` to be the unit and not a
/// conversion: the end, an operator, or a conversion of its own
/// (`20 in in cm`).
const AFTER_A_LENGTH: &str = r"(?P<after>\s*(?:$|[-+*/)\u{d7}\u{f7}]|(?:in|to|as)\s))";
/// `5 ft 11 in` or `5 ft 11`: fend reads a trailing `in` as a conversion,
/// and takes `5 ft 11` as feet and inches only at the very end of a line.
static FEET_AND_INCHES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(?P<ft>\d+(?:\.\d+)?)\s*(?:ft|feet|foot)\s+(?P<in>\d+(?:\.\d+)?)(?:\s*(?:inches|inch|in|\x22))?{AFTER_A_LENGTH}"
    ))
    .unwrap()
});
/// `3 in` on its own: inches, where fend expects a conversion target.
static BARE_INCHES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?P<n>(?:^|[^\w.])\d[\d,]*(?:\.\d+)?)\s*in\b{AFTER_A_LENGTH}"
    ))
    .unwrap()
});
/// A literal percent, for [`percent_factors`].
static PERCENT_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<p>\d+(?:\.\d+)?)\s*%").unwrap());
/// `today`/`tomorrow`/`yesterday`/`now` plus or minus an amount of time.
static DATE_OFFSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)^(?P<base>today|tomorrow|yesterday|now)\s*(?P<sign>[+-])\s*(?P<n>\d+)\s*",
        r"(?P<unit>minutes?|mins?|hours?|days?|weeks?|months?|years?)$",
    ))
    .unwrap()
});
static BARE_DAY_OR_NOW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(today|tomorrow|yesterday|now)$").unwrap());
/// `2026-12-25` without fend's `@`, which fend reads as subtraction.
static DATE_WITHOUT_AT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^@\d])\d{4}-\d{2}-\d{2}\b").unwrap());
/// `<now or clock time> [<from zone>] in|to <zone>`. Requiring `now` or a
/// clock time keeps `20 inches in cm` out of this branch.
static TZ_CONVERT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)^(?P<when>now|\d{1,2}:\d{2}\s*(?:am|pm)?|\d{1,2}\s*(?:am|pm))",
        r"(?:\s+(?P<from>[A-Za-z][A-Za-z_ /]*?))?",
        r"\s+(?:in|to)\s+(?P<to>[A-Za-z][A-Za-z_ /]*)$",
    ))
    .unwrap()
});
static TIME_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?P<h>\d{1,2})(?::(?P<m>\d{2}))?\s*(?P<ampm>am|pm)?$").unwrap()
});
/// Abbreviations the IANA database has no entry for. Regional ones map to
/// the region's zone, so `PST` in July means PDT, as people use it. `GMT`
/// is the fixed UTC+0 reference, not London. `IST` is read as India and
/// `CST` as US Central.
const ZONE_ABBREVIATIONS: &[(&str, &str)] = &[
    ("PST", "America/Los_Angeles"),
    ("PDT", "America/Los_Angeles"),
    ("MST", "America/Denver"),
    ("MDT", "America/Denver"),
    ("CST", "America/Chicago"),
    ("CDT", "America/Chicago"),
    ("EST", "America/New_York"),
    ("EDT", "America/New_York"),
    ("GMT", "Etc/GMT"),
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

/// Classify a raw line, stripping its label prefix, `//` comment and
/// `"quoted notes"` if it's an expression.
pub fn classify(raw: &str) -> LineKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return LineKind::Blank;
    }
    if trimmed.starts_with('#') {
        return LineKind::Header;
    }
    if is_label(trimmed) {
        return LineKind::Label;
    }
    let trimmed = LEADING_LABEL
        .captures(trimmed)
        .and_then(|caps| caps.name("rest"))
        .map_or(trimmed, |rest| rest.as_str());

    let mut expr = TRAILING_LINE_COMMENT.replace(trimmed, "").into_owned();
    expr = INLINE_QUOTED_NOTE.replace_all(&expr, "").into_owned();
    let expr = expr.trim().to_string();
    if expr.is_empty() {
        return LineKind::Blank;
    }
    LineKind::Expr(expr)
}

/// A line that's only a label: a trailing `:` with no digits before it
/// (`Costs:`). `Total 5:` has a digit, so it's an expression, and fend
/// reports the stray `:`.
pub(crate) fn is_label(trimmed: &str) -> bool {
    trimmed
        .strip_suffix(':')
        .is_some_and(|rest| !rest.chars().any(|c| c.is_ascii_digit()))
}

fn symbol_regex(before: bool) -> Regex {
    let mut symbols: Vec<&str> = CURRENCY_STYLES
        .iter()
        .filter(|s| s.symbol_first == before)
        .map(|s| s.symbol)
        .collect();
    // Longest first, so `A$` wins over `$`.
    symbols.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let symbols: Vec<String> = symbols.into_iter().map(regex::escape).collect();
    let symbols = symbols.join("|");
    // fend's own `k` and `M` scale suffixes (`$50k`); any other letter after
    // the number is a unit (`5m` is metres).
    let number = r"\d[\d,]*(?:\.\d+)?(?:[kM]\b)?";
    let pattern = if before {
        format!(r"(?P<pre>^|[^\p{{L}}\p{{N}}_])(?P<sym>{symbols})(?P<num>{number})")
    } else {
        format!(r"(?P<num>{number})\s?(?P<sym>{symbols})(?P<post>$|[^\p{{L}}\p{{N}}_])")
    };
    Regex::new(&pattern).unwrap()
}

fn currency_code(symbol: &str) -> &'static str {
    CURRENCY_STYLES
        .iter()
        .find(|s| s.symbol == symbol)
        .map_or("", |s| s.code)
}

/// An expression rewritten into a form fend parses.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rewritten {
    pub expr: String,
    /// Show the result with a `%` suffix (`X as a % of Y`).
    pub as_percent: bool,
    /// The variable in `var on X`, which must hold a percent.
    pub percent_var: Option<String>,
}

/// Rewrite the natural-language phrasing fend doesn't parse. At most one
/// percent phrase is rewritten per line; chaining two (`5% on 20% off 100`)
/// isn't supported.
pub(crate) fn rewrite(expr: &str) -> Rewritten {
    let expr = SYMBOL_BEFORE.replace_all(expr, |caps: &Captures| {
        format!(
            "{}{} {}",
            &caps["pre"],
            &caps["num"],
            currency_code(&caps["sym"])
        )
    });
    let expr = SYMBOL_AFTER
        .replace_all(&expr, |caps: &Captures| {
            format!(
                "{} {}{}",
                &caps["num"],
                currency_code(&caps["sym"]),
                &caps["post"]
            )
        })
        .into_owned();
    let expr = INTO_WORD.replace_all(&expr, "to").into_owned();
    let expr = TIMES_WORD.replace_all(&expr, "*").into_owned();
    let expr = TEA_SPOON.replace_all(&expr, "teaspoon$1").into_owned();
    let expr = TRIG_WITH_UNIT
        .replace_all(&expr, "${f}(${n} ${u})")
        .into_owned();
    let expr = FEET_AND_INCHES
        .replace_all(&expr, "(${ft} ft + ${in} inch)${after}")
        .into_owned();
    let expr = BARE_INCHES
        .replace_all(&expr, "${n} inch${after}")
        .into_owned();
    let (assign, phrase) = match ASSIGNMENT.captures(&expr) {
        Some(caps) => (caps["lhs"].to_string(), caps["rhs"].to_string()),
        None => (String::new(), expr.clone()),
    };
    let done = |rewritten: String| Rewritten {
        expr: format!("{assign}{rewritten}"),
        as_percent: false,
        percent_var: None,
    };

    if let Some(caps) = PERCENT_AS_OF.captures(&phrase) {
        let (x, y) = (caps["x"].trim(), caps["y"].trim());
        return Rewritten {
            as_percent: true,
            ..done(format!("({x}) / ({y}) * 100"))
        };
    }
    if let Some(caps) = PERCENT_OF_WHAT_IS.captures(&phrase) {
        let (p, x) = (caps["p"].trim(), caps["x"].trim());
        return done(format!("({x}) / (({p})/100)"));
    }
    if let Some(caps) = PERCENT_ON.captures(&phrase) {
        let (p, x) = (caps["p"].trim(), caps["x"].trim());
        return done(format!("({x}) + ({p})% of ({x})"));
    }
    if let Some(caps) = VAR_ON.captures(&phrase) {
        let (p, x) = (caps["p"].trim(), caps["x"].trim());
        return Rewritten {
            percent_var: Some(p.to_string()),
            ..done(format!("({x}) + ({p}) * ({x})"))
        };
    }
    if let Some(caps) = PERCENT_OFF.captures(&phrase) {
        let (p, x) = (caps["p"].trim(), caps["x"].trim());
        return done(format!("({x}) - ({p})% of ({x})"));
    }
    if let Some(caps) = PERCENT_ADD_SUB.captures(&phrase) {
        let (x, op, p) = (caps["x"].trim(), &caps["op"], &caps["p"]);
        if !ENDS_BEFORE_A_SIGN.is_match(x) {
            return done(format!("({x}) {op} ({p})% of ({x})"));
        }
    }
    Rewritten {
        expr: percent_factors(&expr),
        as_percent: false,
        percent_var: None,
    }
}

/// A literal percent multiplying, dividing or raising a plain value becomes
/// a fraction: `100 * 15%` is 15, `100 / 20%` is 500 and `4 ^ 50%` is 2.
/// fend applies `%` to the whole product, quotient or power, so it shows
/// `1500%` for the first and a wrong `5%` and `1600%` for the others. A
/// percent times a percent (`50% * 50%`), a percent divided by a number
/// (`20% / 2`) and a variable holding a percent (`2 * fee`) stay percents,
/// and `10 % 3` is modulo, not a percent.
fn percent_factors(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut copied = 0;
    for caps in PERCENT_LITERAL.captures_iter(expr) {
        let whole = caps.get(0).unwrap();
        let before = &expr[..whole.start()];
        let after = expr[whole.end()..].trim_start();
        let inside_a_token = before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '_');
        let is_percent = after.is_empty()
            || after.starts_with(['+', '-', '*', '/', ')', ',', '^', '\u{d7}', '\u{f7}']);
        if inside_a_token || !is_percent {
            continue;
        }
        let before = before.trim_end();
        let divides = before.ends_with(['/', '\u{f7}', '^']) || before.ends_with("**");
        let times_a_value = before
            .strip_suffix(['*', '\u{d7}'])
            .is_some_and(|left| !left.trim_end().ends_with(['%', '*']));
        let times_by_a_value = after.strip_prefix(['*', '\u{d7}']).is_some_and(|right| {
            let right = right.trim_start();
            !right.starts_with('*') && PERCENT_LITERAL.find(right).is_none_or(|m| m.start() != 0)
        });
        if divides || times_a_value || times_by_a_value {
            out.push_str(&expr[copied..whole.start()]);
            out.push_str(&format!("({}/100)", &caps["p"]));
            copied = whole.end();
        }
    }
    out.push_str(&expr[copied..]);
    out
}

/// Answer a date or time-zone line (`today + 3 days`, `3pm PST in Tokyo`),
/// or `None` for anything else, which then goes to fend. `now` is the
/// recalculation's clock.
pub fn eval_date(expr: &str, now: DateTime<Local>) -> Option<Result<String, String>> {
    if DATE_WITHOUT_AT.is_match(expr) {
        return Some(Err(
            "write a date with @ in front, like @2026-12-25".to_string()
        ));
    }
    if let Some(converted) = eval_timezone_at(expr, now) {
        return Some(Ok(converted));
    }
    eval_date_at(expr.trim(), now.naive_local())
}

/// Resolve a zone name: an abbreviation (`PST`), an IANA name
/// (`Asia/Tokyo`) or a city (`Tokyo`, `new york`). `None` lets the line
/// fall through to fend, so `3pm in cm` still errors normally.
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

/// `now` or a clock time, optionally with a from-zone, `in`/`to` a zone.
/// Without a from-zone, a clock time is read in this machine's zone.
fn eval_timezone_at(expr: &str, now: DateTime<Local>) -> Option<String> {
    let caps = TZ_CONVERT.captures(expr.trim())?;
    let to = resolve_zone(&caps["to"])?;

    let when = &caps["when"];
    let source: DateTime<Utc> = if when.eq_ignore_ascii_case("now") {
        now.with_timezone(&Utc)
    } else {
        let time = parse_clock(&TIME_LITERAL.captures(when)?)?;
        // `.earliest()`: on a DST fall-back the hour happens twice.
        match caps.name("from") {
            Some(m) => {
                let from = resolve_zone(m.as_str())?;
                let naive = now.with_timezone(&from).date_naive().and_time(time);
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

/// `12am` is midnight and `12pm` noon; `13pm` or `0am` is `None`.
fn parse_clock(caps: &regex::Captures) -> Option<NaiveTime> {
    let mut hour: u32 = caps["h"].parse().ok()?;
    let minute: u32 = caps.name("m").map_or(Ok(0), |m| m.as_str().parse()).ok()?;
    if let Some(ampm) = caps.name("ampm") {
        if !(1..=12).contains(&hour) {
            return None;
        }
        let is_pm = ampm.as_str().eq_ignore_ascii_case("pm");
        hour %= 12;
        if is_pm {
            hour += 12;
        }
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// A [`BARE_DAY_OR_NOW`] word or [`DATE_OFFSET`], against a given clock.
fn eval_date_at(expr: &str, now: NaiveDateTime) -> Option<Result<String, String>> {
    if BARE_DAY_OR_NOW.is_match(expr) {
        return Some(if expr.eq_ignore_ascii_case("now") {
            Ok(format_datetime(now))
        } else {
            named_day(expr, now)
                .map(format_date)
                .ok_or_else(|| "date out of range".to_string())
        });
    }
    let caps = DATE_OFFSET.captures(expr)?;
    let base = &caps["base"];
    let is_now = base.eq_ignore_ascii_case("now");
    let unit = caps["unit"].to_ascii_lowercase();
    let unit = unit.trim_end_matches('s');
    if !is_now && matches!(unit, "minute" | "min" | "hour") {
        return Some(Err(format!(
            "{} has no time of day; use now, like now + 2 hours",
            base.to_ascii_lowercase()
        )));
    }
    let start = if is_now {
        Some(now)
    } else {
        named_day(base, now).map(|day| day.and_time(NaiveTime::MIN))
    };
    let Some(start) = start else {
        return Some(Err("date out of range".to_string()));
    };
    let shifted = caps["n"]
        .parse::<i64>()
        .ok()
        .map(|n| if &caps["sign"] == "-" { -n } else { n })
        .and_then(|n| shift(start, n, unit));
    Some(match shifted {
        Some(moment) if is_now => Ok(format_datetime(moment)),
        Some(moment) => Ok(format_date(moment.date())),
        None => Err("date out of range".to_string()),
    })
}

/// The day `today`, `tomorrow` or `yesterday` names.
fn named_day(word: &str, now: NaiveDateTime) -> Option<NaiveDate> {
    let days = match word.to_ascii_lowercase().as_str() {
        "tomorrow" => 1,
        "yesterday" => -1,
        _ => 0,
    };
    now.date().checked_add_signed(Duration::try_days(days)?)
}

/// `n` comes from an unbounded digit run, so every step is checked: a panic
/// here would end the app (`panic = "abort"`).
fn shift(start: NaiveDateTime, n: i64, unit: &str) -> Option<NaiveDateTime> {
    let months = |m: i64| {
        let magnitude = Months::new(u32::try_from(m.unsigned_abs()).ok()?);
        if m >= 0 {
            start.checked_add_months(magnitude)
        } else {
            start.checked_sub_months(magnitude)
        }
    };
    match unit {
        "minute" | "min" => start.checked_add_signed(Duration::try_minutes(n)?),
        "hour" => start.checked_add_signed(Duration::try_hours(n)?),
        "day" => start.checked_add_signed(Duration::try_days(n)?),
        "week" => start.checked_add_signed(Duration::try_weeks(n)?),
        "month" => months(n),
        "year" => months(n.checked_mul(12)?),
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
    fn classify_strips_a_leading_label_in_any_script() {
        assert_eq!(classify("Price: $7 * 4"), LineKind::Expr("$7 * 4".into()));
        assert_eq!(classify("Tiền nhà: 1800"), LineKind::Expr("1800".into()));
    }

    #[test]
    fn classify_label_heuristic() {
        assert_eq!(classify("Costs:"), LineKind::Label);
        assert_eq!(classify("Total 5:"), LineKind::Expr("Total 5:".into()));
        assert_eq!(classify("x: 5"), LineKind::Expr("5".into()));
    }

    #[test]
    fn rewrite_into_as_to() {
        assert_eq!(rewrite("20 inches into cm").expr, "20 inches to cm");
    }

    #[test]
    fn rewrite_percent_on_off() {
        assert_eq!(rewrite("5% on 30").expr, "(30) + (5)% of (30)");
        assert_eq!(rewrite("6% off 40 EUR").expr, "(40 EUR) - (6)% of (40 EUR)");
    }

    /// `X - P%` is P% of X taken off; fend alone gives `100 - 0.15`.
    #[test]
    fn rewrite_percent_added_to_or_taken_off_a_value() {
        assert_eq!(rewrite("100 - 15%").expr, "(100) - (15)% of (100)");
        assert_eq!(rewrite("50+10%").expr, "(50) + (10)% of (50)");
        assert_eq!(rewrite("$100 - 15%").expr, "(100 USD) - (15)% of (100 USD)");
        assert_eq!(
            rewrite("100 - 10 - 5%").expr,
            "(100 - 10) - (5)% of (100 - 10)"
        );
    }

    /// Where the `-` is a sign, or both sides are percents, fend's reading
    /// is already right.
    #[test]
    fn rewrite_percent_leaves_signs_and_percent_sums_alone() {
        for expr in ["20% + 10%", "2 * -5%", "1e-5%", "-5%", "10 % 3", "x = -5%"] {
            assert_eq!(rewrite(expr).expr, expr);
        }
    }

    #[test]
    fn rewrite_percent_phrases_keep_an_assignment_in_front() {
        assert_eq!(
            rewrite("price = $100 - 15%").expr,
            "price = (100 USD) - (15)% of (100 USD)"
        );
        assert_eq!(rewrite("tip = 5% on 30").expr, "tip = (30) + (5)% of (30)");
        let share = rewrite("share = 30 as a % of 120");
        assert_eq!(share.expr, "share = (30) / (120) * 100");
        assert!(share.as_percent);
    }

    /// fend reads `sin 30 deg` as `(sin 30) deg`.
    #[test]
    fn rewrite_puts_a_trig_angle_unit_inside_the_function() {
        assert_eq!(rewrite("sin 30 deg").expr, "sin(30 deg)");
        assert_eq!(rewrite("cos 60\u{b0} + 1").expr, "cos(60 \u{b0}) + 1");
        assert_eq!(rewrite("tan 1 rad").expr, "tan(1 rad)");
        assert_eq!(rewrite("sin 30").expr, "sin 30");
        assert_eq!(rewrite("sin(30) deg").expr, "sin(30) deg");
    }

    #[test]
    fn rewrite_reads_in_after_a_length_as_inches() {
        assert_eq!(rewrite("5 ft 11 in").expr, "(5 ft + 11 inch)");
        assert_eq!(rewrite("5 ft 11").expr, "(5 ft + 11 inch)");
        assert_eq!(rewrite("5 ft 11 in cm").expr, "(5 ft + 11 inch) in cm");
        assert_eq!(rewrite("3 in + 2 in").expr, "3 inch + 2 inch");
        assert_eq!(rewrite("20 in in cm").expr, "20 inch in cm");
        // `in` as a conversion stays one.
        assert_eq!(rewrite("20 inches in cm").expr, "20 inches in cm");
        assert_eq!(rewrite("255 in hex").expr, "255 in hex");
        assert_eq!(rewrite("12 in cm").expr, "12 in cm");
    }

    /// fend applies `%` to a whole product or quotient: `100 / 20%` is 5%.
    #[test]
    fn rewrite_percent_factors_as_fractions() {
        assert_eq!(rewrite("100 * 15%").expr, "100 * (15/100)");
        assert_eq!(rewrite("15% * 100").expr, "(15/100) * 100");
        assert_eq!(rewrite("1 + 100 / 20%").expr, "1 + 100 / (20/100)");
        assert_eq!(rewrite("4 ^ 50%").expr, "4 ^ (50/100)");
        assert_eq!(rewrite("x = 100 * 15%").expr, "x = 100 * (15/100)");
    }

    #[test]
    fn rewrite_keeps_percent_arithmetic_and_modulo() {
        for expr in [
            "50% * 50%",
            "20% / 2",
            "2 * 10 % 3",
            "2 * 15% of 80",
            "2 * fee",
            "15%",
        ] {
            assert_eq!(rewrite(expr).expr, expr);
        }
    }

    #[test]
    fn rewrite_currency_symbols_keep_scale_suffixes() {
        assert_eq!(rewrite("$50k").expr, "50k USD");
        assert_eq!(rewrite("2 * \u{20ac}1.5M").expr, "2 * 1.5M EUR");
        assert_eq!(rewrite("$5m").expr, "5 USDm");
    }

    #[test]
    fn rewrite_percent_as_of() {
        let out = rewrite("50 as a % of 100");
        assert_eq!(out.expr, "(50) / (100) * 100");
        assert!(out.as_percent);
    }

    #[test]
    fn rewrite_percent_of_what_is() {
        assert_eq!(rewrite("20% of what is 30 cm").expr, "(30 cm) / ((20)/100)");
    }

    #[test]
    fn rewrite_times_word() {
        assert_eq!(rewrite("$8 times 3").expr, "8 USD * 3");
    }

    #[test]
    fn rewrite_currency_symbols_anywhere() {
        assert_eq!(rewrite("2 * $840").expr, "2 * 840 USD");
        assert_eq!(rewrite("$5 * $2").expr, "5 USD * 2 USD");
        assert_eq!(rewrite("$1,234.56 * 2").expr, "1,234.56 USD * 2");
        assert_eq!(rewrite("-$5.50").expr, "-5.50 USD");
        assert_eq!(rewrite("\u{20ac}37.60 * 2").expr, "37.60 EUR * 2");
        assert_eq!(rewrite("A$150.00 in USD").expr, "150.00 AUD in USD");
        assert_eq!(rewrite("CN\u{a5}780.00").expr, "780.00 CNY");
        assert_eq!(rewrite("\u{a5}16000").expr, "16000 JPY");
        assert_eq!(rewrite("26,125 \u{20ab} + 1").expr, "26,125 VND + 1");
        // Not a symbol when it's part of a word.
        assert_eq!(rewrite("ARM5").expr, "ARM5");
    }

    #[test]
    fn rewrite_tea_spoon() {
        assert_eq!(rewrite("20 ml in tea spoons").expr, "20 ml in teaspoons");
        assert_eq!(rewrite("1 tea spoon").expr, "1 teaspoon");
    }

    #[test]
    fn rewrite_var_on_names_the_variable_to_check() {
        let out = rewrite("fee on price");
        assert_eq!(out.expr, "(price) + (fee) * (price)");
        assert_eq!(out.percent_var.as_deref(), Some("fee"));
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn ok(s: &str) -> Option<Result<String, String>> {
        Some(Ok(s.to_string()))
    }

    #[test]
    fn eval_date_bare_today_and_now() {
        let now = at(2026, 9, 14, 10, 30);
        assert_eq!(eval_date_at("today", now), ok("2026-09-14"));
        assert_eq!(eval_date_at("now", now), ok("2026-09-14 10:30"));
        assert_eq!(eval_date_at("1 + 1", now), None);
    }

    #[test]
    fn eval_date_offsets() {
        let now = at(2026, 9, 14, 10, 30);
        assert_eq!(eval_date_at("today + 17 days", now), ok("2026-10-01"));
        assert_eq!(eval_date_at("today - 1 month", now), ok("2026-08-14"));
        assert_eq!(eval_date_at("today + 1 year", now), ok("2027-09-14"));
        assert_eq!(eval_date_at("now + 2 hours", now), ok("2026-09-14 12:30"));
        assert_eq!(eval_date_at("now - 45 min", now), ok("2026-09-14 09:45"));
        assert_eq!(eval_date_at("now + 1 day", now), ok("2026-09-15 10:30"));
    }

    #[test]
    fn eval_date_tomorrow_and_yesterday() {
        let now = at(2026, 12, 31, 23, 0);
        assert_eq!(eval_date_at("tomorrow", now), ok("2027-01-01"));
        assert_eq!(eval_date_at("Yesterday", now), ok("2026-12-30"));
        assert_eq!(eval_date_at("tomorrow + 1 week", now), ok("2027-01-08"));
        let result = eval_date_at("tomorrow + 2 hours", now);
        assert!(matches!(result, Some(Err(e)) if e.starts_with("tomorrow has no time")));
    }

    #[test]
    fn eval_date_today_plus_hours_says_to_use_now() {
        let result = eval_date_at("today + 2 hours", at(2026, 9, 14, 0, 0));
        assert!(matches!(result, Some(Err(e)) if e.contains("use now")));
    }

    #[test]
    fn eval_date_offsets_overflow_is_an_error_not_a_panic() {
        let now = at(2026, 9, 14, 0, 0);
        let out_of_range = Some(Err("date out of range".to_string()));
        assert_eq!(
            eval_date_at("today + 999999999999999999999 days", now),
            out_of_range
        );
        assert_eq!(eval_date_at("today - 99999999999 years", now), out_of_range);
        assert_eq!(eval_date_at("now + 99999999999 months", now), out_of_range);
        assert_eq!(shift(now, i64::MAX, "day"), None);
        assert_eq!(shift(now, i64::MAX, "year"), None);
    }

    /// A fixed instant, independent of this machine's zone.
    fn fixed_now() -> DateTime<Local> {
        Local.from_utc_datetime(&at(2026, 9, 14, 12, 0))
    }

    #[test]
    fn eval_date_rejects_a_date_without_at() {
        let result = eval_date("2026-12-25", fixed_now());
        assert!(matches!(result, Some(Err(e)) if e.contains("@2026-12-25")));
        assert_eq!(eval_date("@2026-12-25", fixed_now()), None);
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
    fn eval_timezone_converts_between_zones() {
        // 2026-09-14 is in US and EU summer time.
        assert_eq!(
            eval_timezone_at("3pm PST in CET", fixed_now()).as_deref(),
            Some("2026-09-15 00:00 CEST")
        );
        assert_eq!(
            eval_timezone_at("9am PST to Tokyo", fixed_now()).as_deref(),
            Some("2026-09-15 01:00 JST")
        );
    }

    #[test]
    fn gmt_is_utc_all_year_not_london() {
        assert_eq!(
            eval_timezone_at("12:00 GMT in UTC", fixed_now()).as_deref(),
            Some("2026-09-14 12:00 UTC")
        );
        assert_eq!(
            eval_timezone_at("12:00 BST in UTC", fixed_now()).as_deref(),
            Some("2026-09-14 11:00 UTC")
        );
    }

    #[test]
    fn eval_timezone_rejects_an_impossible_am_pm_hour() {
        assert_eq!(eval_timezone_at("13pm in UTC", fixed_now()), None);
        assert_eq!(eval_timezone_at("0am in UTC", fixed_now()), None);
        assert!(eval_timezone_at("12am in UTC", fixed_now()).is_some());
    }

    #[test]
    fn eval_timezone_now_in_utc() {
        let out = eval_timezone_at("now in UTC", fixed_now());
        assert_eq!(out.as_deref(), Some("2026-09-14 12:00 UTC"));
    }

    #[test]
    fn eval_timezone_does_not_swallow_unit_conversions() {
        assert_eq!(eval_timezone_at("20 inches in cm", fixed_now()), None);
        assert_eq!(eval_timezone_at("3 apples in a box", fixed_now()), None);
        assert_eq!(eval_timezone_at("12 in cm", fixed_now()), None);
        assert_eq!(eval_date("20 inches in cm", fixed_now()), None);
    }
}
