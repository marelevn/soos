//! One line through fend-core, with the guards fend lacks, plus the units
//! Soos adds.

use std::cell::RefCell;
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Local};
use regex::Regex;

use crate::error::LineError;
use crate::preprocess::{self, CONVERT_KW};

mod guard;

/// One successful line: what to show, and what `prev`/`sum`/`avg` may feed
/// back into fend. They differ where the display has parts fend can't read
/// back (`≈ 1.41`, `25%`).
pub(crate) struct Evaluated {
    pub display: String,
    /// `None` for a date or time: fend would read `2026-10-02` as
    /// subtraction, so it can't be substituted into a later line.
    pub value: Option<String>,
    /// fend showed every digit; otherwise `value` is rounded (see
    /// [`remember_last`]).
    pub exact: bool,
}

/// Longest line handed to fend, in bytes. With [`MAX_EXPR_NESTING_DEPTH`]
/// this bounds fend's recursive parser and evaluator, which have no depth
/// guard: a stack overflow there ends the process with no error shown.
const MAX_EXPR_LEN: usize = 4096;
/// Deepest nesting fend may recurse through -- see [`expr_nesting_depth`].
/// [`crate::on_big_stack`] gives recursion up to this depth room to run.
const MAX_EXPR_NESTING_DEPTH: usize = 64;

/// fend has no timeout, and a line like `9999999!` would otherwise freeze
/// the app -- again on every launch, since the document is saved.
const EVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

/// fend's own display of a date (`@2026-12-25` is "Friday, 25 December
/// 2026").
static FEND_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Mon|Tues|Wednes|Thurs|Fri|Satur|Sun)day, \d{1,2} [A-Z][a-z]+ -?\d+$").unwrap()
});

/// A wall-clock cutoff that fend polls inside its long-running loops.
struct Deadline(std::time::Instant);

impl Deadline {
    fn new(budget: std::time::Duration) -> Self {
        Self(std::time::Instant::now() + budget)
    }
}

impl fend_core::Interrupt for Deadline {
    fn should_interrupt(&self) -> bool {
        std::time::Instant::now() >= self.0 || cancelled()
    }
}

/// Whether the recalculation on this thread should stop.
pub(crate) type Cancel = Arc<dyn Fn() -> bool + Send + Sync>;

thread_local! {
    /// Set for a recalculation that may be cancelled -- see
    /// [`crate::recalc_document_unless`].
    static CANCEL: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}

pub(crate) fn set_cancel(cancel: Option<Cancel>) {
    CANCEL.with_borrow_mut(|slot| *slot = cancel);
}

/// Checked between lines and, through [`Deadline`], inside fend's long
/// loops, so a cancelled line stops as quickly as a slow one times out.
pub(crate) fn cancelled() -> bool {
    CANCEL.with_borrow(|cancel| cancel.as_ref().is_some_and(|cancel| cancel()))
}

/// Max nesting depth of `expr`, or `None` if it's longer than
/// [`MAX_EXPR_LEN`].
///
/// Parens alone aren't enough. In fend-core's `parser.rs`, `parse_power`
/// recurses once per prefix `-`/`+`/`/` and once per `^`, and a flat
/// `1+1+...+1` builds a left-leaning tree that the evaluator walks
/// recursively. So every operator counts against its paren level, and a
/// closing paren drops its level's count, since the closed group is a single
/// operand to what's around it.
fn expr_nesting_depth(expr: &str) -> Option<usize> {
    if expr.len() > MAX_EXPR_LEN {
        return None;
    }
    // One operator count per open paren level; [0] is the top level. An
    // unmatched `)` is ignored.
    let mut scope_ops: Vec<usize> = vec![0];
    let mut max_depth: usize = 0;
    for b in expr.bytes() {
        match b {
            b'(' => scope_ops.push(0),
            b')' if scope_ops.len() > 1 => {
                scope_ops.pop();
            }
            b'+' | b'-' | b'*' | b'/' | b'^' | b'!' | b'%' | b'&' | b'|' | b'<' | b'>' | b'=' => {
                *scope_ops.last_mut().unwrap() += 1;
            }
            _ => {}
        }
        let paren_depth = scope_ops.len() - 1;
        max_depth = max_depth.max(paren_depth + scope_ops.last().unwrap());
    }
    Some(max_depth)
}

/// [`eval_line_at`] against the current time.
pub(crate) fn eval_line(ctx: &mut fend_core::Context, expr: &str) -> Result<Evaluated, LineError> {
    eval_line_at(ctx, expr, Local::now())
}

/// Evaluate one classified expression line. Assignments persist in `ctx`,
/// so `x = 5` on one line is visible to later lines.
pub(crate) fn eval_line_at(
    ctx: &mut fend_core::Context,
    expr: &str,
    now: DateTime<Local>,
) -> Result<Evaluated, LineError> {
    if let Some(date) = preprocess::eval_date(expr, now) {
        return date.map(|display| Evaluated {
            display,
            value: None,
            exact: true,
        });
    }
    let (expr, scientific) = match SCI_TARGET.find(expr) {
        Some(target) => (&expr[..target.start()], true),
        None => (expr, false),
    };
    let rewritten = preprocess::rewrite(expr);
    if STRAY_CLOCK.is_match(&rewritten.expr) {
        return Err(LineError::own(
            "a time of day works alone, plus or minus minutes or hours, or minus another time",
            "unsupported time",
        ));
    }
    match expr_nesting_depth(&rewritten.expr) {
        Some(depth) if depth <= MAX_EXPR_NESTING_DEPTH => {}
        Some(_) => return Err(LineError::plain("too nested")),
        None => return Err(LineError::TooLong),
    }
    guard::check(ctx, &rewritten.expr)?;
    let deadline = Deadline::new(EVAL_TIMEOUT);
    if let Some(var) = &rewritten.percent_var {
        let held = fend_core::evaluate_with_interrupt(var, ctx, &deadline)?;
        if !held.get_main_result().ends_with('%') {
            return Err(LineError::own(
                format!("'{var}' is not a percent; 'on' needs one, like fee = 8%"),
                "not a percent",
            ));
        }
    }
    let r = match fend_core::evaluate_with_interrupt(&rewritten.expr, ctx, &deadline) {
        Ok(r) => r,
        Err(error) => {
            let retried = plain_terms_in_unit(&rewritten.expr, &error).and_then(|tagged| {
                fend_core::evaluate_with_interrupt(&tagged, ctx, &deadline).ok()
            });
            match retried {
                Some(r) => r,
                None => {
                    return Err(explain_percent_sum(&rewritten.expr, &error)
                        .unwrap_or_else(|| explain_date_words(error)))
                }
            }
        }
    };
    let raw = match base_prefix(&rewritten.expr) {
        Some(prefix) => with_base_prefix(r.get_main_result(), &prefix),
        None => r.get_main_result().to_string(),
    };
    let raw = raw.as_str();
    if FEND_DATE.is_match(raw) {
        return Ok(Evaluated {
            display: raw.to_string(),
            value: None,
            exact: true,
        });
    }
    let exact = !raw.starts_with(crate::format::FEND_APPROX);
    let value = raw
        .strip_prefix(crate::format::FEND_APPROX)
        .unwrap_or(raw)
        .to_string();
    let mut display = crate::format::approx_symbol(raw);
    if scientific {
        display = to_scientific(&display);
    }
    if rewritten.as_percent && !display.is_empty() {
        display.push('%');
    }
    Ok(Evaluated {
        display,
        value: Some(value),
        exact,
    })
}

/// fend's refusal to add a plain number to a unit (`$5 + 1`, `5 m + 1`) names
/// the unit, either side. Reads the line again with each plain term (only
/// digits, operators and parentheses) of its sum written in that unit, as
/// `sum` counts the plain numbers of a block. A conversion on the end
/// (`$5 + 1 in EUR`) is left as it is, and so is a sum with no unit in it
/// (`3 + 4 in m` is still an error). `None` if the error is another one or
/// nothing was changed, so the caller reports fend's own message.
fn plain_terms_in_unit(expr: &str, error: &str) -> Option<String> {
    static MISMATCH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^cannot convert from (?:unitless to (?P<to>[^:]+)|(?P<from>[^:]+) to unitless):",
        )
        .unwrap()
    });
    let caps = MISMATCH.captures(error)?;
    let unit = caps
        .name("to")
        .or_else(|| caps.name("from"))?
        .as_str()
        .trim();
    let unit = if unit.contains(' ') {
        format!("({unit})")
    } else {
        unit.to_string()
    };
    let (sum, conversion) = preprocess::split_conversion(expr);
    tag_plain_terms(sum, &unit).map(|tagged| format!("{tagged}{conversion}"))
}

/// What is inside `term` when it is one parenthesised group: `5 m + 1` in
/// `(5 m + 1)`, but not in `(1) + (2)`.
fn group_inside(term: &str) -> Option<&str> {
    let inner = term.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0usize;
    for b in inner.bytes() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    (depth == 0).then_some(inner)
}

/// `expr` with the plain terms of its sum written in `unit`, looking inside
/// a parenthesised term too. `None` if no term was changed, or no term has a
/// unit of its own for the plain ones to join.
fn tag_plain_terms(expr: &str, unit: &str) -> Option<String> {
    // The `+` and `-` that join terms: not a sign (`2 * -1`, a leading `-`),
    // not an exponent's (`1e-5`), and not inside parentheses.
    let bytes = expr.as_bytes();
    let mut bounds = vec![0];
    let mut depth = 0usize;
    let mut prev: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'+' | b'-' if depth == 0 => {
                let exponent = i >= 2
                    && matches!(bytes[i - 1], b'e' | b'E')
                    && (bytes[i - 2].is_ascii_digit() || bytes[i - 2] == b'.');
                if prev.is_some_and(|p| !b"+-*/^(=<>,".contains(&p)) && !exponent {
                    bounds.push(i);
                }
            }
            _ => {}
        }
        if !b.is_ascii_whitespace() {
            prev = Some(b);
        }
    }
    bounds.push(expr.len());

    let is_plain = |term: &str| {
        let term = term.trim().trim_start_matches(['-', '+']);
        term.bytes().any(|b| b.is_ascii_digit())
            && term
                .bytes()
                .all(|b| b.is_ascii_digit() || b" .,*/^()+-eE".contains(&b))
    };
    let mut out = String::with_capacity(expr.len() + 16);
    let mut tagged = false;
    let mut has_unit = false;
    for (n, bound) in bounds.windows(2).enumerate() {
        let segment = &expr[bound[0]..bound[1]];
        let (op, term) = if n == 0 {
            ("", segment)
        } else {
            segment.split_at(1)
        };
        out.push_str(op);
        if is_plain(term) {
            tagged = true;
            out.push_str(&format!(" ({}) {unit}", term.trim()));
        } else if let Some(inner) = group_inside(term).and_then(|g| tag_plain_terms(g, unit)) {
            tagged = true;
            has_unit = true;
            out.push_str(&format!(" ({inner})"));
        } else {
            has_unit = true;
            out.push_str(term);
        }
    }
    (tagged && has_unit).then_some(out)
}

/// A time of day that [`preprocess`] didn't answer or turn into a duration
/// (`3PM * 2`). fend has no `AM` or `PM`, so without this the line would say
/// `unknown PM`. Lowercase `am` and `pm` are fend's attometre and picometre,
/// and pass through as lengths.
static STRAY_CLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+(?:\.\d+)?\s*(?:AM|PM)\b").unwrap());

/// A trailing `in sci`, which fend has no word for. Cut off before the
/// line is rewritten, so a percent phrase doesn't take it into its operand.
static SCI_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\s+(?:{CONVERT_KW})\s+(?:sci|scientific)\s*$"
    ))
    .unwrap()
});

/// The first number in `display` as a mantissa and a power of ten
/// (`5300 m` is `5.3e3 m`). It works on the digits, so nothing is rounded,
/// and fend reads the result back.
fn to_scientific(display: &str) -> String {
    static NUMBER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?P<int>[0-9]+)(?:\.(?P<frac>[0-9]+))?").unwrap());
    let Some(caps) = NUMBER.captures(display) else {
        return display.to_string();
    };
    let int = &caps["int"];
    let digits = format!("{int}{}", caps.name("frac").map_or("", |m| m.as_str()));
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return display.to_string();
    }
    let exponent = int.len() as i64 - (digits.len() - significant.len()) as i64 - 1;
    let (first, rest) = significant.trim_end_matches('0').split_at(1);
    let mantissa = if rest.is_empty() {
        first.to_string()
    } else {
        format!("{first}.{rest}")
    };
    let number = caps.get(0).unwrap();
    format!(
        "{}{mantissa}e{exponent}{}",
        &display[..number.start()],
        &display[number.end()..]
    )
}

/// A line ending in a conversion to another base, in fend's words for one
/// (fend-core's `ast.rs`) or with its `base N`.
static BASE_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        concat!(
            r"(?i)\b(?:{kw})\s+(?:(?P<word>bin|binary|ternary|senary|seximal|oct|octal|dec|decimal|hex|hexadecimal)",
            r"|base\s*\(?\s*(?P<n>\d{{1,2}})\s*\)?)\s*$",
        ),
        kw = CONVERT_KW,
    ))
    .unwrap()
});

/// The prefix fend writes on a number in `expr`'s target base: `0b`, `0o`,
/// `0x`, or `N#`; `None` for decimal or no conversion. fend prints a
/// converted number bare (`255 in binary` is `11111111`), which reads, groups
/// and feeds `prev` as eleven million; with the prefix it reads back as the
/// same number.
fn base_prefix(expr: &str) -> Option<String> {
    let caps = BASE_TARGET.captures(expr)?;
    let base: u8 = match caps.name("word") {
        Some(word) => match word.as_str().to_ascii_lowercase().as_str() {
            "bin" | "binary" => 2,
            "ternary" => 3,
            "senary" | "seximal" => 6,
            "oct" | "octal" => 8,
            "hex" | "hexadecimal" => 16,
            _ => 10,
        },
        None => caps["n"].parse().ok()?,
    };
    match base {
        2 => Some("0b".to_string()),
        8 => Some("0o".to_string()),
        16 => Some("0x".to_string()),
        10 => None,
        n => Some(format!("{n}#")),
    }
}

/// `prefix` on the number fend's result starts with, after its `approx. `
/// and a minus sign (`approx. -0b0.1`). Anything else -- a result already
/// prefixed, or one that isn't a number -- is returned as it is.
fn with_base_prefix(result: &str, prefix: &str) -> String {
    let (approx, rest) = match result.strip_prefix(crate::format::FEND_APPROX) {
        Some(rest) => (crate::format::FEND_APPROX, rest),
        None => ("", result),
    };
    let (sign, digits) = match rest.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", rest),
    };
    let prefixed = ["0b", "0o", "0x"].iter().any(|p| digits.starts_with(p))
        || digits
            .split_once('#')
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if prefixed || !digits.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return result.to_string();
    }
    format!("{approx}{sign}{prefix}{digits}")
}

/// Save fend's last result as the variable `name`. fend keeps it in `_` at
/// full precision, so a later line can use an inexact result without the
/// rounding of its display.
pub(crate) fn remember_last(ctx: &mut fend_core::Context, name: &str) -> bool {
    let deadline = Deadline::new(EVAL_TIMEOUT);
    fend_core::evaluate_with_interrupt(&format!("{name} = _"), ctx, &deadline).is_ok()
}

/// A percent beside an amount (`$100 - 15%`, `fee = 8%` then `$100 - fee`) is
/// fend's unit mismatch with `%` as one of its two sides. The words for
/// adding and taking off a percent are `on` and `off`. A conversion to a
/// percent (`5 EUR in %`) is a mismatch of its own, so it keeps fend's.
fn explain_percent_sum(expr: &str, error: &str) -> Option<LineError> {
    static PERCENT_SIDE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^cannot convert from (?:% to [^:]+|[^:]+ to %):").unwrap());
    static CONVERT_TO_PERCENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"\b(?:{CONVERT_KW})\s+%")).unwrap());
    let beside_an_amount = PERCENT_SIDE.is_match(error)
        && expr.contains(['+', '-'])
        && !CONVERT_TO_PERCENT.is_match(expr);
    beside_an_amount.then(|| {
        LineError::own(
            "a percent can't be added to or taken from an amount; use on or off, like 15% off $100",
            "needs on or off",
        )
    })
}

/// A date word inside a larger expression reaches fend, which can't read
/// the clock, and a `before` or `after` that isn't `3 days before 15 nov`
/// reaches it as a name. Say which forms work instead.
fn explain_date_words(error: String) -> LineError {
    match error.as_str() {
        "unknown identifier 'today'"
        | "unknown identifier 'tomorrow'"
        | "unknown identifier 'yesterday'"
        | "unknown identifier 'now'"
        | "unable to get the current date" => LineError::own(
            "today, tomorrow and now only work alone, plus or minus a whole number of minutes, hours, days, weeks, months or years (now + 2 hours), or with a zone (now in Tokyo)",
            "unsupported date",
        ),
        "unknown identifier 'before'" | "unknown identifier 'after'" => LineError::own(
            "before and after take a whole number of days and a date, like 3 days before 15 nov",
            "needs a date",
        ),
        _ => LineError::Fend(error),
    }
}

/// Register a unit as `definition` (a fend expression such as
/// `"0.3048 m"`), singular and plural, with no SI prefixes.
pub(crate) fn define_unit(ctx: &mut fend_core::Context, name: &str, definition: &str) {
    ctx.define_custom_unit_v1(
        name,
        name,
        definition,
        &fend_core::CustomUnitAttribute::None,
    );
}

/// CSS and typography units, from the CSS reference pixel (1px = 1/96 in)
/// and the default root font size (16px). `pt`, `pc`, `rem` and `ch` replace
/// fend's pint, parsec, roentgen equivalent man and chain; `pint`, `parsec`
/// and `chain` still name three of them.
const CSS_UNITS: &[(&str, &str)] = &[
    ("px", "1/96 inch"),
    ("pt", "1/72 inch"),
    ("pc", "12 pt"),
    ("rem", "16 px"),
    ("em", "16 px"),
    ("ch", "8 px"),
];

/// Registered before any converter, so converters can't take these names.
pub(crate) fn register_builtin_units(ctx: &mut fend_core::Context) {
    for (name, def) in CSS_UNITS {
        define_unit(ctx, name, def);
    }
}

/// `root 3 (27)`: fend has no nth root, but its functions curry, so
/// `root 3` is a function still waiting for the number. The power of a
/// negative number is complex, so [`preprocess::rewrite`] sends an odd root
/// to `__soos_real_root`: the real root of the positive part of `x`, less
/// that of its negative part, which needs no branch and is 0 at 0.
pub(crate) fn register_root(ctx: &mut fend_core::Context) {
    let _ = fend_core::evaluate(r"root = \n.\x. x^(1/n)", ctx);
    let _ = fend_core::evaluate(
        r"__soos_real_root = \n.\x. ((abs(x) + x)/2)^(1/n) - ((abs(x) - x)/2)^(1/n)",
        ctx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(ctx: &mut fend_core::Context, expr: &str) -> Result<String, String> {
        eval_line(ctx, expr)
            .map(|e| e.display)
            .map_err(|e| e.to_string())
    }

    /// fend can't be interrupted inside a long shift, a die or a unit's big
    /// power, so they are refused before it runs -- see [`guard`].
    #[test]
    fn lines_fend_cannot_interrupt_are_refused_before_fend_runs() {
        let mut ctx = fend_core::Context::new();
        for (expr, message) in [
            ("4 << 100000000", guard::SHIFT_TOO_LARGE),
            ("cm^400000 kg", guard::POWER_TOO_LARGE),
            ("d1000 + d1000", "dice like 4d6 aren't supported"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Err(message.to_string()), "{expr}");
        }
        assert_eq!(eval(&mut ctx, "4 << 5"), Ok("128".to_string()));
    }

    /// Lines fend reads differently from how people mean them.
    #[test]
    fn natural_readings_of_units_and_percents() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("sin 30 deg", "0.5"),
            ("cos 60\u{b0}", "0.5"),
            ("100 * 15%", "15"),
            ("100 / 20%", "500"),
            ("50% * 50%", "25%"),
            ("5 ft 11 in in cm", "180.34 cm"),
            ("3 in + 2 in", "5 inches"),
            ("1 ft in in", "12 inches"),
            ("2.54 cm to in", "1 inch"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
    }

    /// An odd root of a negative number is real, where the power is complex.
    #[test]
    fn odd_roots_of_negative_numbers_are_real() {
        let mut ctx = fend_core::Context::new();
        register_root(&mut ctx);
        for (expr, shown) in [
            ("root 3 (-8)", "-2"),
            ("cbrt(-8)", "-2"),
            ("cbrt 27", "3"),
            ("root 5 (-32)", "-2"),
            ("root 3 (0)", "0"),
            ("root 3 (-27 m^3)", "-3 m"),
            ("root 2 (16)", "4"),
            ("root 4 (16)", "2"),
            ("sqrt(-4)", "\u{2248} 0 + 2i"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
    }

    /// A time is written `AM` or `PM`, so one that isn't a plain time of day
    /// or a difference must not reach fend, which has no such unit.
    #[test]
    fn a_time_of_day_never_becomes_a_length() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("(3PM - 10AM) * 2", "10 hours"),
            ("3PM - 10AM + 1 hour", "6 hours"),
            ("3PM - 10AM in minutes", "300 minutes"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        for expr in ["3PM * 2", "3PM + 10AM", "3.5PM", "5PM in m", "t3 = 3PM"] {
            let err = eval(&mut ctx, expr).unwrap_err();
            assert!(err.contains("a time of day works alone"), "{expr}: {err}");
        }
    }

    /// Lowercase `am` and `pm` are fend's attometre and picometre, in any
    /// line: only the capitals `AM` and `PM` are a time of day.
    #[test]
    fn lowercase_am_and_pm_are_lengths() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("74 pm in nm", "0.074 nm"),
            ("1000 pm to nm", "1 nm"),
            ("3pm * 2", "6 pm"),
            ("5 pm in m", "0.000000000005 m"),
            ("1 Pm to km", "1000000000000 km"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
    }

    /// A plain number added to a unit takes that unit, as in a `sum` block.
    #[test]
    fn a_plain_number_joins_a_unit_it_is_added_to() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("5 m + 1", "6 m"),
            ("1 + 5 m", "6 m"),
            ("5 m - 1", "4 m"),
            ("5 m * 2 + 1", "11 m"),
            ("5 m + 1 * 2", "7 m"),
            ("(1 + 1) + 5 m", "7 m"),
            ("5 m + 1e-3", "5.001 m"),
            // With a conversion on the end, or inside parentheses.
            ("5 m + 1 in cm", "600 cm"),
            ("1 + 5 m to cm", "600 cm"),
            ("(5 m + 1) in cm", "600 cm"),
            ("((5 m) + (1)) in cm", "600 cm"),
            // A unit with spaces in it still reads back (fend names it newtons).
            ("5 kg m / s^2 + 1", "6 newtons"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        let _ = eval(&mut ctx, "rent = 1800 m");
        assert_eq!(eval(&mut ctx, "rent + 50"), Ok("1850 m".to_string()));
        // The assignment keeps the sum, not the failed first reading.
        let _ = eval(&mut ctx, "x = 5 m + 1");
        assert_eq!(eval(&mut ctx, "x * 2"), Ok("12 m".to_string()));
    }

    /// Two different units, or a conversion, are still the error fend gives.
    #[test]
    fn a_plain_number_does_not_hide_a_real_mismatch() {
        let mut ctx = fend_core::Context::new();
        for expr in ["5 m + 1 kg", "5 in m", "3 + 4 in m", "5 m + 1 + 2 kg"] {
            let err = eval(&mut ctx, expr).unwrap_err();
            assert!(err.starts_with("cannot convert from"), "{expr}: {err}");
        }
    }

    /// A number converted to another base carries fend's prefix for it, so
    /// it isn't read, grouped or summed as a decimal.
    #[test]
    fn base_conversions_carry_their_prefix() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("255 in binary", "0b11111111"),
            ("4095 to octal", "0o7777"),
            ("255 in hex", "0xff"),
            ("255 in base 3", "3#100110"),
            ("-5 in binary", "-0b101"),
            ("0xff in decimal", "255"),
            ("0b1010 + 1", "0b1011"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        let ev = eval_line(&mut ctx, "1/3 in binary").unwrap();
        assert_eq!(ev.display, "\u{2248} 0b0.0101010101");
        assert_eq!(ev.value.as_deref(), Some("0b0.0101010101"));
    }

    #[test]
    fn basic_arithmetic() {
        let mut ctx = fend_core::Context::new();
        assert_eq!(eval(&mut ctx, "1 + 1"), Ok("2".to_string()));
    }

    #[test]
    fn variables_persist_across_calls_on_same_context() {
        let mut ctx = fend_core::Context::new();
        let _ = eval(&mut ctx, "x = 5");
        assert_eq!(eval(&mut ctx, "x + 1"), Ok("6".to_string()));
    }

    #[test]
    fn unit_conversion() {
        let mut ctx = fend_core::Context::new();
        assert_eq!(eval(&mut ctx, "20 inches in cm"), Ok("50.8 cm".to_string()));
    }

    #[test]
    fn error_surfaces() {
        let mut ctx = fend_core::Context::new();
        assert!(eval(&mut ctx, "1 +").is_err());
    }

    #[test]
    fn custom_unit_registers() {
        let mut ctx = fend_core::Context::new();
        define_unit(&mut ctx, "horse", "2.4 m");
        assert_eq!(eval(&mut ctx, "1 horse to m"), Ok("2.4 m".to_string()));
    }

    #[test]
    fn css_units_register_and_shadow_builtins() {
        let mut ctx = fend_core::Context::new();
        register_builtin_units(&mut ctx);
        assert_eq!(eval(&mut ctx, "16 px to pt"), Ok("12 pt".to_string()));
        assert_eq!(eval(&mut ctx, "1 em to px"), Ok("16 px".to_string()));
        assert_eq!(eval(&mut ctx, "72 pt to inches"), Ok("1 inch".to_string()));
        assert!(eval(&mut ctx, "1 pint to ml").is_ok());
    }

    #[test]
    fn dates_have_no_substitution_value() {
        let mut ctx = fend_core::Context::new();
        assert!(eval_line(&mut ctx, "today").unwrap().value.is_none());
        let fend_date = eval_line(&mut ctx, "@2026-12-25").unwrap();
        assert_eq!(fend_date.display, "Friday, 25 December 2026");
        assert!(fend_date.value.is_none());
    }

    #[test]
    fn today_inside_a_larger_expression_explains_itself() {
        let mut ctx = fend_core::Context::new();
        for expr in ["now + 2 hours + 5", "today * 2", "(today)"] {
            let err = eval(&mut ctx, expr).unwrap_err();
            assert!(err.contains("now + 2 hours"), "{expr}: {err}");
        }
    }

    #[test]
    fn approx_and_percent_values_are_fend_parseable() {
        let mut ctx = fend_core::Context::new();
        let ev = eval_line(&mut ctx, "sqrt(2)").unwrap();
        assert_eq!(ev.display, "\u{2248} 1.4142135624");
        assert_eq!(ev.value.as_deref(), Some("1.4142135624"));

        // fend reads "50%" back as 0.5, not 50.
        let ev = eval_line(&mut ctx, "50 as a % of 100").unwrap();
        assert_eq!(ev.display, "50%");
        assert_eq!(ev.value.as_deref(), Some("50"));
    }

    #[test]
    fn var_on_needs_a_percent_variable() {
        let mut ctx = fend_core::Context::new();
        let _ = eval(&mut ctx, "fee = 8%");
        let _ = eval(&mut ctx, "x = 2");
        assert_eq!(eval(&mut ctx, "fee on 200"), Ok("216".to_string()));
        let err = eval(&mut ctx, "x on 10").unwrap_err();
        assert!(err.contains("not a percent"), "{err}");
        assert!(eval(&mut ctx, "nope on 10")
            .unwrap_err()
            .starts_with("unknown identifier"));
    }

    #[test]
    fn expression_too_deep_rejects_overdeep_parens() {
        let mut ctx = fend_core::Context::new();
        let deep: String =
            "(".repeat(MAX_EXPR_NESTING_DEPTH + 1) + "1" + &")".repeat(MAX_EXPR_NESTING_DEPTH + 1);
        assert_eq!(eval(&mut ctx, &deep), Err("too nested".to_string()));
    }

    /// A percent beside an amount says to use `on` or `off`; a mismatch that
    /// is about something else, or a conversion to a percent, keeps fend's.
    #[test]
    fn a_percent_beside_an_amount_says_to_use_on_or_off() {
        let mut ctx = fend_core::Context::new();
        for expr in [
            "100 m - 15%",
            "100 m + 15%",
            "15% + 100 m",
            "100 m - 15% in cm",
        ] {
            let err = eval_line(&mut ctx, expr).err().unwrap();
            assert_eq!(err.short(), "needs on or off", "{expr}");
            assert!(err.to_string().contains("use on or off"), "{expr}");
        }
        let _ = eval(&mut ctx, "fee = 8%");
        assert_eq!(
            eval_line(&mut ctx, "100 m - fee").err().unwrap().short(),
            "needs on or off"
        );
        for expr in ["5 m + 3 kg - 10%", "5 m in %"] {
            let err = eval_line(&mut ctx, expr).err().unwrap();
            assert_eq!(err.short(), "unit mismatch", "{expr}");
        }
        assert_eq!(eval(&mut ctx, "2 * (1 - 5%)"), Ok("1.9".to_string()));
    }

    /// Recursion at the full cap doesn't fit a test thread's default stack,
    /// so this runs on the same big stack real callers use.
    #[test]
    fn expression_at_depth_cap_still_evaluates() {
        let at_cap: String =
            "(".repeat(MAX_EXPR_NESTING_DEPTH) + "1" + &")".repeat(MAX_EXPR_NESTING_DEPTH);
        let result = crate::on_big_stack(move || {
            let mut ctx = fend_core::Context::new();
            eval(&mut ctx, &at_cap)
        });
        assert!(result.is_ok());
    }

    #[test]
    fn expression_too_long_rejects_oversized_input() {
        let mut ctx = fend_core::Context::new();
        let expr = "1+".repeat(MAX_EXPR_LEN / 2 + 1);
        assert!(expr.len() > MAX_EXPR_LEN);
        assert_eq!(eval(&mut ctx, &expr), Err("too long".to_string()));
    }

    #[test]
    fn slow_expression_is_interrupted_not_left_to_run() {
        let mut ctx = fend_core::Context::new();
        let start = std::time::Instant::now();
        assert!(eval(&mut ctx, "9999999!").is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn expression_unbalanced_parens_is_not_mislabeled_too_deep() {
        let mut ctx = fend_core::Context::new();
        assert_eq!(eval(&mut ctx, "1))"), Ok("1".to_string()));
    }

    #[test]
    fn expression_too_deep_counts_operators_not_just_parens() {
        let mut ctx = fend_core::Context::new();
        for deep in [
            "-".repeat(3000) + "1",
            "1".to_string() + &"!".repeat(3000),
            "- ".repeat(100) + "1",
            "1+".repeat(200) + "1",
        ] {
            assert!(deep.len() < MAX_EXPR_LEN);
            assert_eq!(eval(&mut ctx, &deep), Err("too nested".to_string()));
        }
    }

    #[test]
    fn expression_nesting_depth_resets_after_a_closed_paren() {
        let inner = format!("({})", "1+".repeat(60) + "1");
        let expr = format!("{inner}+{inner}");
        let result = crate::on_big_stack(move || {
            let mut ctx = fend_core::Context::new();
            eval(&mut ctx, &expr)
        });
        assert!(result.is_ok());
    }

    /// Phrasing fend alone doesn't read, one line per rewrite in
    /// `preprocess`.
    #[test]
    fn natural_phrasing_reads_as_people_write_it() {
        let mut ctx = fend_core::Context::new();
        register_root(&mut ctx);
        for (expr, shown) in [
            ("4 plus 4", "8"),
            ("4 with 4", "8"),
            ("4 and 4", "8"),
            ("4 & 6", "4"),
            ("4 minus 4", "0"),
            ("4 subtract 4", "0"),
            ("4 without 4", "0"),
            ("4 times 4", "16"),
            ("4 multiplied by 4", "16"),
            ("4 mul 4", "16"),
            ("4 divide 4", "1"),
            ("4 divide by 4", "1"),
            ("4 divided by 4", "1"),
            ("100 minus 15%", "99.85"),
            ("arcsin(1)", "\u{2248} 1.5707963268"),
            ("root 3 (27)", "3"),
            ("20% of what is 30 cm", "150 cm"),
            ("20% on what is 30 cm", "25 cm"),
            ("20% off what is 30 cm", "37.5 cm"),
            ("20 sq cm", "20 cm^2"),
            ("20 cu cm", "20 cm^3"),
            ("100 m^2 / 5 sq m", "20"),
            ("100 m^3 / 2 cu m", "50"),
            ("5 300 + 1", "5301"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
    }

    #[test]
    fn in_sci_shows_a_mantissa_and_a_power_of_ten() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("5 300 in sci", "5.3e3"),
            ("5300 m to scientific", "5.3e3 m"),
            ("100 as sci", "1e2"),
            ("0.0053 in sci", "5.3e-3"),
            ("-12345.6789 in sci", "-1.23456789e4"),
            ("5% on 30 in sci", "3.15e1"),
            ("0 in sci", "0"),
            ("1/3 in sci", "\u{2248} 3.333333333e-1"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        // The shown result reads back as the same number, and `prev` still
        // gets the plain one.
        assert_eq!(eval(&mut ctx, "5.3e3 + 1"), Ok("5301".to_string()));
        let ev = eval_line(&mut ctx, "5300 in sci").unwrap();
        assert_eq!(ev.value.as_deref(), Some("5300"));
    }
}
