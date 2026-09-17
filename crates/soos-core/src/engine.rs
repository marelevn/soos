//! Thin wrapper around fend-core: one evaluation call, plus the unit table
//! soos ships that fend itself doesn't know.

use crate::preprocess;

/// One successful line: what to show, and what `prev`/`sum`/`avg`/`total`
/// may feed back into fend. They differ whenever soos adds display-only
/// polish fend can't re-read -- the `\u{2248}` prefix, the `%` suffix on
/// "X as a % of Y" -- or when the result never came from fend at all: a
/// date/time line's display (`"2026-10-02"`) isn't fend-parseable, so
/// feeding it back into a later `sum` would parse it as subtraction
/// (`2026 - 10 - 2 = 2014`) and silently corrupt the total instead of
/// erroring.
pub(crate) struct Evaluated {
    pub display: String,
    /// `None` for a date/time: computed entirely outside fend (see
    /// `preprocess::eval_date`), so there is no fend-parseable value to
    /// substitute -- `document::recalc` leaves such a line out of
    /// `prev`/`sum`/`avg` entirely rather than feed back a string fend
    /// would misread.
    pub value: Option<String>,
}

/// Evaluate one already-classified expression line against a fend context.
/// The context is mutated (variable assignments persist into it), matching
/// fend's own model -- callers own the Context's lifetime across a document
/// pass so `x = 5` on one line is visible to `x + 1` on a later one.
pub(crate) fn eval_line(ctx: &mut fend_core::Context, expr: &str) -> Result<Evaluated, String> {
    if let Some(date) = preprocess::eval_date(expr) {
        return Ok(Evaluated {
            display: date,
            value: None,
        });
    }
    let (rewritten, as_percent) = preprocess::rewrite(expr);
    let r = fend_core::evaluate(&rewritten, ctx)?;
    let raw = r.get_main_result();
    // The bare value substitution needs: fend prefixes an inexact result
    // with "approx. ", which it can't parse back in on a later line, same
    // reasoning as the `%` suffix below.
    let value = raw
        .strip_prefix(crate::format::FEND_APPROX)
        .unwrap_or(raw)
        .to_string();
    let mut display = crate::format::approx_symbol(raw);
    if as_percent && !display.is_empty() {
        display.push('%');
    }
    Ok(Evaluated {
        display,
        value: Some(value),
    })
}

/// Register one unit or constant -- used both for soos's own `CSS_UNITS`
/// below and for the user's own converters (see `document::define_converters`).
/// `definition` is a plain fend expression string, e.g. `"0.3048 m"` -- see fend-core's own units.rs
/// for the format (it's evaluated the same way as fend's built-in units,
/// `$CURRENCY` included as a special sentinel). Registers `name` as both
/// singular and plural with no SI-prefix support -- every caller wants
/// exactly that; call `ctx.define_custom_unit_v1` directly for anything else.
pub(crate) fn define_unit(ctx: &mut fend_core::Context, name: &str, definition: &str) {
    ctx.define_custom_unit_v1(
        name,
        name,
        definition,
        &fend_core::CustomUnitAttribute::None,
    );
}

/// CSS/typography units, none of which fend knows. Anchored on the CSS
/// reference pixel (1px = 1/96 inch) and the browser default root font size
/// (1rem = 1em = 16px) -- both are conventions, not physical constants.
/// Registered in `new_context`, before any document is read, so the user's
/// own converters (see `document::define_converters`) see these as
/// already-taken names and can only add new units, never shadow one of
/// these. Note `pt`, `rem` and `ch` shadow fend builtins (pint, roentgen
/// equivalent man, and the 66-foot chain) -- intentional, that's what users
/// mean here.
const CSS_UNITS: &[(&str, &str)] = &[
    ("px", "1/96 inch"),
    ("pt", "1/72 inch"),
    ("pc", "12 pt"),
    ("rem", "16 px"),
    ("em", "16 px"),
    ("ch", "8 px"), // ~advance width of "0" at 16px in a typical font
];

/// Register the units soos ships itself (see `CSS_UNITS`).
pub(crate) fn register_builtin_units(ctx: &mut fend_core::Context) {
    for (name, def) in CSS_UNITS {
        define_unit(ctx, name, def);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only shorthand: most callers only care about the display
    /// string, not the separate substitution `value` -- see `document.rs`'s
    /// own tests for coverage of when the two differ.
    pub(crate) fn eval(ctx: &mut fend_core::Context, expr: &str) -> Result<String, String> {
        eval_line(ctx, expr).map(|e| e.display)
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
        // pt is fend's *pint* until this shadows it.
        assert_eq!(eval(&mut ctx, "72 pt to inches"), Ok("1 inch".to_string()));
    }

    #[test]
    fn date_line_has_no_substitution_value() {
        // The whole point of splitting display/value: a date's display
        // isn't fend-parseable, so there must be nothing to substitute.
        let mut ctx = fend_core::Context::new();
        let ev = eval_line(&mut ctx, "today").unwrap();
        assert!(ev.value.is_none());
    }

    #[test]
    fn approx_and_percent_values_are_fend_parseable() {
        let mut ctx = fend_core::Context::new();
        let ev = eval_line(&mut ctx, "sqrt(2)").unwrap();
        assert_eq!(ev.display, "\u{2248} 1.4142135624");
        assert_eq!(ev.value.as_deref(), Some("1.4142135624"));

        let ev = eval_line(&mut ctx, "50 as a % of 100").unwrap();
        assert_eq!(ev.display, "50%");
        // Not "50%" -- fend can't parse a bare "%" suffix back in as the
        // number 50, only as 0.5. See the `document.rs` sum/avg tests for
        // the wrong-answer that would cause if `value` fed the display
        // string back in instead.
        assert_eq!(ev.value.as_deref(), Some("50"));
    }
}
