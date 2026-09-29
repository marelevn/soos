//! One line through fend-core, with the guards fend lacks, plus the units
//! Soos adds.

use std::cell::RefCell;
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Local};
use regex::Regex;

use crate::preprocess;

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
pub(crate) fn eval_line(ctx: &mut fend_core::Context, expr: &str) -> Result<Evaluated, String> {
    eval_line_at(ctx, expr, Local::now())
}

/// Evaluate one classified expression line. Assignments persist in `ctx`,
/// so `x = 5` on one line is visible to later lines.
pub(crate) fn eval_line_at(
    ctx: &mut fend_core::Context,
    expr: &str,
    now: DateTime<Local>,
) -> Result<Evaluated, String> {
    if let Some(date) = preprocess::eval_date(expr, now) {
        return date.map(|display| Evaluated {
            display,
            value: None,
            exact: true,
        });
    }
    let rewritten = preprocess::rewrite(expr);
    match expr_nesting_depth(&rewritten.expr) {
        Some(depth) if depth <= MAX_EXPR_NESTING_DEPTH => {}
        Some(_) => return Err("too nested".to_string()),
        None => return Err("too long".to_string()),
    }
    let deadline = Deadline::new(EVAL_TIMEOUT);
    if let Some(var) = &rewritten.percent_var {
        let held = fend_core::evaluate_with_interrupt(var, ctx, &deadline)?;
        if !held.get_main_result().ends_with('%') {
            return Err(format!(
                "'{var}' is not a percent; 'on' needs one, like fee = 8%"
            ));
        }
    }
    let r = fend_core::evaluate_with_interrupt(&rewritten.expr, ctx, &deadline)
        .map_err(explain_date_words)?;
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
    if rewritten.as_percent && !display.is_empty() {
        display.push('%');
    }
    Ok(Evaluated {
        display,
        value: Some(value),
        exact,
    })
}

/// A line ending in a conversion to another base, in fend's words for one
/// (fend-core's `ast.rs`) or with its `base N`.
static BASE_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?:in|to|as)\s+(?:(?P<word>bin|binary|ternary|senary|seximal|oct|octal|dec|decimal|hex|hexadecimal)",
        r"|base\s*\(?\s*(?P<n>\d{1,2})\s*\)?)\s*$",
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

/// A date word inside a larger expression reaches fend, which can't
/// read the clock, so say which forms work instead.
fn explain_date_words(error: String) -> String {
    match error.as_str() {
        "unknown identifier 'today'"
        | "unknown identifier 'tomorrow'"
        | "unknown identifier 'yesterday'"
        | "unknown identifier 'now'"
        | "unable to get the current date" => {
            "today, tomorrow and now only work alone, with + or - (now + 2 hours), or with a zone (now in Tokyo)"
                .to_string()
        }
        _ => error,
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
/// and the default root font size (16px). `pt`, `rem` and `ch` replace
/// fend's pint, roentgen-equivalent-man and chain; the README's calculation
/// rules say so.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(ctx: &mut fend_core::Context, expr: &str) -> Result<String, String> {
        eval_line(ctx, expr).map(|e| e.display)
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
        ] {
            assert_eq!(eval(&mut ctx, expr), Ok(shown.to_string()), "{expr}");
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
}
