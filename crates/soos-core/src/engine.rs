//! One line through fend-core, with the guards fend lacks, plus the units
//! Soos adds.

use std::cell::RefCell;
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Local};
use regex::Regex;

use crate::error::LineError;
use crate::preprocess::{self, CONVERT_KW};

mod guard;

pub(crate) use guard::{arguments as function_arguments, CALLS_ITSELF};

/// One successful line: what to show, and what `prev`/`sum`/`avg` may feed
/// back into fend. They differ where the display has parts fend can't read
/// back (`≈ 1.41`, `25%`).
pub(crate) struct Evaluated {
    pub display: String,
    /// `None` for a date or time: fend can't read its display (`Friday, 2
    /// October 2026`) back, so it can't be substituted into a later line.
    pub value: Option<String>,
    /// fend showed every digit; otherwise `value` is rounded (see
    /// [`remember_last`]).
    pub exact: bool,
}

/// Longest line handed to fend, in bytes. With [`MAX_EXPR_NESTING_DEPTH`]
/// this bounds fend's recursive parser and evaluator, which have no depth
/// guard: a stack overflow there ends the process with no error shown.
pub(crate) const MAX_EXPR_LEN: usize = 4096;
/// Deepest nesting fend may recurse through -- see [`expr_nesting_depth`].
/// [`crate::STACK_SIZE`] gives recursion up to this depth room to run.
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

thread_local! {
    /// This recalculation's converters (names and aliases) with the power of
    /// units each base holds -- see [`crate::document::define_converters`].
    /// fend shows a converter as `1 big`, so without this [`guard`] would
    /// count `big^8000` as 8,000 whatever `big` is made of.
    static CONVERTER_POWERS: RefCell<Vec<(String, u64)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn clear_converter_powers() {
    CONVERTER_POWERS.with_borrow_mut(Vec::clear);
}

pub(crate) fn add_converter_power(name: &str, power: u64) {
    CONVERTER_POWERS.with_borrow_mut(|powers| powers.push((name.to_string(), power)));
}

/// The power of units a converter's base holds, if `name` is a converter.
pub(crate) fn converter_power(name: &str) -> Option<u64> {
    CONVERTER_POWERS.with_borrow(|powers| {
        powers
            .iter()
            .find(|(n, _)| n == name)
            .map(|&(_, power)| power)
    })
}

/// The power of units in fend's display of a value, each name times its
/// exponent: `1 cm^5000 kg` is 5,001. `own` gives a name's power when it
/// isn't 1 (a converter's base).
pub(crate) fn units_shown(shown: &str, own: &dyn Fn(&str) -> Option<u64>) -> u64 {
    static TERM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)(?:\s*\^\s*-?(?P<e>\d+))?").unwrap()
    });
    let shown = shown
        .strip_prefix(crate::format::FEND_APPROX)
        .unwrap_or(shown);
    TERM.captures_iter(shown)
        .map(|caps| {
            let exponent = caps
                .name("e")
                .map_or(1, |e| e.as_str().parse().unwrap_or(u64::MAX));
            own(&caps["name"]).unwrap_or(1).saturating_mul(exponent)
        })
        .fold(0, u64::saturating_add)
}

/// Whether a newer edit cancelled this recalculation, or the watchdog gave
/// up on its thread. Checked between lines and, through [`Deadline`], inside
/// fend's long loops, so a cancelled line stops as quickly as a slow one
/// times out.
pub(crate) fn cancelled() -> bool {
    crate::watchdog::given_up()
        || CANCEL.with_borrow(|cancel| cancel.as_ref().is_some_and(|cancel| cancel()))
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
pub(crate) fn expr_nesting_depth(expr: &str) -> Option<usize> {
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
    let expr = preprocess::date_differences(expr, now);
    let expr = expr.as_ref();
    let (expr, scientific) = match SCI_TARGET.find(expr) {
        Some(target) => (&expr[..target.start()], true),
        None => (expr, false),
    };
    let rewritten = preprocess::rewrite(expr, &|name| holds_a_unit(ctx, name), &|left| {
        is_a_percent(ctx, left)
    });
    if STRAY_CLOCK.is_match(&rewritten.expr) {
        // `3PM in Atlantis`: the place is what's wrong, as with `now`.
        if let Some(unknown) = unknown_place(ctx, expr) {
            return Err(unknown);
        }
        return Err(LineError::own(
            "a time of day works alone, plus or minus minutes, hours, days, weeks, months or years, or minus another time",
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
            let retried = plain_terms_in_unit(ctx, &rewritten.expr, &error).and_then(|tagged| {
                fend_core::evaluate_with_interrupt(&tagged, ctx, &deadline).ok()
            });
            match retried {
                Some(r) => r,
                None => {
                    return Err(explain_percent_sum(&rewritten.expr, &error)
                        .or_else(|| explain_root(&rewritten.expr, &error))
                        .or_else(|| explain_lowercase_clock(&rewritten.expr, &error))
                        .unwrap_or_else(|| explain_date_words(error)))
                }
            }
        }
    };
    let raw = in_target_base(&rewritten.expr, r.get_main_result());
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
    Ok(Evaluated {
        display,
        value: Some(value),
        exact,
    })
}

/// fend's error for the place a time of day is converted to (`in Atlantis`),
/// when fend doesn't know it either; `None` when it does (`3PM in cm`).
fn unknown_place(ctx: &fend_core::Context, expr: &str) -> Option<LineError> {
    static PLACE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s(?:in|to)\s+(?P<place>[A-Za-z][A-Za-z_ /]*?)\s*$").unwrap()
    });
    let place = &PLACE.captures(expr)?["place"];
    let error =
        fend_core::evaluate_with_interrupt(place, &mut ctx.clone(), &Deadline::new(EVAL_TIMEOUT))
            .err()?;
    error
        .starts_with("unknown identifier")
        .then(|| LineError::from(error))
}

/// Whether `expr` holds a percent (`5%`, `fee` with `fee = 8%`), so a typed
/// percent after it adds to it (see [`preprocess::rewrite`]). Evaluated on a
/// copy of the context, once the guard has passed it.
fn is_a_percent(ctx: &fend_core::Context, expr: &str) -> bool {
    guard::check(ctx, expr).is_ok()
        && fend_core::evaluate_with_interrupt(expr, &mut ctx.clone(), &Deadline::new(EVAL_TIMEOUT))
            .is_ok_and(|r| r.get_main_result().ends_with('%'))
}

/// Whether `name` is a unit, or a variable holding a quantity, rather than
/// a plain number (`x = 10`, `pi`).
fn holds_a_unit(ctx: &fend_core::Context, name: &str) -> bool {
    fend_core::evaluate_with_interrupt(name, &mut ctx.clone(), &Deadline::new(EVAL_TIMEOUT))
        .map_or(true, |r| shows_a_unit(r.get_main_result()))
}

/// Whether fend's display of a value holds a unit: a letter outside its
/// `approx. ` prefix (`approx. 3.14` is a plain number).
pub(super) fn shows_a_unit(shown: &str) -> bool {
    shown
        .strip_prefix(crate::format::FEND_APPROX)
        .unwrap_or(shown)
        .chars()
        .any(char::is_alphabetic)
}

/// fend's refusal to add a plain number to a unit (`$5 + 1`, `5 m + 1`) names
/// the unit, either side. Reads the line again with each plain term (only
/// digits, operators and parentheses) of its sum written in that unit, as
/// `sum` counts the plain numbers of a block. A conversion on the end
/// (`$5 + 1 in EUR`) is left as it is, and so is a sum with no unit in it
/// (`3 + 4 in m` is still an error). `None` if the error is another one or
/// nothing was changed, so the caller reports fend's own message.
fn plain_terms_in_unit(ctx: &fend_core::Context, expr: &str, error: &str) -> Option<String> {
    let named = mismatched_unit(error)?;
    let unit = if named.contains(' ') {
        format!("({named})")
    } else {
        named.to_string()
    };
    // A bracket takes `unit` only when fend names the same one for it alone:
    // `(1 meter + 1)`, which fend calls `meters`, but not the `(2 + 3 cm)`
    // of `(5 m + 1) / (2 + 3 cm)`.
    let same_unit = |inside: &str| {
        fend_core::evaluate_with_interrupt(inside, &mut ctx.clone(), &Deadline::new(EVAL_TIMEOUT))
            .err()
            .is_some_and(|e| mismatched_unit(&e) == Some(named))
    };
    let body = guard::after_assignment(expr);
    let assignment = &expr[..expr.len() - body.len()];
    let (sum, conversion) = preprocess::split_conversion(body);
    tag_plain_terms(sum, &unit, &same_unit)
        .map(|tagged| format!("{assignment}{tagged}{conversion}"))
}

/// The unit in fend's refusal to add a plain number to it, either side.
fn mismatched_unit(error: &str) -> Option<&str> {
    static MISMATCH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^cannot convert from (?:unitless to (?P<to>[^:]+)|(?P<from>[^:]+) to unitless):",
        )
        .unwrap()
    });
    let caps = MISMATCH.captures(error)?;
    Some(
        caps.name("to")
            .or_else(|| caps.name("from"))?
            .as_str()
            .trim(),
    )
}

/// `term` with [`tag_plain_terms`] applied inside each of its outermost
/// brackets that `same_unit` accepts: `(5 m + 1)`, and `((100 m) + 5)` in
/// the `(15)% of ((100 m) + 5)` that `15% off 100 m + 5` becomes. `None` if
/// no bracket changed.
fn tag_groups(term: &str, unit: &str, same_unit: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut out = String::with_capacity(term.len() + 16);
    let (mut depth, mut open, mut copied) = (0usize, 0, 0);
    for (i, b) in term.bytes().enumerate() {
        match b {
            b'(' => {
                if depth == 0 {
                    open = i;
                }
                depth += 1;
            }
            b')' if depth > 0 => {
                depth -= 1;
                let inside = &term[open + 1..i];
                if depth == 0 && same_unit(inside) {
                    if let Some(inner) = tag_plain_terms(inside, unit, same_unit) {
                        out.push_str(&term[copied..=open]);
                        out.push_str(&inner);
                        copied = i;
                    }
                }
            }
            _ => {}
        }
    }
    (copied > 0).then(|| out + &term[copied..])
}

/// `expr` with the plain terms of its sum written in `unit`, looking inside
/// brackets too. `None` if no term was changed, or no term has a unit of
/// its own for the plain ones to join.
fn tag_plain_terms(expr: &str, unit: &str, same_unit: &dyn Fn(&str) -> bool) -> Option<String> {
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
        } else if let Some(inner) = tag_groups(term, unit, same_unit) {
            tagged = true;
            has_unit = true;
            out.push_str(&inner);
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

/// fend's `result` for `expr`, with the prefix of the base `expr` converts
/// to (see [`base_prefix`]), so it reads back as the same number.
fn in_target_base(expr: &str, result: &str) -> String {
    match base_prefix(expr) {
        Some(prefix) => with_base_prefix(result, &prefix),
        None => result.to_string(),
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

/// A percent beside an amount that isn't typed after it (`15% + $100`,
/// `fee = 8%` then `$100 - fee`) is fend's unit mismatch with `%` as one of
/// its two sides. The words for adding and taking off a percent are `on`
/// and `off`. A conversion to a percent (`5 EUR in %`) is a mismatch of its
/// own, so it keeps fend's.
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
            "use on or off, like 15% off $100 or fee on $100; a percent typed after the amount works too: $100 - 15%",
            "needs on or off",
        )
    })
}

/// [`register_root`]'s check that the index is a whole number.
fn explain_root(expr: &str, error: &str) -> Option<LineError> {
    static ROOT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(?:root|__soos_real_root)\b").unwrap());
    (error == NOT_A_WHOLE_INDEX && ROOT.is_match(expr)).then(|| {
        LineError::own(
            "root takes a whole number, like root 3 (27)",
            "needs a whole number",
        )
    })
}

/// A line with a lowercase time that fails the way one does: a zone after
/// it (`9am PST to Tokyo`), a `10:30pm`, or hours added to it. `am` and `pm`
/// are attometres and picometres, and a time of day takes the capitals. A
/// line that works as a length (`74 pm in nm`) never gets here, and one that
/// fails as one (`5 pm / 0`) keeps its own error.
fn explain_lowercase_clock(expr: &str, error: &str) -> Option<LineError> {
    static LOWERCASE_CLOCK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b\d{1,2}(?::\d{2})?\s*(?:am|pm)\b").unwrap());
    // An unknown name only counts after `in` or `to`, where a zone goes, so
    // a typo elsewhere (`5 pm + 2 metr`) keeps its own error.
    let zone = error
        .strip_prefix("unknown identifier '")
        .and_then(|rest| rest.strip_suffix('\''))
        .is_some_and(|name| {
            expr.contains(&format!("in {name}")) || expr.contains(&format!("to {name}"))
        });
    let as_a_time = zone || error == "expected an identifier" || error.contains("'second'");
    (as_a_time && LOWERCASE_CLOCK.is_match(expr)).then(|| {
        LineError::own(
            "a time of day takes a capital AM or PM, like 9AM; am and pm are attometres and picometres",
            "needs AM or PM",
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
            "before and after take a whole number of minutes, hours, days, weeks, months or years and a date, like 3 days before 15 nov",
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

/// A root's index `n`, checked to be a whole number: `abs(n) mod 2` is
/// fend's [`NOT_A_WHOLE_INDEX`] error for anything else, and adds 0 to a
/// whole one.
const WHOLE_INDEX: &str = "(n + 0 * (abs(n) mod 2))";
/// fend's error for a `mod` of a number that isn't whole.
const NOT_A_WHOLE_INDEX: &str = "modulo is only supported for positive integers";

/// `root 3 (27)`: fend has no nth root, but its functions curry, so
/// `root 3` is a function still waiting for the number. The power of a
/// negative number is complex, so [`preprocess::rewrite`] sends an odd root
/// to `__soos_real_root`: the real root of the positive part of `x`, less
/// that of its negative part, which needs no branch and is 0 at 0.
///
/// The index must be a whole number: `root (1/100000) (cm)` would be a power
/// [`guard`] can't see, which fend can't interrupt. fend's `mod` refuses
/// anything else, so [`WHOLE_INDEX`] checks it inside the function, where
/// `f = root` and `ans` reach it too.
pub(crate) fn register_root(ctx: &mut fend_core::Context) {
    let _ = fend_core::evaluate(&format!(r"root = \n.\x. x^(1/{WHOLE_INDEX})"), ctx);
    let _ = fend_core::evaluate(
        &format!(
            r"__soos_real_root = \n.\x. ((abs(x) + x)/2)^(1/{WHOLE_INDEX}) - ((abs(x) - x)/2)^(1/{WHOLE_INDEX})"
        ),
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

    /// An index below 1 would be a large power fend can't interrupt, however
    /// `root` is reached.
    #[test]
    fn a_root_takes_a_whole_number() {
        let mut ctx = fend_core::Context::new();
        register_root(&mut ctx);
        let start = std::time::Instant::now();
        for expr in ["root (1/100000) (cm) * kg", "root 0.5 (16)"] {
            let error = eval_line(&mut ctx, expr).err().map(|e| e.short());
            assert_eq!(error.as_deref(), Some("needs a whole number"), "{expr}");
        }
        assert!(eval(&mut ctx, "root 0 (4)").is_err());
        let mut alias = fend_core::Context::new();
        register_root(&mut alias);
        fend_core::evaluate("g = root", &mut alias).unwrap();
        assert!(eval(&mut alias, "g (1/100000) (cm) * kg").is_err());
        // Another modulo error is fend's own.
        let error = eval_line(&mut ctx, "rootless = 5.5 mod 2")
            .err()
            .map(|e| e.short());
        assert_ne!(error.as_deref(), Some("needs a whole number"));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    /// A lowercase time fails as a length, so the error says to use the
    /// capitals; a lowercase length still works.
    /// A time of day sent to a place fend doesn't know says so, as `now`
    /// does; anything else done to it lists what does work.
    #[test]
    fn a_time_of_day_in_an_unknown_place_names_the_place() {
        let mut ctx = fend_core::Context::new();
        for expr in ["3PM in Atlantis", "3PM PST in Atlantis"] {
            let error = eval_line(&mut ctx, expr).err().unwrap();
            assert_eq!(error.short(), "unknown Atlantis", "{expr}");
        }
        for expr in ["3PM in cm", "3PM + 30 s"] {
            let error = eval_line(&mut ctx, expr).err().unwrap();
            assert_eq!(error.short(), "unsupported time", "{expr}");
            assert!(
                error.to_string().contains("days, weeks, months or years"),
                "{expr}"
            );
        }
    }

    /// One day taken from another is the days between, which fend can use.
    #[test]
    fn a_day_minus_a_day_is_the_days_between() {
        use chrono::TimeZone;
        let now = Local.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
        let mut ctx = fend_core::Context::new();
        let shown = |ctx: &mut fend_core::Context, expr: &str| {
            eval_line_at(ctx, expr, now)
                .map(|e| e.display)
                .map_err(|e| e.short())
        };
        assert_eq!(
            shown(&mut ctx, "@2026-12-25 - @2026-12-01"),
            Ok("24 days".to_string())
        );
        assert_eq!(
            shown(&mut ctx, "@2026-12-01 - @2026-12-25"),
            Ok("-24 days".to_string())
        );
        assert_eq!(
            shown(&mut ctx, "@2026-12-25 minus today"),
            Ok("74 days".to_string())
        );
        assert_eq!(
            shown(&mut ctx, "tomorrow - yesterday"),
            Ok("2 days".to_string())
        );
        assert_eq!(
            shown(&mut ctx, "(@2026-12-29 - @2026-12-01) in weeks"),
            Ok("4 weeks".to_string())
        );
        assert!(shown(&mut ctx, "@2026-02-30 - @2026-02-01").is_err());
    }

    #[test]
    fn a_lowercase_time_that_fails_asks_for_capitals() {
        let mut ctx = fend_core::Context::new();
        for expr in ["9am PST to Tokyo", "3pm + 2 hours", "10:30pm in Hanoi"] {
            let error = eval_line(&mut ctx, expr).err().map(|e| e.short());
            assert_eq!(error.as_deref(), Some("needs AM or PM"), "{expr}");
        }
        assert_eq!(eval(&mut ctx, "74 pm in nm"), Ok("0.074 nm".to_string()));
        // A length that fails as a length keeps its own error.
        for (expr, short) in [
            ("5 pm / 0", "\u{f7} by 0"),
            ("2 pm + 1 kg", "unit mismatch"),
            ("5 pm + 2 metr", "unknown metr"),
        ] {
            let error = eval_line(&mut ctx, expr).err().map(|e| e.short());
            assert_eq!(error.as_deref(), Some(short), "{expr}");
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
        // The plain number may come first after `name =`, and a percent
        // phrase repeats its amount inside brackets.
        for (expr, shown) in [
            ("y = 1 + 5 m", "6 m"),
            ("cost = 20 + 5 m", "25 m"),
            ("15% off 100 m + 5", "89.25 m"),
            ("10% on 100 m + 5", "115.5 m"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        // A bracket with a unit of its own isn't given another.
        assert!(eval(&mut ctx, "(5 m + 1) / (2 + 3 cm)").is_err());
        // fend names a unit in its own spelling (`meters`), not as typed.
        for (expr, shown) in [
            ("(1 meter + 1) in cm", "200 cm"),
            ("(1 hour + 30) in minutes", "1860 minutes"),
            ("(1 foot + 1) in inches", "24 inches"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
    }

    /// A percent phrase in brackets ends at its bracket. One whose parts
    /// can't be read whole is an error, never a wrong number.
    #[test]
    fn a_percent_phrase_in_brackets_ends_there() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("(20% off 100) + 20", "100"),
            ("(5% on 100) + 10", "115"),
            ("(20% of what is 30) + 1", "151"),
            ("(5% on what is 105) * 2", "200"),
            ("20% off (5% on 100)", "84"),
            ("x = (20% off 100) + 20", "100"),
            ("20% off 100 + 20", "96"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        // `as a % of` in brackets is a fraction, like any percent in them.
        for (expr, shown) in [
            ("(10 as a % of 40)", "25%"),
            ("((10 as a % of 40))", "25%"),
            ("y = (10 as a % of 40)", "25%"),
            ("(10 as a % of 40) * 2", "0.5"),
            ("2 * (10 as a % of 40)", "0.5"),
            ("(10 as a % of 40) + 5", "5.25"),
            ("5 + (10 as a % of 40)", "5.25"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        // A whole line in brackets is a percent, as without them: a plain
        // number added to it keeps the percent, and a typed one adds.
        let _ = eval(&mut ctx, "y = (10 as a % of 40)");
        assert_eq!(eval(&mut ctx, "y + 1"), Ok("125%".to_string()));
        assert_eq!(eval(&mut ctx, "y + 5%"), Ok("30%".to_string()));
        // Chained and nested phrases are in `hostile_inputs_never_panic_or_hang`.
    }

    /// A quantity after a unit's `/` divides whole; after a name that holds
    /// a plain number it doesn't, as after a number.
    #[test]
    fn a_divisor_is_grouped_only_after_a_unit() {
        let mut ctx = fend_core::Context::new();
        let _ = eval(&mut ctx, "x = 10");
        let _ = eval(&mut ctx, "d = 6 m");
        for (expr, shown) in [
            ("6 m / 2 s", "3 m / s"),
            ("d / 2 s", "3 m / s"),
            ("x / 2 m", "5 m"),
            ("10 / 2 m", "5 m"),
            ("pi / 2 m", "\u{2248} 1.5707963268 m"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
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

        // A percent's value keeps its `%`, which fend reads back as 0.5.
        let ev = eval_line(&mut ctx, "50 as a % of 100").unwrap();
        assert_eq!(ev.display, "50%");
        assert_eq!(ev.value.as_deref(), Some("50%"));
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

    /// A typed percent after an amount is a percent of it. One before it, or
    /// in a variable, says to use `on` or `off`; a mismatch that is about
    /// something else, or a conversion to a percent, keeps fend's.
    #[test]
    fn a_percent_beside_an_amount_says_to_use_on_or_off() {
        let mut ctx = fend_core::Context::new();
        for (expr, shown) in [
            ("100 m - 15%", "85 m"),
            ("100 m + 15%", "115 m"),
            ("100 m - 15% in cm", "8500 cm"),
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
        }
        for expr in ["15% + 100 m", "15% + 100 m in cm"] {
            let err = eval_line(&mut ctx, expr).err().unwrap();
            assert_eq!(err.short(), "needs on or off", "{expr}");
            assert!(err.to_string().contains("on or off"), "{expr}");
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
            ("100 minus 15%", "85"),
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
