//! Date, clock and time-zone lines, which Soos answers itself because fend
//! can't read the clock.

use std::sync::LazyLock;

use chrono::{
    DateTime, Datelike, Duration, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use regex::Regex;

use crate::error::LineError;

/// The units of time a date can be shifted by.
const TIME_UNIT: &str = "minutes?|mins?|hours?|days?|weeks?|months?|years?";

/// `today`/`tomorrow`/`yesterday`/`now`, a clock time or an `@` date, plus or
/// minus an amount of time.
/// [`eval_date`] runs before [`super::rewrite`], so the operator words are matched
/// here too, with the same closing `\b` as [`super::OPERATOR_WORD`].
static DATE_OFFSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^(?P<base>today|tomorrow|yesterday|now|@\d{{4}}-\d{{2}}-\d{{2}}|{CLOCK})\s*(?P<sign>{SIGN})\s*(?P<n>\d+)\s*(?P<unit>{TIME_UNIT})$"
    ))
    .unwrap()
});

/// A time of day: `3:15PM`, `15:00`, `3PM`. A bare number isn't one. `AM` and
/// `PM` are matched in capitals only, whatever the regex around this says:
/// `am` and `pm` are fend's attometre and picometre, so `74 pm in nm` is a
/// length.
pub(super) const CLOCK: &str = r"\d{1,2}:\d{2}\s*(?-i:AM|PM)?|\d{1,2}\s*(?-i:AM|PM)";

/// `+` or `-`, or the word for it ([`super::OPERATOR_WORD`]'s, as [`eval_date`]
/// sees the line before [`super::rewrite`]).
const SIGN: &str = r"[+-]|\b(?:plus|with|and|minus|subtract|without)\b";

/// A clock time on its own: today at that time.
static BARE_CLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)^(?:{CLOCK})$")).unwrap());

/// A clock time plus or minus a plain number: hours or minutes?
static CLOCK_PLUS_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)^(?:{CLOCK})\s*(?:{SIGN})\s*\d+(?:\.\d+)?$")).unwrap()
});

/// `35 days before 15 nov`: an amount of time before or after a day.
static DATE_BEFORE_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^(?P<n>\d+)\s*(?P<unit>{TIME_UNIT})\s+(?P<dir>before|after)\s+(?P<base>.+)$"
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
    Regex::new(&format!(
        r"(?i)^(?P<when>now|{CLOCK})(?:\s+(?P<from>[A-Za-z][A-Za-z_ /]*?))?\s+(?:in|to)\s+(?P<to>[A-Za-z][A-Za-z_ /]*)$"
    ))
    .unwrap()
});

static TIME_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<h>\d{1,2})(?::(?P<m>\d{2}))?\s*(?P<ampm>AM|PM)?$").unwrap());

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

/// Cities people write that the IANA database names after another city or
/// doesn't list (it has Ho Chi Minh, not Hanoi). Lowercase, spaces as
/// written.
const ZONE_CITIES: &[(&str, &str)] = &[
    ("hanoi", "Asia/Ho_Chi_Minh"),
    ("san francisco", "America/Los_Angeles"),
    ("washington", "America/New_York"),
    ("beijing", "Asia/Shanghai"),
    ("mumbai", "Asia/Kolkata"),
    ("delhi", "Asia/Kolkata"),
];

/// Answer a date or time-zone line (`today + 3 days`, `3PM PST in Tokyo`),
/// or `None` for anything else, which then goes to fend. `now` is the
/// recalculation's clock.
pub(crate) fn eval_date(expr: &str, now: DateTime<Local>) -> Option<Result<String, LineError>> {
    if DATE_WITHOUT_AT.is_match(expr) {
        return Some(Err(LineError::own(
            "write a date with @ in front, like @2026-12-25",
            "date needs @",
        )));
    }
    if let Some(converted) = eval_timezone_at(expr, now) {
        return Some(Ok(converted));
    }
    eval_date_at(expr.trim(), now.naive_local())
}

/// Resolve a zone name: an abbreviation (`PST`), an IANA name
/// (`Asia/Tokyo`) or a city (`Tokyo`, `new york`). `None` lets the line
/// fall through to fend, so `3PM in cm` still errors normally.
fn resolve_zone(name: &str) -> Option<Tz> {
    let name = name.trim();
    if let Some((_, iana)) = ZONE_ABBREVIATIONS
        .iter()
        .find(|(abbr, _)| abbr.eq_ignore_ascii_case(name))
    {
        return iana.parse().ok();
    }
    if let Some((_, iana)) = ZONE_CITIES
        .iter()
        .find(|(city, _)| city.eq_ignore_ascii_case(name))
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

/// The time of day `text` names, if it is a clock time ([`CLOCK`]) that
/// exists: `None` for `3` or `13PM`.
pub(super) fn clock_time(text: &str) -> Option<NaiveTime> {
    let text = text.trim();
    if !BARE_CLOCK.is_match(text) {
        return None;
    }
    parse_clock(&TIME_LITERAL.captures(text)?)
}

/// `12AM` is midnight and `12PM` noon; `13PM` or `0AM` is `None`.
fn parse_clock(caps: &regex::Captures) -> Option<NaiveTime> {
    let mut hour: u32 = caps["h"].parse().ok()?;
    let minute: u32 = caps.name("m").map_or(Ok(0), |m| m.as_str().parse()).ok()?;
    if let Some(ampm) = caps.name("ampm") {
        if !(1..=12).contains(&hour) {
            return None;
        }
        let is_pm = ampm.as_str() == "PM";
        hour %= 12;
        if is_pm {
            hour += 12;
        }
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// A [`BARE_DAY_OR_NOW`] word, [`DATE_OFFSET`] or [`DATE_BEFORE_AFTER`],
/// against a given clock.
fn eval_date_at(expr: &str, now: NaiveDateTime) -> Option<Result<String, LineError>> {
    if BARE_DAY_OR_NOW.is_match(expr) {
        return Some(if expr.eq_ignore_ascii_case("now") {
            Ok(format_datetime(now))
        } else {
            named_day(expr, now)
                .map(format_date)
                .ok_or_else(|| LineError::plain("date out of range"))
        });
    }
    if BARE_CLOCK.is_match(expr) {
        return Some(match clock_time(expr) {
            Some(time) => Ok(format_datetime(now.date().and_time(time))),
            None => Err(LineError::plain("not a time of day")),
        });
    }
    if CLOCK_PLUS_NUMBER.is_match(expr) {
        return Some(Err(LineError::own(
            "add a unit to a time, like 3PM + 2 hours",
            "needs a unit",
        )));
    }
    let (base, minus, n, unit) = if let Some(caps) = DATE_OFFSET.captures(expr) {
        (
            caps["base"].to_string(),
            matches!(
                caps["sign"].to_ascii_lowercase().as_str(),
                "-" | "minus" | "subtract" | "without"
            ),
            caps["n"].to_string(),
            caps["unit"].to_ascii_lowercase(),
        )
    } else {
        let caps = DATE_BEFORE_AFTER.captures(expr)?;
        (
            caps["base"].trim().to_string(),
            caps["dir"].eq_ignore_ascii_case("before"),
            caps["n"].to_string(),
            caps["unit"].to_ascii_lowercase(),
        )
    };
    let is_now = base.eq_ignore_ascii_case("now");
    let clock = clock_time(&base);
    // Shown with its time of day: `now` and a clock time keep theirs.
    let timed = is_now || clock.is_some();
    let unit = unit.trim_end_matches('s');
    if !timed && matches!(unit, "minute" | "min" | "hour") {
        return Some(Err(LineError::own(
            format!(
                "{} has no time of day; use now, like now + 2 hours",
                base.to_ascii_lowercase()
            ),
            "no time of day",
        )));
    }
    let start = if is_now {
        Some(now)
    } else if let Some(time) = clock {
        Some(now.date().and_time(time))
    } else if BARE_DAY_OR_NOW.is_match(&base) {
        named_day(&base, now).map(|day| day.and_time(NaiveTime::MIN))
    } else {
        match parse_day(&base, now.date()) {
            Some(day) => Some(day.and_time(NaiveTime::MIN)),
            None => {
                return Some(Err(LineError::own(
                    format!("'{base}' is not a date"),
                    "invalid date",
                )))
            }
        }
    };
    let Some(start) = start else {
        return Some(Err(LineError::plain("date out of range")));
    };
    let shifted = n
        .parse::<i64>()
        .ok()
        .map(|n| if minus { -n } else { n })
        .and_then(|n| shift(start, n, unit))
        // chrono prints year 10000 as `+10000` and year 1 BC as `0000`.
        .filter(|moment| (1..=9999).contains(&moment.year()));
    Some(match shifted {
        Some(moment) if timed => Ok(format_datetime(moment)),
        Some(moment) => Ok(format_date(moment.date())),
        None => Err(LineError::plain("date out of range")),
    })
}

/// A day written `@2026-11-15`, `15 nov` or `nov 15`, with or without a
/// year (`15 nov 2027`, `November 15, 2027`). Without one, it's `today`'s.
fn parse_day(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    if let Some(iso) = text.strip_prefix('@') {
        return NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok();
    }
    let text = text.replace(',', "");
    ["%d %b %Y", "%b %d %Y"].iter().find_map(|format| {
        NaiveDate::parse_from_str(&text, format)
            .or_else(|_| NaiveDate::parse_from_str(&format!("{text} {}", today.year()), format))
            .ok()
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

/// Like fend's own `@2026-12-25`, `Friday, 25 December 2026`, so every date
/// line reads the same.
fn format_date(d: NaiveDate) -> String {
    d.format("%A, %-d %B %Y").to_string()
}

fn format_datetime(dt: NaiveDateTime) -> String {
    dt.format("%A, %-d %B %Y %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`super::eval_date`] with its error as text, to compare with a message.
    fn eval_date(expr: &str, now: DateTime<Local>) -> Option<Result<String, String>> {
        super::eval_date(expr, now).map(|r| r.map_err(|e| e.to_string()))
    }

    /// [`super::eval_date_at`] with its error as text.
    fn eval_date_at(expr: &str, now: NaiveDateTime) -> Option<Result<String, String>> {
        super::eval_date_at(expr, now).map(|r| r.map_err(|e| e.to_string()))
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
        assert_eq!(eval_date_at("today", now), ok("Monday, 14 September 2026"));
        assert_eq!(
            eval_date_at("now", now),
            ok("Monday, 14 September 2026 10:30")
        );
        assert_eq!(eval_date_at("1 + 1", now), None);
    }

    /// A clock time is a time: alone it is today at that hour, and plus or
    /// minus minutes or hours it stays one, not picometres.
    #[test]
    fn eval_date_clock_times() {
        let now = at(2026, 9, 14, 10, 30);
        for (typed, shown) in [
            ("3PM", "Monday, 14 September 2026 15:00"),
            ("3:15PM", "Monday, 14 September 2026 15:15"),
            ("3:15 PM", "Monday, 14 September 2026 15:15"),
            ("15:00", "Monday, 14 September 2026 15:00"),
            ("10 AM", "Monday, 14 September 2026 10:00"),
            ("12AM", "Monday, 14 September 2026 00:00"),
            ("3PM + 2 hours", "Monday, 14 September 2026 17:00"),
            ("3PM plus 2 hours", "Monday, 14 September 2026 17:00"),
            ("3PM - 30 min", "Monday, 14 September 2026 14:30"),
            ("11PM + 2 hours", "Tuesday, 15 September 2026 01:00"),
            ("3PM + 1 day", "Tuesday, 15 September 2026 15:00"),
            ("2 hours before 3PM", "Monday, 14 September 2026 13:00"),
        ] {
            assert_eq!(eval_date_at(typed, now), ok(shown), "{typed}");
        }
    }

    /// Hours or minutes is a guess that could be silently wrong, and a time
    /// that doesn't exist is not one.
    #[test]
    fn eval_date_clock_time_errors() {
        let now = at(2026, 9, 14, 10, 30);
        for typed in ["3PM + 2", "3PM - 1.5", "3PM minus 10"] {
            assert_eq!(
                eval_date_at(typed, now),
                Some(Err("add a unit to a time, like 3PM + 2 hours".to_string())),
                "{typed}"
            );
        }
        for typed in ["25:00", "13PM", "0AM", "12:60"] {
            assert_eq!(
                eval_date_at(typed, now),
                Some(Err("not a time of day".to_string())),
                "{typed}"
            );
        }
    }

    /// Only the capitals are a time of day: lowercase `am` and `pm` are
    /// fend's attometre and picometre, so none of these is answered here.
    #[test]
    fn eval_date_reads_only_capital_am_and_pm() {
        let now = at(2026, 9, 14, 10, 30);
        for typed in [
            "3pm",
            "3:15pm",
            "10 am",
            "3pm + 2 hours",
            "3Pm",
            "3pM",
            "74 pm",
            "13 pm",
        ] {
            assert_eq!(eval_date_at(typed, now), None, "{typed}");
        }
        for typed in ["3pm PST in Tokyo", "9am to Tokyo", "74 pm in nm"] {
            assert_eq!(eval_timezone_at(typed, fixed_now()), None, "{typed}");
        }
    }

    /// Left to fend: a time that is a unit conversion or other arithmetic.
    #[test]
    fn eval_date_leaves_other_clock_lines_to_fend() {
        let now = at(2026, 9, 14, 10, 30);
        for typed in ["10AM in cm", "3PM + 5 mm", "3PM * 2", "3 + 2", "12:30:45"] {
            assert_eq!(eval_date_at(typed, now), None, "{typed}");
        }
    }

    #[test]
    fn eval_date_offsets() {
        let now = at(2026, 9, 14, 10, 30);
        assert_eq!(
            eval_date_at("today + 17 days", now),
            ok("Thursday, 1 October 2026")
        );
        assert_eq!(
            eval_date_at("today - 1 month", now),
            ok("Friday, 14 August 2026")
        );
        assert_eq!(
            eval_date_at("today + 1 year", now),
            ok("Tuesday, 14 September 2027")
        );
        assert_eq!(
            eval_date_at("now + 2 hours", now),
            ok("Monday, 14 September 2026 12:30")
        );
        assert_eq!(
            eval_date_at("now - 45 min", now),
            ok("Monday, 14 September 2026 09:45")
        );
        assert_eq!(
            eval_date_at("now + 1 day", now),
            ok("Tuesday, 15 September 2026 10:30")
        );
    }

    /// Every unit works on an `@` date, not only days.
    #[test]
    fn eval_date_offsets_from_an_at_date() {
        let now = at(2026, 9, 14, 10, 30);
        for (typed, shown) in [
            ("@2026-12-25 + 3 days", "Monday, 28 December 2026"),
            ("@2026-12-25 + 1 week", "Friday, 1 January 2027"),
            ("@2026-12-25 plus 1 month", "Monday, 25 January 2027"),
            ("@2024-02-29 + 1 year", "Friday, 28 February 2025"),
            ("@2026-12-25 - 3 days", "Tuesday, 22 December 2026"),
        ] {
            assert_eq!(eval_date_at(typed, now), ok(shown), "{typed}");
        }
        assert_eq!(
            eval_date_at("@2026-02-30 + 1 week", now),
            Some(Err("'@2026-02-30' is not a date".to_string()))
        );
    }

    /// The operator words work here as they do everywhere else, since
    /// `eval_date` sees the line before `rewrite` turns them into symbols.
    #[test]
    fn eval_date_offsets_in_operator_words() {
        let now = at(2026, 9, 14, 10, 30);
        assert_eq!(
            eval_date_at("today plus 3 days", now),
            ok("Thursday, 17 September 2026")
        );
        assert_eq!(
            eval_date_at("today with 3 days", now),
            ok("Thursday, 17 September 2026")
        );
        assert_eq!(
            eval_date_at("Today AND 3 days", now),
            ok("Thursday, 17 September 2026")
        );
        assert_eq!(
            eval_date_at("today minus 1 week", now),
            ok("Monday, 7 September 2026")
        );
        assert_eq!(
            eval_date_at("today subtract 1 week", now),
            ok("Monday, 7 September 2026")
        );
        assert_eq!(
            eval_date_at("today without 1 week", now),
            ok("Monday, 7 September 2026")
        );
        assert_eq!(
            eval_date_at("now plus 2 hours", now),
            ok("Monday, 14 September 2026 12:30")
        );
        // Only whole words.
        assert_eq!(eval_date_at("todayplus3 days", now), None);
    }

    #[test]
    fn eval_date_tomorrow_and_yesterday() {
        let now = at(2026, 12, 31, 23, 0);
        assert_eq!(eval_date_at("tomorrow", now), ok("Friday, 1 January 2027"));
        assert_eq!(
            eval_date_at("Yesterday", now),
            ok("Wednesday, 30 December 2026")
        );
        assert_eq!(
            eval_date_at("tomorrow + 1 week", now),
            ok("Friday, 8 January 2027")
        );
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

    /// The long form prints four digits of year; chrono would print the
    /// next day as `+10000` and the day before 1 January 0001 as `0000`.
    #[test]
    fn a_date_outside_years_1_to_9999_is_out_of_range() {
        let now = at(2026, 9, 14, 0, 0);
        let out_of_range = Some(Err("date out of range".to_string()));
        for expr in [
            "@9999-12-31 + 1 day",
            "@9999-12-31 + 1 year",
            "1 day after 31 dec 9999",
            "today + 7975 years",
            "now + 99999999 hours",
            "@0001-01-01 - 1 day",
            "1 day before 1 jan 0001",
            "@0001-01-01 - 1 year",
        ] {
            assert_eq!(eval_date_at(expr, now), out_of_range, "{expr}");
        }
        assert_eq!(
            eval_date_at("@9999-12-30 + 1 day", now),
            Some(Ok("Friday, 31 December 9999".to_string()))
        );
        assert_eq!(
            eval_date_at("@0001-01-02 - 1 day", now),
            Some(Ok("Monday, 1 January 0001".to_string()))
        );
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
        assert_eq!(resolve_zone("Hanoi"), Some(Tz::Asia__Ho_Chi_Minh));
        assert_eq!(
            resolve_zone("san francisco"),
            Some(Tz::America__Los_Angeles)
        );
        assert_eq!(resolve_zone("Mumbai"), Some(Tz::Asia__Kolkata));
        assert_eq!(resolve_zone("cm"), None);
        assert_eq!(resolve_zone("hours"), None);
    }

    #[test]
    fn eval_timezone_converts_between_zones() {
        // 2026-09-14 is in US and EU summer time.
        assert_eq!(
            eval_timezone_at("3PM PST in CET", fixed_now()).as_deref(),
            Some("Tuesday, 15 September 2026 00:00 CEST")
        );
        assert_eq!(
            eval_timezone_at("9AM PST to Tokyo", fixed_now()).as_deref(),
            Some("Tuesday, 15 September 2026 01:00 JST")
        );
    }

    #[test]
    fn gmt_is_utc_all_year_not_london() {
        assert_eq!(
            eval_timezone_at("12:00 GMT in UTC", fixed_now()).as_deref(),
            Some("Monday, 14 September 2026 12:00 UTC")
        );
        assert_eq!(
            eval_timezone_at("12:00 BST in UTC", fixed_now()).as_deref(),
            Some("Monday, 14 September 2026 11:00 UTC")
        );
    }

    #[test]
    fn eval_timezone_rejects_an_impossible_am_pm_hour() {
        assert_eq!(eval_timezone_at("13PM in UTC", fixed_now()), None);
        assert_eq!(eval_timezone_at("0AM in UTC", fixed_now()), None);
        assert!(eval_timezone_at("12AM in UTC", fixed_now()).is_some());
    }

    #[test]
    fn eval_timezone_now_in_utc() {
        let out = eval_timezone_at("now in UTC", fixed_now());
        assert_eq!(out.as_deref(), Some("Monday, 14 September 2026 12:00 UTC"));
    }

    #[test]
    fn eval_timezone_does_not_swallow_unit_conversions() {
        assert_eq!(eval_timezone_at("20 inches in cm", fixed_now()), None);
        assert_eq!(eval_timezone_at("3 apples in a box", fixed_now()), None);
        assert_eq!(eval_timezone_at("12 in cm", fixed_now()), None);
        assert_eq!(eval_date("20 inches in cm", fixed_now()), None);
    }

    #[test]
    fn eval_date_before_and_after() {
        let now = at(2026, 9, 14, 10, 30);
        assert_eq!(
            eval_date_at("35 days before 15 nov", now),
            ok("Sunday, 11 October 2026")
        );
        assert_eq!(
            eval_date_at("35 days after 15 nov", now),
            ok("Sunday, 20 December 2026")
        );
        assert_eq!(
            eval_date_at("1 week after 15 nov 2027", now),
            ok("Monday, 22 November 2027")
        );
        assert_eq!(
            eval_date_at("1 month before Nov 15, 2027", now),
            ok("Friday, 15 October 2027")
        );
        assert_eq!(
            eval_date_at("2 weeks after @2026-12-25", now),
            ok("Friday, 8 January 2027")
        );
        assert_eq!(
            eval_date_at("3 days before today", now),
            ok("Friday, 11 September 2026")
        );
        assert_eq!(
            eval_date_at("3 days after tomorrow", now),
            ok("Friday, 18 September 2026")
        );
        assert_eq!(
            eval_date_at("2 hours before now", now),
            ok("Monday, 14 September 2026 08:30")
        );
    }

    #[test]
    fn eval_date_before_and_after_name_a_non_day() {
        let now = at(2026, 9, 14, 10, 30);
        assert_eq!(
            eval_date_at("5 days before blah", now),
            Some(Err("'blah' is not a date".to_string()))
        );
        // 2026 has no 29 February.
        assert_eq!(
            eval_date_at("5 days before 29 feb", now),
            Some(Err("'29 feb' is not a date".to_string()))
        );
        let result = eval_date_at("5 hours before 15 nov", now);
        assert!(matches!(result, Some(Err(e)) if e.contains("no time of day")));
    }
}
