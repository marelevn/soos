//! The line-by-line document model: recalculation order, and the `prev` /
//! `sum` / `total` / `avg` tokens that reach across lines (fend has no
//! concept of "the document above this line", so it's implemented here by
//! substituting literal text before each line reaches fend).

use std::sync::LazyLock;

use regex::Regex;

use crate::engine;
use crate::highlight::{CONVERSION_WORD, DATE_WORD};
use crate::preprocess::{self, LineKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineResult {
    Blank,
    /// No payload -- nothing downstream reads the heading text itself.
    Header,
    /// No payload -- same reasoning as `Header`.
    Label,
    Value(String),
    Error(String),
}

/// One converter's fields (unit, aliases, base, factor) -- the input to
/// `define_converter`/`define_converters`. Callers (the app's converter
/// table) build this directly from their own editable rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawConverter {
    pub unit: String,
    pub aliases: Vec<String>,
    pub base: String,
    /// Not yet parsed to a number.
    pub factor: String,
}

/// Longest a converter's unit/alias name may be, and the charset it must
/// use -- letters/digits/underscore, starting with a letter. Rejects
/// operator characters that would make the name unparseable in the very
/// expressions it's meant to be used in.
static CONVERTER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]{0,23}$").unwrap());

/// At most this many converters per document. Each one costs a define plus
/// a verification eval, and this runs on every recalc, so this bounds that
/// cost; raise it if a document legitimately needs more. `pub` so the app
/// can disable its "+ Add converter" button at the cap instead of letting
/// the user add rows that can only ever fail with "too many converters".
pub const MAX_CONVERTERS: usize = 64;

/// Names the document's own substitution/preprocessing already gives
/// meaning to. Reuses those regexes rather than retyping the word list, so
/// this check can't drift from what `substitute_tokens` and
/// `preprocess::rewrite` actually match -- a converter named `sum` would
/// otherwise be silently rewritten mid-expression, a wrong-answer bug.
fn is_reserved_word(name: &str) -> bool {
    PREV.is_match(name)
        || SUM.is_match(name)
        || AVG.is_match(name)
        || CONVERSION_WORD.is_match(name)
        || DATE_WORD.is_match(name)
}

/// Whether `needle` appears as a whole word in `haystack` -- used to reject
/// a converter whose `base` mentions its own unit/alias, which would
/// otherwise be a recursive definition.
fn mentions(haystack: &str, needle: &str) -> bool {
    haystack
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == needle)
}

/// Whether `name` already means something in `ctx` -- a fend/CSS builtin
/// unit, a variable assigned earlier in the document, or an earlier
/// converter -- so defining it here would silently change results
/// elsewhere in the document. An eval probe rather than a hardcoded unit
/// list, so it covers all three the same way.
fn already_resolves(ctx: &mut fend_core::Context, name: &str) -> bool {
    engine::eval_line(ctx, &format!("1 {name}")).is_ok()
}

/// Parses a converter's factor field -- a plain number first (the common
/// case, and free), only then falling back to the engine so an expression
/// like `1/2.54` works too, in an app whose whole premise is that every
/// line is an expression. Evaluated against a *throwaway* context, never
/// the document's: the factor is meant to reduce to a dimensionless number,
/// and must not be able to assign a variable into the context every later
/// line gets evaluated against. `1 kg` still fails here (the value won't
/// parse as `f64`), which is correct -- the base unit is the base column's
/// job, not the factor's.
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

/// Registers one converter (unit = factor * base) as fend custom units, via
/// the same `engine::define_unit` first-party unit packs use. Returns the
/// definition's own value (`1 unit to base`), or an error if
/// `unit`/`base`/`factor` don't make sense -- validated here, before any of
/// it reaches fend, since this becomes a unit definition every later line
/// is evaluated against.
///
/// A bad *alias* is skipped rather than failing the whole converter -- an
/// alias is a best-effort extra name, not the thing the user actually
/// asked for; the unit itself still gets defined under its primary name.
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

    let definition = format!("{factor} {}", conv.base);
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

    // A distinct message from "invalid base" above: the charset check
    // already rejected anything unparseable, so this eval failing means the
    // base names a unit fend doesn't know -- almost always because it's a
    // converter defined *later* in the table (see `define_converters`'
    // doc comment on why order matters). An instruction ("move that row up")
    // rather than a diagnosis is what makes "reorder the rows" (the app's
    // own fix for this) discoverable from the message alone.
    let display = engine::eval_line(ctx, &format!("1 {} to {}", conv.unit, conv.base))
        .map_err(|_| "define base first".to_string())?
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

/// Names the skipped aliases rather than just counting them, capped so the
/// (now-bounded, see the app's `STATUS_COL_WIDTH`) status cell doesn't need
/// to hold an unbounded alias list -- three names plus a `+N` tail covers
/// the common case (one or two bad aliases) and still degrades gracefully
/// for a row that lists a dozen.
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

/// Registers every converter in order (a later one may build on an earlier
/// one's unit, so order matters), against the same context a document is
/// about to be evaluated in. One display-or-error result per input, in the
/// same order, for the converter table to show inline.
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

// pub(crate): also the keyword vocabulary `highlight.rs` colours.
pub(crate) static PREV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bprev\b").unwrap());
pub(crate) static SUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(sum|total)\b").unwrap());
pub(crate) static AVG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(avg|average)\b").unwrap());

/// Successful, non-blank results seen so far in this pass. `prev` and the
/// sum/avg/total block track separately, and reset differently: `prev`
/// reaches the last successful line no matter how many boundaries or
/// errors sit in between (so it's only ever *set*, never cleared, by
/// `recalc`), while `block` resets at every blank/header/label boundary. A
/// `sum`/`avg`/`total` line's own result is visible to `prev` on the next
/// line, but is invisible to a *later* sum/avg/total -- it's neither part
/// of the block nor a boundary, so `sum` immediately followed by `avg`
/// still averages the same plain numbers the sum just added, not the sum
/// itself.
struct RunningValues {
    /// The most recent successful Expr line's display value, if any.
    prev: Option<String>,
    /// Plain (non-aggregate) Expr values in the current block, in order.
    /// A blank/header/label boundary clears this; aggregate lines and
    /// errors leave it untouched -- transparent to block scanning, not a
    /// boundary.
    block: Vec<String>,
}

/// Bare numeric prefix of a value (`"258990.56 VND"` -> `"258990.56"`) --
/// what a `sum`/`avg` over a block of mismatched units falls back to when
/// adding the values as-is doesn't type-check (see `substitute_tokens`'s
/// `bare` parameter).
fn strip_unit(value: &str) -> &str {
    value.split_whitespace().next().unwrap_or(value)
}

/// The one unit shared by every unit-bearing value in `block`, if any --
/// e.g. `["258990.56 VND", "11"]` -> `Some("VND")` (a currency total mixed
/// with a plain number is still a currency total), but `["5 m", "3 kg"]` ->
/// `None` (no sensible unit to reattach to the bare sum). Used to undo
/// `strip_unit`'s work on the bare-fallback result in `recalc`.
fn common_unit(block: &[String]) -> Option<&str> {
    let mut units = block.iter().filter_map(|v| {
        let unit = v[strip_unit(v).len()..].trim();
        (!unit.is_empty()).then_some(unit)
    });
    let first = units.next()?;
    units.all(|u| u == first).then_some(first)
}

/// `bare`: when set, every block value is reduced to its bare number via
/// [`strip_unit`] before joining -- the retry `recalc` makes when summing
/// the block's own units fails, since the user asking `sum` over `100` and
/// `10 usd to vnd` means "total these", not "convert one to the other".
fn substitute_tokens(expr: &str, running: &RunningValues, bare: bool) -> String {
    let mut out = expr.to_string();
    if PREV.is_match(&out) {
        let replacement = running.prev.as_deref().unwrap_or("prev");
        out = PREV.replace_all(&out, replacement).into_owned();
    }
    if SUM.is_match(&out) || AVG.is_match(&out) {
        let sum = if running.block.is_empty() {
            "0".to_string()
        } else {
            running
                .block
                .iter()
                .map(|v| format!("({})", if bare { strip_unit(v) } else { v }))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        if SUM.is_match(&out) {
            out = SUM.replace_all(&out, sum.as_str()).into_owned();
        }
        if AVG.is_match(&out) {
            let avg = format!("(({sum}) / {})", running.block.len().max(1));
            out = AVG.replace_all(&out, avg.as_str()).into_owned();
        }
    }
    out
}

/// Recalculate every line of `source` top to bottom against `ctx`.
/// `ctx` should be freshly seeded by the caller (units/currency handler
/// registered, no stale variables) -- a full pass re-evaluates everything,
/// which keeps "a line above changed" trivially correct at the cost of
/// redoing cheap work on every keystroke.
pub(crate) fn recalc(ctx: &mut fend_core::Context, source: &str) -> Vec<LineResult> {
    let mut results = Vec::new();
    let mut running = RunningValues {
        prev: None,
        block: Vec::new(),
    };

    for raw_line in source.lines() {
        match preprocess::classify(raw_line) {
            LineKind::Blank => {
                running.block.clear();
                results.push(LineResult::Blank);
            }
            LineKind::Header => {
                running.block.clear();
                results.push(LineResult::Header);
            }
            LineKind::Label => {
                running.block.clear();
                results.push(LineResult::Label);
            }
            LineKind::Expr(expr) => {
                let is_aggregate = SUM.is_match(&expr) || AVG.is_match(&expr);
                let mut evaluated =
                    engine::eval_line(ctx, &substitute_tokens(&expr, &running, false));
                // A block mixing "10000" and "258990.56 VND" can't be added
                // as units -- fend says "cannot convert from". A sum/avg is
                // asking to total the block, so retry on the bare numbers
                // rather than show an error for a total the user can see is
                // right there. A fallback that also fails leaves the
                // original (more informative) error in place.
                if is_aggregate
                    && matches!(&evaluated, Err(e) if crate::format::shorten_error(e) == "unit mismatch")
                {
                    let bare = substitute_tokens(&expr, &running, true);
                    if let Ok(mut ev) = engine::eval_line(ctx, &bare) {
                        // The bare retry above strips every block value's
                        // unit, including a currency code -- if the block
                        // actually agreed on one unit (a currency total mixed
                        // with a plain number, say), reattach it so the
                        // result still formats (rounding, symbol) like any
                        // other currency line instead of showing a raw,
                        // unformatted float regardless of high_precision.
                        if let Some(unit) = common_unit(&running.block) {
                            ev.display = format!("{} {unit}", ev.display);
                            ev.value = ev.value.map(|v| format!("{v} {unit}"));
                        }
                        evaluated = Ok(ev);
                    }
                }
                match evaluated {
                    // Transparent to block scanning: an error doesn't reset
                    // the block or touch `prev`, it just contributes nothing.
                    Err(e) => results.push(LineResult::Error(e)),
                    Ok(ev) => {
                        // A date/time line's `value` is `None` -- its display
                        // isn't fend-parseable (see `engine::Evaluated`'s doc
                        // comment), so it's left out of `prev`/`sum`/`avg`
                        // entirely, the same as an error, rather than being
                        // fed back in and silently misread.
                        if let Some(value) = ev.value {
                            running.prev = Some(value.clone());
                            if !is_aggregate {
                                running.block.push(value);
                            }
                        }
                        results.push(LineResult::Value(ev.display));
                    }
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
        recalc(&mut ctx, source)
    }

    #[test]
    fn prev_references_previous_line() {
        let results = recalc_str("5 + 5\nprev + 1");
        assert_eq!(results[0], LineResult::Value("10".into()));
        assert_eq!(results[1], LineResult::Value("11".into()));
    }

    #[test]
    fn sum_and_avg_over_a_block() {
        let results = recalc_str("1\n2\n3\nsum\navg");
        assert_eq!(results[3], LineResult::Value("6".into()));
        assert_eq!(results[4], LineResult::Value("2".into()));
    }

    #[test]
    fn sum_over_mixed_units_totals_the_bare_numbers() {
        // "5 m" + "3 kg" can't be added as units -- the user asking for a
        // sum means "total these numbers", not "convert one to the other".
        let results = recalc_str("5 m\n3 kg\nsum");
        assert_eq!(results[2], LineResult::Value("8".into()));
    }

    #[test]
    fn sum_over_matching_units_still_keeps_the_unit() {
        // The mixed-unit fallback must not become the default: a block
        // that *does* share a unit still totals with it attached.
        let results = recalc_str("1 m\n50 cm\nsum");
        assert_eq!(results[2], LineResult::Value("1.5 m".into()));
    }

    #[test]
    fn sum_of_a_unit_and_a_plain_number_keeps_the_unit() {
        // "5 m" + a bare "3" can't add as units either (fend won't add a
        // number to a length), but the block only ever names one real unit
        // -- the bare fallback should reattach it rather than show a raw
        // "8" that skips a length result's own formatting.
        let results = recalc_str("5 m\n3\nsum");
        assert_eq!(results[2], LineResult::Value("8 m".into()));
    }

    #[test]
    fn total_and_avg_aliases_also_keep_the_unit() {
        // `total` and `avg`/`average` share the exact same aggregate/bare
        // fallback path as `sum` (see `SUM`/`AVG` regexes) -- not a
        // `sum`-only fix.
        let results = recalc_str("5 m\n3\ntotal\naverage");
        assert_eq!(results[2], LineResult::Value("8 m".into()));
        assert_eq!(results[3], LineResult::Value("4 m".into()));
    }

    #[test]
    fn prev_after_an_approx_result_still_parses() {
        // sqrt(2) is inexact, so its display is "≈ 1.4142135624" -- prev
        // must substitute the bare number (fend rejects a leading "≈"), not
        // that display string. The substituted value is a rounded decimal
        // literal by this point, so the arithmetic on it is exact.
        let results = recalc_str("sqrt(2)\nprev + 1");
        assert_eq!(results[1], LineResult::Value("2.4142135624".into()));
    }

    #[test]
    fn sum_over_a_block_with_an_approx_value() {
        let results = recalc_str("sqrt(2)\n2\nsum");
        assert_eq!(results[2], LineResult::Value("3.4142135624".into()));
    }

    #[test]
    fn date_line_is_transparent_to_sum_not_poisoning() {
        // A date line's display string ("2026-10-02") isn't fend-parseable
        // -- feeding it back into `sum` would parse it as subtraction and
        // silently return 2019 instead of erroring or (as here) just
        // skipping the date line.
        let results = recalc_str("today + 17 days\n5\nsum");
        assert_eq!(results[2], LineResult::Value("5".into()));
    }

    #[test]
    fn date_line_is_transparent_to_prev_too() {
        // Same reasoning, `prev` side: resolving to the date's raw display
        // string would silently corrupt the next line's arithmetic instead
        // of reaching past the date to the last real value.
        let results = recalc_str("5\ntoday\nprev + 1");
        assert_eq!(results[2], LineResult::Value("6".into()));
    }

    #[test]
    fn percent_result_sums_as_its_number_not_a_fraction() {
        // "50%" displays with a literal '%' fend can't parse back in as
        // "50" -- it reads as 0.5, which would make `prev + 1` come out
        // "150%" instead of 51. `engine::Evaluated::value` carries the bare
        // "50" instead.
        let results = recalc_str("50 as a % of 100\nprev + 1");
        assert_eq!(results[1], LineResult::Value("51".into()));
    }

    #[test]
    fn percent_result_in_a_sum_block() {
        let results = recalc_str("50 as a % of 100\n10\nsum");
        assert_eq!(results[2], LineResult::Value("60".into()));
    }

    #[test]
    fn blank_line_breaks_the_block() {
        let results = recalc_str("1\n2\n\n3\nsum");
        // sum only sees "3" -- the blank line above resets the block.
        assert_eq!(results[4], LineResult::Value("3".into()));
    }

    #[test]
    fn headers_and_labels_are_not_evaluated() {
        let results = recalc_str("# Totals\nCosts:\n1 + 1");
        assert_eq!(results[0], LineResult::Header);
        assert_eq!(results[1], LineResult::Label);
        assert_eq!(results[2], LineResult::Value("2".into()));
    }

    #[test]
    fn variables_flow_top_to_bottom() {
        let results = recalc_str("x = 10\nx * 2");
        assert_eq!(results[1], LineResult::Value("20".into()));
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

    #[test]
    fn converter_defines_a_usable_unit_and_its_aliases() {
        let (mut ctx, results) = define_all(&[converter("teu", &["TEU", "teus"], "cbm", "33.2")]);
        assert!(results[0].is_ok());
        assert_eq!(
            engine::eval_line(&mut ctx, "2 teu in cbm").unwrap().display,
            "66.4 m^3"
        );
        assert_eq!(
            engine::eval_line(&mut ctx, "1 TEU in cbm").unwrap().display,
            "33.2 m^3"
        );
        assert_eq!(
            engine::eval_line(&mut ctx, "1 teus in cbm")
                .unwrap()
                .display,
            "33.2 m^3"
        );
    }

    #[test]
    fn converter_builds_on_an_earlier_one() {
        let (mut ctx, results) = define_all(&[
            converter("teu", &[], "cbm", "33.2"),
            converter("FEU", &[], "teu", "2"),
        ]);
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert_eq!(
            engine::eval_line(&mut ctx, "1 FEU in cbm").unwrap().display,
            "66.4 m^3"
        );
    }

    #[test]
    fn converter_base_on_an_undefined_unit_errors() {
        // "teu" isn't defined yet -- the verification eval must catch this
        // on this converter itself, not silently succeed and fail later.
        // Distinct message from the charset/length rejection below: this is
        // the one the reorder buttons are meant to fix.
        let (_, results) = define_all(&[converter("FEU", &[], "teu", "2")]);
        assert_eq!(results[0], Err("define base first".to_string()));
    }

    #[test]
    fn converter_bad_name_charset_errors() {
        let (_, results) = define_all(&[converter("t+u", &[], "m", "2")]);
        assert!(results[0].is_err());
    }

    #[test]
    fn converter_reserved_word_errors() {
        let (_, results) = define_all(&[converter("sum", &[], "m", "2")]);
        assert!(results[0].is_err());
    }

    #[test]
    fn converter_shadowing_existing_unit_errors() {
        let (_, results) = define_all(&[converter("m", &[], "ft", "2")]);
        assert!(results[0].is_err());
    }

    #[test]
    fn converter_shadowing_alias_is_skipped_not_fatal() {
        // "m" already means metres -- the alias is dropped, but the unit
        // itself still defines and works, and the message names it rather
        // than just counting it.
        let (mut ctx, results) = define_all(&[converter("teu", &["m"], "cbm", "33.2")]);
        match &results[0] {
            Ok(d) => assert!(d.contains("skipped: m")),
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
        assert_eq!(
            engine::eval_line(&mut ctx, "1 teu in cbm").unwrap().display,
            "33.2 m^3"
        );
    }

    #[test]
    fn converter_self_reference_errors() {
        let (_, results) = define_all(&[converter("teu", &[], "2 teu", "3")]);
        assert!(results[0].is_err());
    }

    #[test]
    fn converter_bad_factor_errors() {
        for factor in ["0", "-1", "abc", "1e999"] {
            let (_, results) = define_all(&[converter("teu", &[], "m", factor)]);
            assert!(results[0].is_err(), "factor {factor} should have errored");
        }
    }

    #[test]
    fn converter_factor_accepts_an_expression() {
        // Not just a bare number -- the factor field is an expression like
        // any other line in the app. "10/2" (rather than something like
        // "1/2.54") so the assertion doesn't depend on fend's own decimal
        // formatting of a repeating fraction.
        let (mut ctx, results) = define_all(&[converter("u5", &[], "m", "10/2")]);
        assert!(results[0].is_ok(), "{:?}", results[0]);
        let d = engine::eval_line(&mut ctx, "1 u5 to m").unwrap().display;
        assert_eq!(d, "5 m");
    }

    #[test]
    fn converter_factor_expression_cannot_leak_into_the_document() {
        // "x = 5" is itself a valid factor expression (it evaluates to 5) --
        // the point is that it must be evaluated in a throwaway context, so
        // `x` is never resolvable in the *document's* context afterward.
        let (mut ctx, results) = define_all(&[converter("teu", &[], "m", "x = 5")]);
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert!(engine::eval_line(&mut ctx, "x").is_err());
    }

    #[test]
    fn converter_factor_rejects_a_unit_value() {
        // "1 kg" evaluates fine but isn't a dimensionless number -- that's
        // the base column's job, not the factor's.
        let (_, results) = define_all(&[converter("teu", &[], "m", "1 kg")]);
        assert!(results[0].is_err());
    }

    #[test]
    fn converter_bad_base_errors() {
        let (_, results) = define_all(&[converter("teu", &[], &"x".repeat(65), "2")]);
        assert!(results[0].is_err());
    }

    #[test]
    fn converter_error_messages_stay_short() {
        // The GUI's status cell is now allocated at a fixed width (see
        // `STATUS_COL_WIDTH` in main.rs) and truncates rather than resizing
        // the modal, so this is belt-and-braces rather than the only
        // defense -- but every converter error should still be short and
        // fixed, never echo user input back at unbounded length.
        let cases: &[RawConverter] = &[
            converter("t+u", &[], "m", "2"),             // invalid unit name
            converter("sum", &[], "m", "2"),             // unit name reserved
            converter("teu", &[], "2 teu", "3"),         // base uses this unit
            converter("m", &[], "ft", "2"),              // name already taken
            converter("teu", &[], "m", "abc"),           // factor not a number
            converter("teu", &[], "m", "-1"),            // factor out of range
            converter("teu", &[], &"x".repeat(65), "2"), // invalid base
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
