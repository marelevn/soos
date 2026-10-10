//! The document model: lines are evaluated top to bottom, and `prev`,
//! `sum`/`total` and `avg`/`average` reach across lines by substituting
//! literal values before a line reaches fend, which only sees one line.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, Local};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::engine;
use crate::error::LineError;
use crate::highlight::{CONVERSION_WORD, DATE_WORD};
use crate::preprocess::{self, INTO_WORD, OPERATOR_WORD};

/// One document line's outcome -- see [`recalc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineResult {
    /// Empty, or a `//` comment.
    Blank,
    /// `# heading`.
    Header,
    /// `Label:` on its own.
    Label,
    /// fend's display text, before [`crate::format::shown`] adds currency
    /// symbols and separators.
    Value(String),
    /// A date or time, shown as is and left out of `prev`/`sum`/`avg`.
    Date(String),
    /// Why the line has no result.
    Error(LineError),
}

/// One converter: `unit` = `factor` × `base`, with `aliases` as extra names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawConverter {
    pub unit: String,
    pub aliases: Vec<String>,
    pub base: String,
    /// A number, or an expression that reduces to one (`1/2.54`).
    pub factor: String,
}

/// A converter name: starts with a letter, at most 24 characters, no
/// operators that would make it unparseable.
static CONVERTER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]{0,23}$").unwrap());

/// A converter factor with nothing in it that could assign a variable or
/// name a unit.
static ARITHMETIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9.eE+*/^() -]+$").unwrap());

/// The most converters the table may hold. Each costs a define and a check
/// on every recalculation.
pub const MAX_CONVERTERS: usize = 64;

/// `name = ...` at the start of a line (but not `==`).
static ASSIGNMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=(?:[^=]|$)").unwrap());

/// Words Soos itself gives a meaning to, taken from the same regexes that
/// act on them, so neither a converter nor a variable can take one.
fn is_reserved_word(name: &str) -> bool {
    PREV.is_match(name)
        || SUM.is_match(name)
        || AVG.is_match(name)
        || INTO_WORD.is_match(name)
        || OPERATOR_WORD.is_match(name)
        || CONVERSION_WORD.is_match(name)
        || DATE_WORD.is_match(name)
}

/// The variable a line assigns to (`rent` in `rent = 1800`), if any.
fn assigned_name(expr: &str) -> Option<&str> {
    Some(ASSIGNMENT.captures(expr)?.name("name")?.as_str())
}

/// The reserved word a line assigns to (`total = 5`), if any.
fn reserved_assignment(expr: &str) -> Option<&str> {
    let name = assigned_name(expr)?;
    is_reserved_word(name).then_some(name)
}

/// Whether a variable called `name` would take over a unit (`m`, `min`,
/// `EUR`, a converter) or a constant (`e`, `k`): `m = 5` would make every
/// `1 m` below it 5, with no error anywhere. A single letter fend only finds
/// by its other case is free: `a` finds ampere's `A`, which stays `A`.
fn takes_over_a_unit(ctx: &mut fend_core::Context, name: &str) -> bool {
    let own = format!("1 {name}");
    match engine::eval_line(ctx, &own) {
        Err(_) => false,
        Ok(found) => {
            let by_other_case = name.chars().count() == 1
                && found.display != own
                && found.display.eq_ignore_ascii_case(&own);
            !by_other_case
        }
    }
}

/// Whether `needle` is a whole word in `haystack`.
fn mentions(haystack: &str, needle: &str) -> bool {
    haystack
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == needle)
}

/// Whether `name` already means something in `ctx` (a fend or CSS unit, or
/// an earlier converter).
fn already_resolves(ctx: &mut fend_core::Context, name: &str) -> bool {
    engine::eval_line(ctx, &format!("1 {name}")).is_ok()
}

/// A converter's factor: a plain number, or an expression evaluated in a
/// throwaway context so `x = 5` can't leak a variable into the document.
/// `1 kg` is rejected -- the unit belongs in the base.
fn parse_factor(s: &str) -> Result<f64, String> {
    if let Ok(n) = s.parse::<f64>() {
        return Ok(n);
    }
    engine::eval_line(&mut fend_core::Context::new(), s)
        .ok()
        .and_then(|e| e.value)
        .and_then(|v| v.parse::<f64>().ok())
        .ok_or_else(|| "factor not a number".to_string())
}

/// Register one converter, or return a short reason it can't be. Every
/// check runs before anything is registered. A bad alias is skipped and
/// named in the result rather than failing the whole converter.
fn define_converter(
    ctx: &mut fend_core::Context,
    conv: &RawConverter,
    defined_so_far: usize,
) -> Result<String, String> {
    if defined_so_far >= MAX_CONVERTERS {
        return Err("too many converters".to_string());
    }

    let factor = parse_factor(&conv.factor)?;
    if !(factor.is_finite() && factor > 0.0 && (1e-12..=1e12).contains(&factor)) {
        return Err("factor out of range".to_string());
    }
    if conv.base.is_empty()
        || conv.base.len() > 64
        || !conv
            .base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ./*^-".contains(c))
    {
        return Err("invalid base".to_string());
    }
    if !CONVERTER_NAME.is_match(&conv.unit) {
        return Err("invalid unit name".to_string());
    }
    if is_reserved_word(&conv.unit) {
        return Err("unit name reserved".to_string());
    }
    if mentions(&conv.base, &conv.unit) {
        return Err("base uses this unit".to_string());
    }
    if already_resolves(ctx, &conv.unit) {
        return Err("name already taken".to_string());
    }
    // An unknown name in the base is usually a converter defined further
    // down the table, which the table's reorder buttons fix.
    if let Err(e) = engine::eval_line(ctx, &format!("1 {}", conv.base)) {
        let unknown = matches!(&e, LineError::Fend(m) if m.starts_with("unknown identifier"));
        let message = if unknown {
            "define base first"
        } else {
            "invalid base"
        };
        return Err(message.to_string());
    }

    // The factor as typed when it is only arithmetic, so `1/7` stays exact:
    // `parse_factor` has only fend's 10-digit display of it.
    let definition = if ARITHMETIC.is_match(conv.factor.trim()) {
        format!("({}) {}", conv.factor.trim(), conv.base)
    } else {
        format!("{factor} {}", conv.base)
    };
    engine::define_unit(ctx, &conv.unit, &definition);

    let mut skipped: Vec<&str> = Vec::new();
    for alias in &conv.aliases {
        let safe = CONVERTER_NAME.is_match(alias)
            && !is_reserved_word(alias)
            && alias != &conv.unit
            && !mentions(&conv.base, alias)
            && !already_resolves(ctx, alias);
        if safe {
            engine::define_unit(ctx, alias, &definition);
        } else {
            skipped.push(alias);
        }
    }

    let display = engine::eval_line(ctx, &format!("1 {} to {}", conv.unit, conv.base))
        .map_err(|_| "invalid base".to_string())?
        .display;
    if skipped.is_empty() {
        Ok(display)
    } else {
        Ok(format!(
            "{display} \u{b7} skipped: {}",
            join_skipped(&skipped)
        ))
    }
}

/// Three names, then `+N`, so the app's status cell stays short.
fn join_skipped(skipped: &[&str]) -> String {
    const SHOWN: usize = 3;
    if skipped.len() <= SHOWN {
        skipped.join(", ")
    } else {
        format!(
            "{}, +{}",
            skipped[..SHOWN].join(", "),
            skipped.len() - SHOWN
        )
    }
}

/// Register every converter in order -- a later one may build on an earlier
/// one. One result per converter, for the converter table to show.
pub(crate) fn define_converters(
    ctx: &mut fend_core::Context,
    converters: &[RawConverter],
) -> Vec<Result<String, String>> {
    let mut results = Vec::with_capacity(converters.len());
    let mut defined = 0usize;
    for conv in converters {
        let r = define_converter(ctx, conv, defined);
        if r.is_ok() {
            defined += 1;
        }
        results.push(r);
    }
    results
}

/// `prev`: the last successful line's value. `pub(crate)`, like [`SUM`] and
/// [`AVG`], so [`crate::highlight`] colours exactly these words.
pub(crate) static PREV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bprev\b").unwrap());
/// `sum`/`total`: the current block's values added up.
pub(crate) static SUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(sum|total)\b").unwrap());
/// `avg`/`average`: the mean of the same values.
pub(crate) static AVG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(avg|average)\b").unwrap());

/// A result later lines can use: `reference` is what `prev`, `sum` and
/// `avg` substitute, `unit` its unit (`m`, `USD`; empty for a plain number).
struct Held {
    reference: String,
    unit: String,
    /// Shown as a percent, whether or not its value is written with the `%`
    /// (`50 as a % of 100` holds a bare 50).
    percent: bool,
}

/// What `prev` and the aggregates can see at the current line. `prev`
/// reaches back past blank lines, headers and errors. The block is cleared
/// at every blank line, header and label. An aggregate line's own result
/// feeds `prev` but never joins the block, so `avg` after `sum` averages the
/// numbers, not the sum.
#[derive(Default)]
struct RunningValues {
    prev: Option<String>,
    /// The block's plain (non-aggregate) lines.
    block: Vec<Held>,
    /// A plain line in the block has an error, so a total would be wrong.
    block_has_error: bool,
    /// The variables this document has assigned, which may be assigned again
    /// even when their name is also a unit's.
    assigned: HashSet<String>,
}

/// The unit after a value's number (`"258990.56 VND"` -> `"VND"`).
fn unit_of(value: &str) -> &str {
    value
        .trim()
        .split_once(char::is_whitespace)
        .map_or("", |(_, unit)| unit.trim())
}

/// The one unit the block's unit-bearing values share, if exactly one:
/// `5 m`, `3` -> `Some("m")`; `5 m`, `3 kg` -> `None`.
fn common_unit(block: &[Held]) -> Option<&str> {
    let mut units = block
        .iter()
        .map(|held| held.unit.as_str())
        .filter(|unit| !unit.is_empty());
    let first = units.next()?;
    units.all(|u| u == first).then_some(first)
}

/// Join `values` into a balanced sum, `((a) + (b)) + ((c) + (d))`: fend
/// evaluates a flat `a + b + c + ...` one stack frame per `+`, which a long
/// block would push past the nesting cap.
fn balanced_sum(values: &[String]) -> String {
    match values {
        [] => "0".to_string(),
        [only] => format!("({only})"),
        _ => {
            let mid = values.len() / 2;
            format!(
                "({}) + ({})",
                balanced_sum(&values[..mid]),
                balanced_sum(&values[mid..])
            )
        }
    }
}

/// Replace `prev`, `sum`/`total` and `avg`/`average` with the values they
/// stand for. `prev` with nothing above stays `prev`, which fend reports as
/// unknown. With `promote_to`, plain numbers in the block count as that
/// unit (see [`eval_expr`]).
fn substitute_tokens(expr: &str, running: &RunningValues, promote_to: Option<&str>) -> String {
    let mut out = expr.to_string();
    if PREV.is_match(&out) {
        let replacement = running
            .prev
            .as_ref()
            .map_or_else(|| "prev".to_string(), |reference| format!("({reference})"));
        out = PREV.replace_all(&out, replacement.as_str()).into_owned();
    }
    if SUM.is_match(&out) || AVG.is_match(&out) {
        let values: Vec<String> = running
            .block
            .iter()
            .map(|held| match promote_to {
                Some(unit) if held.unit.is_empty() => format!("({}) {unit}", held.reference),
                _ => held.reference.clone(),
            })
            .collect();
        // One operand, like `prev`: unwrapped, `sum * 2` would multiply only
        // the block's last term.
        let sum = format!("({})", balanced_sum(&values));
        if SUM.is_match(&out) {
            out = SUM.replace_all(&out, sum.as_str()).into_owned();
        }
        if AVG.is_match(&out) {
            let avg = format!("({sum} / {})", running.block.len().max(1));
            out = AVG.replace_all(&out, avg.as_str()).into_owned();
        }
    }
    out
}

/// Evaluate one expression line against what's above it.
fn eval_expr(
    ctx: &mut fend_core::Context,
    expr: &str,
    aggregate: bool,
    running: &RunningValues,
    now: DateTime<Local>,
) -> Result<engine::Evaluated, LineError> {
    if let Some(name) = reserved_assignment(expr) {
        return Err(LineError::own(
            format!("'{name}' is a built-in word and can't be a variable name"),
            "reserved name",
        ));
    }
    if let Some(name) = assigned_name(expr) {
        if !running.assigned.contains(name) && takes_over_a_unit(ctx, name) {
            return Err(LineError::own(
                format!("'{name}' is already a unit or constant; pick another name"),
                "name in use",
            ));
        }
    }
    if aggregate {
        if running.block_has_error {
            return Err(LineError::own(
                "a line in this block has an error",
                "error in block",
            ));
        }
        if AVG.is_match(expr) && running.block.is_empty() {
            return Err(LineError::plain("nothing to average"));
        }
        let percents = running.block.iter().filter(|held| held.percent).count();
        if percents > 0 && percents < running.block.len() {
            return Err(LineError::own(
                "can't add a percent to amounts; take it off the total, like 10% off sum",
                "percent in block",
            ));
        }
    }
    let evaluated = engine::eval_line_at(ctx, &substitute_tokens(expr, running, None), now);
    let Err(error) = &evaluated else {
        return evaluated;
    };
    // The block is written into the line, which has a length cap: a long
    // block fails on a line that only says `sum`.
    if aggregate && matches!(error, LineError::TooLong) {
        return Err(LineError::own(
            "too many lines to add up; start a new block",
            "too many lines",
        ));
    }
    // A block of one unit plus plain numbers (`5 m`, `3`) can't be added
    // as is; count the plain numbers as that unit. Two different units
    // (`5 m`, `3 kg`) stay an error.
    if !aggregate || !error.is_unit_mismatch() {
        return evaluated;
    }
    let Some(unit) = common_unit(&running.block) else {
        return evaluated;
    };
    match engine::eval_line_at(ctx, &substitute_tokens(expr, running, Some(unit)), now) {
        Ok(ev) => Ok(ev),
        Err(_) => evaluated,
    }
}

/// A line that ends in a function (`ans`, `root`, a leftover `Q1:500`) has
/// no value to show, and `prev` and `sum` couldn't use it. An assignment may
/// still hold one: `f = \x.x * 2`.
fn reject_a_function(
    expr: &str,
    evaluated: engine::Evaluated,
) -> Result<engine::Evaluated, LineError> {
    let is_function = evaluated
        .value
        .as_deref()
        .is_some_and(|value| value.starts_with('\\'));
    if is_function && !ASSIGNMENT.is_match(expr) {
        return Err(LineError::own(
            "that is a function, not a number",
            "not a number",
        ));
    }
    Ok(evaluated)
}

/// Recalculate every line of `source`, one [`LineResult`] per line. `ctx`
/// must be freshly set up by the caller: every call re-evaluates the whole
/// document, which keeps "a line above changed" simple. Each line is capped
/// in length, nesting and time, so the cost grows linearly with line count.
pub(crate) fn recalc(
    ctx: &mut fend_core::Context,
    source: &str,
    now: DateTime<Local>,
) -> Vec<LineResult> {
    let mut results = Vec::new();
    let mut running = RunningValues::default();

    for (number, raw_line) in source.lines().enumerate() {
        if engine::cancelled() {
            break;
        }
        match preprocess::classify(raw_line) {
            Err(blank_header_or_label) => {
                running.block.clear();
                running.block_has_error = false;
                results.push(blank_header_or_label);
            }
            Ok(expr) => {
                let aggregate = reserved_assignment(&expr).is_none()
                    && (SUM.is_match(&expr) || AVG.is_match(&expr));
                let evaluated = eval_expr(ctx, &expr, aggregate, &running, now)
                    .and_then(|ev| reject_a_function(&expr, ev));
                match evaluated {
                    Err(e) => {
                        if !aggregate {
                            running.block_has_error = true;
                        }
                        results.push(LineResult::Error(e));
                    }
                    Ok(ev) => match ev.value {
                        Some(value) => {
                            if let Some(name) = assigned_name(&expr) {
                                running.assigned.insert(name.to_string());
                            }
                            // Shown exactly, a result reads back as its
                            // text. An inexact one is kept at full precision
                            // in a hidden variable, so `sqrt(2)`, then
                            // `prev^2`, isn't built on rounded digits.
                            let name = format!("__soos{number}");
                            let reference = if !ev.exact && engine::remember_last(ctx, &name) {
                                name
                            } else {
                                value.clone()
                            };
                            running.prev = Some(reference.clone());
                            if !aggregate {
                                let percent = ev.display.ends_with('%');
                                running.block.push(Held {
                                    reference: if percent && !reference.ends_with('%') {
                                        format!("({reference})%")
                                    } else {
                                        reference
                                    },
                                    unit: unit_of(&value).to_string(),
                                    percent,
                                });
                            }
                            results.push(LineResult::Value(ev.display));
                        }
                        None => results.push(LineResult::Date(ev.display)),
                    },
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recalc_str(source: &str) -> Vec<LineResult> {
        let mut ctx = fend_core::Context::new();
        recalc(&mut ctx, source, Local::now())
    }

    fn value(s: &str) -> LineResult {
        LineResult::Value(s.to_string())
    }

    fn error_containing(result: &LineResult, needle: &str) -> bool {
        matches!(result, LineResult::Error(e) if e.to_string().contains(needle))
    }

    #[test]
    fn prev_after_a_base_conversion_is_the_same_number() {
        let results = recalc_str("255 in binary\nprev + 1 in decimal");
        assert_eq!(results[1], value("256"));
    }

    #[test]
    fn a_variable_assigned_a_percent_phrase_holds_its_result() {
        let results = recalc_str("price = 15% off 100\nprice * 2");
        assert_eq!(results, [value("85"), value("170")]);
    }

    #[test]
    fn percent_taken_off_a_block_total() {
        assert_eq!(recalc_str("60\n40\n15% off sum")[2], value("85"));
    }

    /// Only `on` and `off` add or take off a percent; after `+` or `-` it is
    /// the plain fraction, as in any expression.
    #[test]
    fn a_percent_after_plus_or_minus_is_plain_arithmetic() {
        let results = recalc_str("30 + 5%\n100 - 5%\n(100 - 5%) * 2");
        assert_eq!(results, [value("30.05"), value("99.95"), value("199.9")]);
    }

    #[test]
    fn prev_references_previous_line() {
        let results = recalc_str("5 + 5\nprev + 1");
        assert_eq!(results[0], value("10"));
        assert_eq!(results[1], value("11"));
    }

    #[test]
    fn sum_and_avg_over_a_block() {
        let results = recalc_str("1\n2\n3\nsum\navg");
        assert_eq!(results[3], value("6"));
        assert_eq!(results[4], value("2"));
    }

    #[test]
    fn sum_over_incompatible_units_is_an_error() {
        let results = recalc_str("5 m\n3 kg\nsum");
        assert!(
            error_containing(&results[2], "cannot convert"),
            "{results:?}"
        );
    }

    #[test]
    fn sum_over_matching_units_keeps_the_unit() {
        let results = recalc_str("1 m\n50 cm\nsum");
        assert_eq!(results[2], value("1.5 m"));
    }

    #[test]
    fn sum_of_a_unit_and_plain_numbers_uses_that_unit() {
        let results = recalc_str("5 m\n3\ntotal\naverage");
        assert_eq!(results[2], value("8 m"));
        assert_eq!(results[3], value("4 m"));
    }

    #[test]
    fn an_error_in_the_block_makes_the_total_an_error() {
        let results = recalc_str("1\nfoo\n2\nsum\n\n3\nsum");
        assert!(error_containing(&results[3], "has an error"), "{results:?}");
        // The next block starts clean.
        assert_eq!(results[6], value("3"));
    }

    #[test]
    fn avg_of_an_empty_block_is_an_error() {
        let results = recalc_str("avg\nsum");
        assert_eq!(
            results[0],
            LineResult::Error(LineError::plain("nothing to average"))
        );
        assert_eq!(results[1], value("0"));
    }

    #[test]
    fn a_reserved_word_cannot_be_a_variable() {
        for name in [
            "total", "sum", "avg", "average", "prev", "today", "in", "and", "plus", "mul", "before",
        ] {
            let results = recalc_str(&format!("{name} = 5"));
            assert!(error_containing(&results[0], "built-in word"), "{name}");
        }
        // `==` is a comparison, not an assignment.
        assert_eq!(recalc_str("1\nsum == 1")[1], value("true"));
    }

    #[test]
    fn prev_and_sum_keep_full_precision() {
        assert_eq!(recalc_str("1/3\nprev * 3")[1], value("1"));
        assert_eq!(recalc_str("1/3\n1/3\n1/3\nsum")[3], value("1"));
        assert_eq!(recalc_str("sqrt(2)\nprev^2")[1], value("\u{2248} 2"));
        assert_eq!(
            recalc_str("sqrt(2)\nprev + 1")[1],
            value("\u{2248} 2.4142135624")
        );
    }

    #[test]
    fn prev_is_one_operand() {
        assert_eq!(recalc_str("5 m\nprev^2")[1], value("25 m^2"));
        assert_eq!(recalc_str("1 + 2\nprev * 2")[1], value("6"));
    }

    #[test]
    fn a_function_is_not_a_result_unless_assigned() {
        // As the app sets it up: `ans` on the first line is `root`'s lambda.
        let recalc_str = |source: &str| {
            let mut ctx = fend_core::Context::new();
            engine::register_root(&mut ctx);
            recalc(&mut ctx, source, Local::now())
        };
        for source in ["ans", "root", "5\nroot", "Q1:500"] {
            let last = recalc_str(source).pop().unwrap();
            assert!(
                error_containing(&last, "a function, not a number"),
                "{source}"
            );
        }
        assert_eq!(recalc_str("5\nans")[1], value("5"));
        assert_eq!(recalc_str("root 3 (27)")[0], value("3"));
        assert_eq!(recalc_str("f = \\x.x*2\nf 3")[1], value("6"));
        // Like any error, it stops a `sum` from quietly leaving it out.
        assert!(error_containing(
            &recalc_str("1\n2\nroot\nsum")[3],
            "has an error"
        ));
        // `ans` after a result is that result.
        assert_eq!(recalc_str("1\n2\nans\nsum")[3], value("5"));
    }

    /// `m = 5` would turn every `1 m` below it into 5.
    #[test]
    fn a_variable_cannot_take_over_a_unit_or_constant() {
        for name in [
            "m", "s", "g", "h", "t", "c", "e", "k", "min", "cup", "hours", "pi",
        ] {
            let results = recalc_str(&format!("{name} = 5\n2 {name}"));
            assert!(
                error_containing(&results[0], "is already a unit or constant"),
                "{name}: {:?}",
                results[0]
            );
        }
        // Free: unused letters, and a letter fend only finds by its other case.
        for name in [
            "x", "y", "z", "rent", "a", "f", "n", "p", "r", "u", "v", "w", "D",
        ] {
            assert_eq!(
                recalc_str(&format!("{name} = 5\n{name} * 2"))[1],
                value("10"),
                "{name}"
            );
        }
        // A variable the document made can be assigned again, and a
        // converter's name is taken like any unit's.
        assert_eq!(recalc_str("x = 5\nx = x + 1\nx")[2], value("6"));
        assert_eq!(recalc_str("a = 1\na = a + 1")[1], value("2"));
        let (mut ctx, _) = define_all(&[converter("lap", &[], "m", "400")]);
        assert!(error_containing(
            &recalc(&mut ctx, "lap = 5", Local::now())[0],
            "is already a unit or constant"
        ));
    }

    #[test]
    fn sum_and_avg_are_one_operand() {
        assert_eq!(recalc_str("1\n2\nsum * 2")[2], value("6"));
        assert_eq!(recalc_str("1\n2\n2 * sum")[2], value("6"));
        assert_eq!(recalc_str("1\n2\nsum^2")[2], value("9"));
        assert_eq!(recalc_str("1\n2\n-sum")[2], value("-3"));
        assert_eq!(recalc_str("4\n2\navg^2")[2], value("9"));
        assert_eq!(recalc_str("5 m\n3\nsum * 2")[2], value("16 m"));
    }

    #[test]
    fn dates_stay_out_of_prev_and_sum() {
        let results = recalc_str("today + 17 days\n5\nsum");
        assert!(matches!(results[0], LineResult::Date(_)));
        assert_eq!(results[2], value("5"));
        assert_eq!(recalc_str("5\ntoday\nprev + 1")[2], value("6"));
    }

    #[test]
    fn percent_results_feed_back_as_their_number() {
        assert_eq!(recalc_str("50 as a % of 100\nprev + 1")[1], value("51"));
        assert_eq!(recalc_str("50 as a % of 100\n50%\nsum")[2], value("100%"));
    }

    #[test]
    fn a_percent_among_amounts_is_not_added() {
        for source in ["50%\n10\nsum", "5 m\n10%\navg", "50 as a % of 100\n10\nsum"] {
            let results = recalc_str(source);
            assert!(
                error_containing(&results[2], "can't add a percent"),
                "{source}"
            );
        }
        assert_eq!(recalc_str("10%\n20%\nsum")[2], value("30%"));
        assert_eq!(recalc_str("60\n40\n15% off sum")[2], value("85"));
    }

    #[test]
    fn blank_line_breaks_the_block() {
        let results = recalc_str("1\n2\n\n3\nsum");
        assert_eq!(results[4], value("3"));
    }

    #[test]
    fn headers_and_labels_are_not_evaluated() {
        let results = recalc_str("# Totals\nCosts:\n1 + 1");
        assert_eq!(results[0], LineResult::Header);
        assert_eq!(results[1], LineResult::Label);
        assert_eq!(results[2], value("2"));
    }

    /// A heading with a digit in it starts a block like any other label,
    /// and doesn't spoil the `sum` below it.
    #[test]
    fn a_label_with_a_digit_starts_a_block_and_does_not_break_the_sum() {
        let results = recalc_str("1\nWeek 2:\n10\n20\nsum\nQ1: 500");
        assert_eq!(results[1], LineResult::Label);
        assert_eq!(results[4], value("30"));
        assert_eq!(results[5], value("500"));
    }

    #[test]
    fn variables_flow_top_to_bottom() {
        let results = recalc_str("x = 10\nx * 2");
        assert_eq!(results[1], value("20"));
    }

    fn converter(unit: &str, aliases: &[&str], base: &str, factor: &str) -> RawConverter {
        RawConverter {
            unit: unit.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            base: base.to_string(),
            factor: factor.to_string(),
        }
    }

    fn define_all(
        converters: &[RawConverter],
    ) -> (fend_core::Context, Vec<Result<String, String>>) {
        let mut ctx = fend_core::Context::new();
        let results = define_converters(&mut ctx, converters);
        (ctx, results)
    }

    fn display(ctx: &mut fend_core::Context, expr: &str) -> String {
        engine::eval_line(ctx, expr).unwrap().display
    }

    #[test]
    fn converter_defines_a_usable_unit_and_its_aliases() {
        let (mut ctx, results) = define_all(&[converter("teu", &["TEU", "teus"], "cbm", "33.2")]);
        assert!(results[0].is_ok());
        assert_eq!(display(&mut ctx, "2 teu in cbm"), "66.4 m^3");
        assert_eq!(display(&mut ctx, "1 TEU in cbm"), "33.2 m^3");
        assert_eq!(display(&mut ctx, "1 teus in cbm"), "33.2 m^3");
    }

    #[test]
    fn converter_builds_on_an_earlier_one() {
        let (mut ctx, results) = define_all(&[
            converter("teu", &[], "cbm", "33.2"),
            converter("FEU", &[], "teu", "2"),
        ]);
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(display(&mut ctx, "1 FEU in cbm"), "66.4 m^3");
    }

    #[test]
    fn converter_base_on_an_undefined_unit_says_define_it_first() {
        let (_, results) = define_all(&[converter("FEU", &[], "teu", "2")]);
        assert_eq!(results[0], Err("define base first".to_string()));
    }

    #[test]
    fn converter_base_that_does_not_parse_is_an_invalid_base() {
        let (_, results) = define_all(&[converter("teu", &[], "m ^", "2")]);
        assert_eq!(results[0], Err("invalid base".to_string()));
    }

    #[test]
    fn failed_converter_is_not_registered() {
        let (mut ctx, results) = define_all(&[converter("FEU", &[], "teu", "2")]);
        assert!(results[0].is_err());
        let err = engine::eval_line(&mut ctx, "1 FEU")
            .err()
            .unwrap()
            .to_string();
        assert!(err.starts_with("unknown identifier"), "{err}");
    }

    #[test]
    fn converter_reserved_word_errors() {
        for name in ["sum", "prev", "avg", "today", "in", "times", "into"] {
            let (_, results) = define_all(&[converter(name, &[], "m", "2")]);
            assert_eq!(results[0], Err("unit name reserved".to_string()), "{name}");
        }
    }

    #[test]
    fn converter_shadowing_alias_is_skipped_not_fatal() {
        let (mut ctx, results) = define_all(&[converter("teu", &["m"], "cbm", "33.2")]);
        assert!(results[0].as_ref().unwrap().contains("skipped: m"));
        assert_eq!(display(&mut ctx, "1 teu in cbm"), "33.2 m^3");
    }

    #[test]
    fn converter_factor_accepts_an_expression() {
        let (mut ctx, results) = define_all(&[converter("u5", &[], "m", "10/2")]);
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert_eq!(display(&mut ctx, "1 u5 to m"), "5 m");
    }

    /// `1/7` stays exact, where fend's display of it has ten digits.
    #[test]
    fn converter_factor_division_is_exact() {
        let (mut ctx, results) = define_all(&[converter("seventh", &[], "m", "1/7")]);
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert_eq!(display(&mut ctx, "7 seventh to m"), "1 m");
        assert_eq!(display(&mut ctx, "21 seventh to m"), "3 m");
    }

    #[test]
    fn converter_factor_expression_cannot_leak_into_the_document() {
        let (mut ctx, results) = define_all(&[converter("teu", &[], "m", "x = 5")]);
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert!(engine::eval_line(&mut ctx, "x").is_err());
    }

    /// Every rejection, and the app's status cell only has room for a short
    /// message.
    #[test]
    fn converter_errors_are_short() {
        let cases: &[RawConverter] = &[
            converter("t+u", &[], "m", "2"),             // invalid unit name
            converter("sum", &[], "m", "2"),             // unit name reserved
            converter("teu", &[], "2 teu", "3"),         // base uses this unit
            converter("m", &[], "ft", "2"),              // name already taken
            converter("teu", &[], "m", "abc"),           // factor not a number
            converter("teu", &[], "m", "1 kg"),          // factor not a number
            converter("teu", &[], "m", "-1"),            // factor out of range
            converter("teu", &[], "m", "0"),             // factor out of range
            converter("teu", &[], "m", "1e999"),         // factor out of range
            converter("teu", &[], &"x".repeat(65), "2"), // invalid base
            converter("teu", &[], "m ^", "2"),           // invalid base
            converter("FEU", &[], "teu", "2"),           // define base first
        ];
        for conv in cases {
            let (_, results) = define_all(std::slice::from_ref(conv));
            let Err(e) = &results[0] else {
                panic!("expected {conv:?} to error");
            };
            assert!(e.len() <= 20, "{conv:?}: {e:?} is {} chars", e.len());
        }
    }

    #[test]
    fn converter_cap_rejects_the_65th() {
        let converters: Vec<RawConverter> = (0..65)
            .map(|i| converter(&format!("u{i}"), &[], "m", "1"))
            .collect();
        let (_, results) = define_all(&converters);
        assert!(results[63].is_ok());
        assert!(results[64].is_err());
    }
}
