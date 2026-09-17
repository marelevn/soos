//! Display polish applied after fend/`preprocess` produce a raw result --
//! one pure function per concern, so `engine::eval_line`'s callers (CLI,
//! GUI) get the same denser, Numi-style output without duplicating it.

/// The denser, Numi-style stand-in for fend's own `"approx. "` prefix.
pub(crate) const APPROX: &str = "\u{2248} ";

/// fend's own inexact-result prefix (verified in fend-core's `num/unit.rs`
/// and `num/exact.rs`) -- `engine::eval_line` strips it off the raw value it
/// feeds back into `prev`/`sum`/`avg`, and [`approx_symbol`] below swaps it
/// for the shorter [`APPROX`] Numi uses. One constant so the two can't drift
/// apart.
pub(crate) const FEND_APPROX: &str = "approx. ";

/// Swap fend's own `"approx. "` prefix for the shorter [`APPROX`] Numi uses.
pub(crate) fn approx_symbol(display: &str) -> String {
    display
        .strip_prefix(FEND_APPROX)
        .map_or_else(|| display.to_string(), |rest| format!("{APPROX}{rest}"))
}

/// (ISO code, symbol, symbol goes before the amount). Covers what's visible
/// in the reference screenshots plus the other majors -- an unlisted code
/// (fend supports every ISO 4217 currency, see `currency.rs`) just passes
/// through with its code, same as fend's own default rendering.
const CURRENCY_SYMBOLS: &[(&str, &str, bool)] = &[
    ("USD", "$", true),
    ("EUR", "\u{20ac}", true),
    ("GBP", "\u{a3}", true),
    ("JPY", "\u{a5}", true),
    ("CNY", "\u{a5}", true),
    ("KRW", "\u{20a9}", true),
    ("INR", "\u{20b9}", true),
    ("VND", "\u{20ab}", false),
    ("THB", "\u{e3f}", true),
    ("PHP", "\u{20b1}", true),
    ("IDR", "Rp", true),
    ("MYR", "RM", true),
    ("SGD", "$", true),
    ("AUD", "$", true),
];

/// Reformat a fend currency result (`"7.7015644 USD"`, optionally
/// `"\u{2248} 7.7015644 USD"` after `approx_symbol`) with a currency symbol
/// instead of the trailing ISO code, rounded to 2 decimals unless
/// `high_precision` is set. Anything that isn't `"<number> <known code>"`
/// (a non-currency result, or a currency `format_currency` doesn't have a
/// symbol for) passes through unchanged.
pub fn format_currency(display: &str, high_precision: bool) -> String {
    let (prefix, rest) = match display.strip_prefix(APPROX) {
        Some(rest) => (APPROX, rest),
        None => ("", display),
    };
    let Some((amount_str, code)) = rest.rsplit_once(' ') else {
        return display.to_string();
    };
    let Some(&(_, symbol, is_prefix)) = CURRENCY_SYMBOLS.iter().find(|(c, _, _)| *c == code) else {
        return display.to_string();
    };
    let Ok(amount) = amount_str.parse::<f64>() else {
        return display.to_string();
    };
    let formatted_amount = if high_precision {
        amount_str.to_string()
    } else {
        format!("{amount:.2}")
    };
    let body = if is_prefix {
        format!("{symbol}{formatted_amount}")
    } else {
        format!("{formatted_amount} {symbol}")
    };
    format!("{prefix}{body}")
}

/// (needle, short code), checked in order, first match wins -- order
/// matters since a fend error could in principle contain more than one of
/// these substrings. `contains` throughout (not `starts_with`) is safe here
/// because every needle below is specific enough that it doesn't turn up
/// anywhere else in fend's error text -- verified against fend-core's own
/// error-message source, not assumed.
const ERROR_RULES: &[(&str, &str)] = &[
    // fend chains an exchange-rate failure as "failed to retrieve {code}
    // exchange rate: no exchange rate cached for {code}" -- the first half
    // is fend's own wrapper (verified in fend-core's units.rs), the second
    // half is ours (see currency::RateSource's trait impl).
    ("no exchange rate cached for", "no rate"),
    // "cannot convert from X to Y: units '...' and '...' are incompatible"
    // -- fires identically for an explicit "10 m to kg" *and* an implicit
    // mismatch inside "sum"/"avg" (adding a currency value to a unitless
    // one, say), so the label can't claim the user wrote a conversion.
    ("cannot convert from", "unit mismatch"),
    ("unknown identifier", "missing var"),
    ("found '", "syntax error"),
    ("found an invalid token", "syntax error"),
    ("expected a value, instead found", "syntax error"),
    ("is not a function", "not a function"),
    ("unable to parse a valid base prefix", "invalid number"),
    // fend's own text capitalizes "Expected" here, unlike every other variant.
    ("Expected a date literal", "invalid date"),
    (
        "you need to specify what number of decimal places",
        "specify digits",
    ),
    (
        "you need to specify what number of significant figures",
        "specify digits",
    ),
    ("must lie in the interval", "out of range"),
];

/// Collapses fend's (and soos's own) raw error text to a short, generic
/// status word for the result gutter -- no interpolated specifics (which
/// unit, which identifier), since those just make the message long again;
/// the user can trace the actual cause from their own line. Anything that
/// matches nothing passes through unchanged -- most of fend's ~70 error
/// variants are already short (`division by zero`) and don't need a rule at
/// all, and an unrecognized one degrades to "long but not wrong", never a
/// panic or mangled string.
///
/// Two fend variants are deliberately *not* given a rule here:
/// `NoExchangeRatesAvailable` ("exchange rates are not available") can't
/// fire in soos -- a rate handler is always installed, see `new_context` --
/// and `UnableToGetCurrentDate` can't fire either, since
/// `preprocess::eval_date` intercepts `today`/`now` before fend ever sees
/// them.
pub fn shorten_error(error: &str) -> String {
    ERROR_RULES
        .iter()
        .find(|(needle, _)| error.contains(needle))
        .map_or_else(|| error.to_string(), |(_, code)| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_symbol_replaces_prefix() {
        assert_eq!(approx_symbol("approx. 3.14"), "\u{2248} 3.14");
        assert_eq!(approx_symbol("42"), "42");
    }

    #[test]
    fn format_currency_prefix_symbol_low_precision() {
        assert_eq!(format_currency("7.7015644 USD", false), "$7.70");
        assert_eq!(format_currency("7.7015644 USD", true), "$7.7015644");
    }

    #[test]
    fn format_currency_suffix_symbol() {
        assert_eq!(
            format_currency("25861.6232982940 VND", false),
            "25861.62 \u{20ab}"
        );
    }

    #[test]
    fn format_currency_preserves_approx_prefix() {
        assert_eq!(
            format_currency("\u{2248} 7.7015644 USD", false),
            "\u{2248} $7.70"
        );
    }

    #[test]
    fn format_currency_passes_through_unknown_or_non_currency() {
        assert_eq!(format_currency("50.8 cm", false), "50.8 cm");
        assert_eq!(format_currency("6", false), "6");
        assert_eq!(format_currency("12 XYZ", false), "12 XYZ");
    }

    #[test]
    fn shorten_error_collapses_currency_message() {
        assert_eq!(
            shorten_error("failed to retrieve VND exchange rate: no exchange rate cached for VND"),
            "no rate"
        );
        assert_eq!(shorten_error("1 +"), "1 +");
    }

    #[test]
    fn shorten_error_incompatible_units() {
        assert_eq!(
            shorten_error(
                "cannot convert from m to kg: units 'meter' and 'kilogram' are incompatible"
            ),
            "unit mismatch"
        );
        // Same code regardless of which units, or fend's internal sentinel
        // name for soos's exchange-rate base currency ("1 m to USD"
        // produces "...units 'meter' and 'BASE_CURRENCY' are incompatible")
        // -- nothing here is shown to the user, so the sentinel never needs
        // hiding. This is also the exact error a `sum`/`avg` over
        // incompatible units produces (no explicit "to"/"in" involved at
        // all), which is why the label says "mismatch" and not "convert".
        assert_eq!(
            shorten_error(
                "cannot convert from m to USD: units 'meter' and 'BASE_CURRENCY' are incompatible"
            ),
            "unit mismatch"
        );
    }

    #[test]
    fn shorten_error_missing_var() {
        assert_eq!(shorten_error("unknown identifier 'xyz'"), "missing var");
    }

    #[test]
    fn shorten_error_syntax_error() {
        assert_eq!(
            shorten_error("found ')' while expecting '('"),
            "syntax error"
        );
        assert_eq!(
            shorten_error("found an invalid token while expecting ')'"),
            "syntax error"
        );
        assert_eq!(
            shorten_error("expected a value, instead found ')'"),
            "syntax error"
        );
    }

    #[test]
    fn shorten_error_base_prefix() {
        assert_eq!(
            shorten_error("unable to parse a valid base prefix, expected 0b, 0o, or 0x"),
            "invalid number"
        );
    }

    #[test]
    fn shorten_error_decimal_places_and_sig_figs() {
        assert_eq!(
            shorten_error("you need to specify what number of decimal places to use, e.g. '10 dp'"),
            "specify digits"
        );
        assert_eq!(
            shorten_error(
                "you need to specify what number of significant figures to use, e.g. '10 sf'"
            ),
            "specify digits"
        );
    }

    #[test]
    fn shorten_error_not_a_function() {
        assert_eq!(shorten_error("'foo' is not a function"), "not a function");
        assert_eq!(
            shorten_error("'foo' is not a function or number"),
            "not a function"
        );
    }

    #[test]
    fn shorten_error_date_literal() {
        assert_eq!(
            shorten_error("Expected a date literal, e.g. @1970-01-01"),
            "invalid date"
        );
    }

    #[test]
    fn shorten_error_out_of_range() {
        assert_eq!(
            shorten_error("5 must lie in the interval [0, 4]"),
            "out of range"
        );
    }

    #[test]
    fn shorten_error_leaves_already_short_messages_untouched() {
        assert_eq!(shorten_error("division by zero"), "division by zero");
    }
}
