//! Turns a [`LineResult`] into what the app's result column shows and
//! soos-cli prints: currency symbols and rounding, thousands separators,
//! short error labels.

use crate::LineResult;

/// Shown in place of fend's `"approx. "` prefix.
pub(crate) const APPROX: &str = "\u{2248} ";

/// fend's prefix for an inexact result.
pub(crate) const FEND_APPROX: &str = "approx. ";

pub(crate) fn approx_symbol(display: &str) -> String {
    display
        .strip_prefix(FEND_APPROX)
        .map_or_else(|| display.to_string(), |rest| format!("{APPROX}{rest}"))
}

/// One line's result as the app shows it and soos-cli prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The result as displayed, or an error's short label.
    pub text: String,
    /// What clicking the result copies: no thousands separators and no `≈`,
    /// so it reads back into Soos as the same value. For an error, the short
    /// label.
    pub copy: String,
    /// An error's full message (the app's hover text).
    pub error: Option<String>,
    /// Every digit of a value `text` shows rounded (the app's hover text).
    pub full: Option<String>,
}

/// Decimals an inexact result is shown with -- after any leading zeros of
/// a number below 1, so `≈ 0.0000123457` keeps its digits too. fend gives
/// 10, which is noise for `≈ 11.0231131092 lbs`.
const INEXACT_DECIMALS: usize = 4;

/// Format one [`LineResult`]; `None` for a line with nothing to show.
/// `high_precision` keeps every digit instead of rounding a currency amount
/// to its usual decimals and an inexact result to [`INEXACT_DECIMALS`].
pub fn shown(result: &LineResult, high_precision: bool) -> Option<Shown> {
    match result {
        LineResult::Value(value) => {
            if let Some(money) = Money::parse(value, high_precision) {
                return Some(Shown {
                    text: money.render(true),
                    copy: money.render(false),
                    error: None,
                    full: None,
                });
            }
            let rounded = match high_precision {
                false => round_inexact(value),
                true => None,
            };
            Some(Shown {
                text: group_digits(rounded.as_deref().unwrap_or(value)),
                copy: value.strip_prefix(APPROX).unwrap_or(value).to_string(),
                error: None,
                full: rounded.map(|_| group_digits(value)),
            })
        }
        LineResult::Date(date) => Some(Shown {
            text: date.clone(),
            copy: date.clone(),
            error: None,
            full: None,
        }),
        LineResult::Error(error) => {
            let short = shorten_error(error);
            Some(Shown {
                text: short.clone(),
                copy: short,
                error: Some(error.clone()),
                full: None,
            })
        }
        LineResult::Blank | LineResult::Header | LineResult::Label => None,
    }
}

/// An inexact result (`≈ ...`) with each decimal rounded half up to
/// [`INEXACT_DECIMALS`] and trailing zeros dropped; `None` if that changes
/// nothing. An exact result keeps every digit: `1.609344 km` is the
/// definition, not noise. A number in another base (`0b0.0101`) is left
/// as it is.
fn round_inexact(display: &str) -> Option<String> {
    let rest = display.strip_prefix(APPROX)?;
    let chars: Vec<char> = rest.chars().collect();
    let mut out = String::from(APPROX);
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        let starts_number = chars[i].is_ascii_digit()
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '.' | '#')));
        if !starts_number {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        let number: String = chars[start..i].iter().collect();
        let rounded = number.split_once('.').and_then(|(int_part, frac_part)| {
            let leading_zeros = match int_part.bytes().all(|b| b == b'0') {
                true => frac_part.bytes().take_while(|&b| b == b'0').count(),
                false => 0,
            };
            let decimals = leading_zeros + INEXACT_DECIMALS;
            (frac_part.len() > decimals).then(|| round_half_up(&number, decimals))?
        });
        match rounded {
            Some(rounded) => {
                let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
                out.push_str(trimmed);
                changed = true;
            }
            None => out.push_str(&number),
        }
    }
    changed.then_some(out)
}

/// Insert `,` thousands separators into each integer digit run of four or
/// more digits (`"1234567 m"` -> `"1,234,567 m"`). A run right after a
/// letter, digit, `.` or `#` is left alone: a number in another base
/// (`0x1234`, `3#1012`), an exponent or a fractional part.
pub fn group_digits(display: &str) -> String {
    let chars: Vec<char> = display.chars().collect();
    let mut out = String::with_capacity(chars.len() + chars.len() / 3);
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let run = &chars[start..i];
        let preceded_by_number_or_word = start > 0
            && (chars[start - 1].is_alphanumeric() || matches!(chars[start - 1], '.' | '#'));
        if run.len() >= 4 && !preceded_by_number_or_word {
            for (j, &c) in run.iter().enumerate() {
                if j > 0 && (run.len() - j).is_multiple_of(3) {
                    out.push(',');
                }
                out.push(c);
            }
        } else {
            out.extend(run);
        }
    }
    out
}

/// How one currency is shown. A currency without an entry keeps fend's own
/// form, `12 CAD`.
pub(crate) struct CurrencyStyle {
    /// ISO 4217 code, as fend prints it after the amount.
    pub code: &'static str,
    pub symbol: &'static str,
    /// `$5` (true) or `5 ₫` (false).
    pub symbol_first: bool,
    /// Decimal places when high precision is off.
    pub decimals: usize,
}

const fn style(
    code: &'static str,
    symbol: &'static str,
    symbol_first: bool,
    decimals: usize,
) -> CurrencyStyle {
    CurrencyStyle {
        code,
        symbol,
        symbol_first,
        decimals,
    }
}

/// Every symbol is distinct, so a copied result reads back as the same
/// currency (`A$150.00` is AUD, not USD) -- [`crate::preprocess`] parses
/// them from this table. JPY, KRW, VND and IDR have no minor unit in
/// everyday use.
pub(crate) const CURRENCY_STYLES: &[CurrencyStyle] = &[
    style("USD", "$", true, 2),
    style("EUR", "\u{20ac}", true, 2),
    style("GBP", "\u{a3}", true, 2),
    style("JPY", "\u{a5}", true, 0),
    style("CNY", "CN\u{a5}", true, 2),
    style("KRW", "\u{20a9}", true, 0),
    style("INR", "\u{20b9}", true, 2),
    style("VND", "\u{20ab}", false, 0),
    style("THB", "\u{e3f}", true, 2),
    style("PHP", "\u{20b1}", true, 2),
    style("IDR", "Rp", true, 0),
    style("MYR", "RM", true, 2),
    style("SGD", "S$", true, 2),
    style("AUD", "A$", true, 2),
];

/// A fend currency result (`"7.7015644 USD"`, maybe with `≈`), rounded.
struct Money {
    approx: bool,
    negative: bool,
    /// Unsigned decimal digits, already rounded.
    amount: String,
    style: &'static CurrencyStyle,
}

impl Money {
    /// `None` unless `display` is `<number> <code>` for a code in
    /// [`CURRENCY_STYLES`].
    fn parse(display: &str, high_precision: bool) -> Option<Money> {
        let (approx, rest) = match display.strip_prefix(APPROX) {
            Some(rest) => (true, rest),
            None => (false, display),
        };
        let (amount, code) = rest.rsplit_once(' ')?;
        let style = CURRENCY_STYLES.iter().find(|s| s.code == code)?;
        let (negative, digits) = match amount.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, amount),
        };
        let amount = if high_precision {
            is_decimal(digits).then(|| digits.to_string())?
        } else {
            round_half_up(digits, style.decimals)?
        };
        // Rounding -0.001 gives 0.00, not -0.00.
        let negative = negative && amount.bytes().any(|b| matches!(b, b'1'..=b'9'));
        Some(Money {
            approx,
            negative,
            amount,
            style,
        })
    }

    /// For display: with `≈` and thousands separators. Otherwise the plain
    /// form Soos reads back.
    fn render(&self, for_display: bool) -> String {
        let amount = if for_display {
            group_digits(&self.amount)
        } else {
            self.amount.clone()
        };
        let sign = if self.negative { "-" } else { "" };
        let symbol = self.style.symbol;
        let body = if self.style.symbol_first {
            format!("{sign}{symbol}{amount}")
        } else {
            format!("{sign}{amount} {symbol}")
        };
        if self.approx && for_display {
            format!("{APPROX}{body}")
        } else {
            body
        }
    }
}

fn is_decimal(s: &str) -> bool {
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
    !int_part.is_empty()
        && int_part.bytes().all(|b| b.is_ascii_digit())
        && frac_part.bytes().all(|b| b.is_ascii_digit())
}

/// `digits` rounded half up to `decimals` places, on the decimal text:
/// going through `f64` rounds 1.005 down and loses digits past 2^53.
/// `None` if `digits` isn't a plain unsigned decimal.
fn round_half_up(digits: &str, decimals: usize) -> Option<String> {
    if !is_decimal(digits) {
        return None;
    }
    let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));
    let mut kept: Vec<u8> = int_part
        .bytes()
        .chain(
            frac_part
                .bytes()
                .chain(std::iter::repeat(b'0'))
                .take(decimals),
        )
        .collect();
    if frac_part
        .as_bytes()
        .get(decimals)
        .is_some_and(|&d| d >= b'5')
    {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let (int_digits, frac_digits) = kept.split_at(kept.len() - decimals);
    let mut out = String::from_utf8(int_digits.to_vec()).ok()?;
    if decimals > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(frac_digits).ok()?);
    }
    Some(out)
}

/// (needle, short label): the first needle a message contains picks its
/// label. The needles are fend-core 1.5.8's wording, which is why
/// `Cargo.toml` pins that version exactly. Variants a user couldn't tell
/// apart share a label.
const ERROR_RULES: &[(&str, &str)] = &[
    // fend wraps these as "failed to retrieve USD exchange rate: <ours>".
    ("no exchange rates downloaded yet", "no rates yet"),
    ("no exchange rate cached for", "no rate"),
    // Soos's own messages, from `document` and `engine`.
    ("a line in this block has an error", "error in block"),
    ("is a built-in word", "reserved name"),
    ("is not a percent", "not a percent"),
    ("with @ in front", "date needs @"),
    ("has no time of day", "no time of day"),
    ("only work alone", "unsupported date"),
    // Also what `sum` over two units gives, so not "can't convert".
    ("cannot convert from", "unit mismatch"),
    ("found '", "syntax error"),
    ("found an invalid token", "syntax error"),
    ("expected a value, instead found", "syntax error"),
    ("unexpected character", "syntax error"),
    ("beginning of an identifier", "syntax error"),
    ("digit separator", "syntax error"), // covers singular and plural forms
    ("escape sequence", "syntax error"), // unknown/invalid/out-of-range, all three
    ("uppercase letter, or one of", "syntax error"),
    ("unterminated string literal", "unclosed quote"),
    ("is not a function", "not a function"),
    ("unable to parse a valid base prefix", "invalid number"),
    ("expected a rational number", "invalid number"),
    ("expected a real number", "invalid number"),
    ("expected a unitless number", "no unit expected"),
    ("string cannot be longer than one codepoint", "too long"),
    ("string cannot be empty", "empty string"),
    ("base must be at least", "invalid base"),
    ("base cannot be larger than", "invalid base"),
    ("unable to convert number to a valid base", "invalid base"),
    ("Expected a date literal", "invalid date"),
    ("to a date", "invalid date"), // ParseDateError: "failed to convert '...' to a date"
    ("does not exist, did you mean", "invalid date"), // e.g. February 30th
    (
        "you need to specify what number of decimal places",
        "specify digits",
    ),
    (
        "you need to specify what number of significant figures",
        "specify digits",
    ),
    ("must lie in the interval", "out of range"),
    ("division by zero", "\u{f7} by 0"),
    ("modulo by zero", "mod by 0"),
    ("exponent too large", "too large"),
    ("value is too large", "too large"),
    ("zero to the power of zero is undefined", "undefined"),
    ("negative numbers are not allowed", "no negatives"),
    (
        "roots of negative numbers are not supported",
        "no complex roots",
    ),
    (
        "cannot compute non-integer or negative roots",
        "invalid root",
    ),
    ("cannot convert fraction to integer", "not an integer"),
    ("number cannot be converted to an integer", "not an integer"),
    ("cannot convert complex number to integer", "not an integer"),
    ("cannot convert inexact number to integer", "not an integer"),
    // What `engine`'s per-line deadline produces.
    ("interrupted", "too slow"),
];

/// `unknown metr`: the unknown name is the useful part, so it's kept (up to
/// 12 characters).
fn unknown_identifier_short(error: &str) -> Option<String> {
    let name = error
        .strip_prefix("unknown identifier '")?
        .strip_suffix('\'')?;
    Some(format!(
        "unknown {}",
        name.chars().take(12).collect::<String>()
    ))
}

/// A short label for the result column; the full message stays in
/// [`Shown::error`]. A message no rule matches is kept if it's 20
/// characters or shorter, and otherwise becomes "can't compute".
pub fn shorten_error(error: &str) -> String {
    if let Some(short) = unknown_identifier_short(error) {
        return short;
    }
    if let Some((_, code)) = ERROR_RULES
        .iter()
        .find(|(needle, _)| error.contains(needle))
    {
        return code.to_string();
    }
    if error.chars().count() > 20 {
        "can't compute".to_string()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_value_groups_for_display_but_copies_plain() {
        let shown = shown(&LineResult::Value("1234567.891 USD".into()), false).unwrap();
        assert_eq!(shown.text, "$1,234,567.89");
        assert_eq!(shown.copy, "$1234567.89");
        assert_eq!(shown.error, None);
    }

    #[test]
    fn shown_error_is_the_short_label_with_the_full_message_kept() {
        let raw = "unknown identifier 'metr'";
        let shown = shown(&LineResult::Error(raw.into()), false).unwrap();
        assert_eq!(shown.text, "unknown metr");
        assert_eq!(shown.copy, "unknown metr");
        assert_eq!(shown.error.as_deref(), Some(raw));
    }

    #[test]
    fn shown_nothing_for_lines_without_a_result() {
        for line in [LineResult::Blank, LineResult::Header, LineResult::Label] {
            assert_eq!(shown(&line, false), None);
        }
    }

    #[test]
    fn approx_symbol_replaces_prefix() {
        assert_eq!(approx_symbol("approx. 3.14"), "\u{2248} 3.14");
        assert_eq!(approx_symbol("42"), "42");
    }

    #[test]
    fn group_digits_groups_plain_integers() {
        assert_eq!(group_digits("1234567"), "1,234,567");
        assert_eq!(group_digits("1000"), "1,000");
        assert_eq!(group_digits("-1234567"), "-1,234,567");
        // Under the 4-digit threshold: left alone.
        assert_eq!(group_digits("999"), "999");
    }

    #[test]
    fn group_digits_only_groups_the_integer_part() {
        assert_eq!(group_digits("1234567.89"), "1,234,567.89");
        // A long fractional run stays ungrouped even though it's 4+ digits.
        assert_eq!(group_digits("1.123456789"), "1.123456789");
    }

    #[test]
    fn group_digits_leaves_currency_symbol_and_unit_suffix_alone() {
        assert_eq!(group_digits("$1234567.89"), "$1,234,567.89");
        assert_eq!(group_digits("1234567 m"), "1,234,567 m");
        assert_eq!(group_digits("\u{2248} 1234567.5"), "\u{2248} 1,234,567.5");
    }

    #[test]
    fn group_digits_skips_hex_and_exponents() {
        assert_eq!(group_digits("0x12345"), "0x12345");
        assert_eq!(group_digits("1.5e12345"), "1.5e12345");
        assert_eq!(group_digits("3#100110"), "3#100110");
    }

    /// An inexact result shows four decimals; every digit stays in `copy`
    /// and `full`.
    #[test]
    fn inexact_results_are_rounded_for_display_only() {
        let rounded = shown(
            &LineResult::Value("\u{2248} 11.0231131092 lbs".into()),
            false,
        )
        .unwrap();
        assert_eq!(rounded.text, "\u{2248} 11.0231 lbs");
        assert_eq!(rounded.copy, "11.0231131092 lbs");
        assert_eq!(rounded.full.as_deref(), Some("\u{2248} 11.0231131092 lbs"));

        let precise = shown(
            &LineResult::Value("\u{2248} 11.0231131092 lbs".into()),
            true,
        )
        .unwrap();
        assert_eq!(precise.text, "\u{2248} 11.0231131092 lbs");
        assert_eq!(precise.full, None);
    }

    #[test]
    fn round_inexact_keeps_small_numbers_and_exact_results() {
        assert_eq!(
            round_inexact("\u{2248} 0.0000123456").as_deref(),
            Some("\u{2248} 0.00001235")
        );
        assert_eq!(
            round_inexact("\u{2248} 1.9999999999").as_deref(),
            Some("\u{2248} 2")
        );
        assert_eq!(
            round_inexact("\u{2248} -0.988031625").as_deref(),
            Some("\u{2248} -0.988")
        );
        assert_eq!(round_inexact("\u{2248} 0b0.0101010101"), None);
        assert_eq!(round_inexact("\u{2248} 3.14"), None);
        assert_eq!(round_inexact("1.609344 km"), None);
    }

    /// `(text, copy)` for a value line.
    fn money(value: &str, high_precision: bool) -> (String, String) {
        let shown = shown(&LineResult::Value(value.into()), high_precision).unwrap();
        (shown.text, shown.copy)
    }

    fn pair(text: &str, copy: &str) -> (String, String) {
        (text.to_string(), copy.to_string())
    }

    #[test]
    fn currency_rounds_to_its_usual_decimals_unless_high_precision() {
        assert_eq!(money("7.7015644 USD", false), pair("$7.70", "$7.70"));
        assert_eq!(
            money("7.7015644 USD", true),
            pair("$7.7015644", "$7.7015644")
        );
        assert_eq!(
            money("25861.6232982940 VND", false),
            pair("25,862 \u{20ab}", "25862 \u{20ab}")
        );
        assert_eq!(
            money("1500.7 KRW", false),
            pair("\u{20a9}1,501", "\u{20a9}1501")
        );
    }

    #[test]
    fn currency_rounds_half_up_on_the_decimal_digits() {
        assert_eq!(money("1.005 USD", false).0, "$1.01");
        assert_eq!(money("2.675 USD", false).0, "$2.68");
        assert_eq!(money("0.125 USD", false).0, "$0.13");
        assert_eq!(money("0.124 USD", false).0, "$0.12");
        assert_eq!(money("9.995 USD", false).0, "$10.00");
        assert_eq!(money("999.5 JPY", false).0, "\u{a5}1,000");
        assert_eq!(
            money("12345678901234567.89 USD", false).0,
            "$12,345,678,901,234,567.89"
        );
    }

    #[test]
    fn negative_amounts_put_the_sign_before_the_symbol() {
        assert_eq!(money("-5.5 USD", false), pair("-$5.50", "-$5.50"));
        assert_eq!(money("-1234 VND", false).0, "-1,234 \u{20ab}");
        assert_eq!(money("-0.001 USD", false).0, "$0.00");
    }

    #[test]
    fn currencies_that_share_a_sign_elsewhere_get_distinct_symbols() {
        assert_eq!(money("150 AUD", false).0, "A$150.00");
        assert_eq!(money("150 SGD", false).0, "S$150.00");
        assert_eq!(money("780 CNY", false).0, "CN\u{a5}780.00");
        assert_eq!(money("1500 JPY", false).0, "\u{a5}1,500");
        assert_eq!(money("1500000 IDR", false).0, "Rp1,500,000");
        let mut symbols: Vec<&str> = CURRENCY_STYLES.iter().map(|s| s.symbol).collect();
        symbols.sort_unstable();
        symbols.dedup();
        assert_eq!(symbols.len(), CURRENCY_STYLES.len());
    }

    #[test]
    fn approx_shows_but_is_not_copied() {
        assert_eq!(
            money("\u{2248} 7.7015644 USD", false),
            pair("\u{2248} $7.70", "$7.70")
        );
        assert_eq!(
            money("\u{2248} 1234.5678", false),
            pair("\u{2248} 1,234.5678", "1234.5678")
        );
    }

    #[test]
    fn non_currency_values_pass_through() {
        assert_eq!(money("50.8 cm", false), pair("50.8 cm", "50.8 cm"));
        assert_eq!(money("12 XYZ", false), pair("12 XYZ", "12 XYZ"));
        assert_eq!(money("1e999 USD", false).0, "1e999 USD");
    }

    #[test]
    fn dates_are_shown_as_is() {
        let date = LineResult::Date("Friday, 25 December 2026".into());
        assert_eq!(
            shown(&date, false).unwrap().text,
            "Friday, 25 December 2026"
        );
    }

    #[test]
    fn shorten_error_collapses_currency_message() {
        assert_eq!(
            shorten_error("failed to retrieve VND exchange rate: no exchange rate cached for VND"),
            "no rate"
        );
        assert_eq!(
            shorten_error("failed to retrieve USD exchange rate: no exchange rates downloaded yet"),
            "no rates yet"
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
        // Including fend's internal BASE_CURRENCY name.
        assert_eq!(
            shorten_error(
                "cannot convert from m to USD: units 'meter' and 'BASE_CURRENCY' are incompatible"
            ),
            "unit mismatch"
        );
    }

    #[test]
    fn shorten_error_unknown_identifier_keeps_the_name() {
        assert_eq!(shorten_error("unknown identifier 'xyz'"), "unknown xyz");
        assert_eq!(
            shorten_error("unknown identifier 'a_very_long_mistyped_name'"),
            "unknown a_very_long_"
        );
    }

    #[test]
    fn shorten_error_labels_soos_own_messages() {
        for (message, label) in [
            ("a line in this block has an error", "error in block"),
            (
                "'total' is a built-in word and can't be a variable name",
                "reserved name",
            ),
            (
                "'x' is not a percent; 'on' needs one, like fee = 8%",
                "not a percent",
            ),
            (
                "write a date with @ in front, like @2026-12-25",
                "date needs @",
            ),
            (
                "today has no time of day; use now, like now + 2 hours",
                "no time of day",
            ),
            ("nothing to average", "nothing to average"),
            ("date out of range", "date out of range"),
        ] {
            assert_eq!(shorten_error(message), label, "{message}");
        }
        let explained = crate::engine::eval_line(&mut fend_core::Context::new(), "now * 2");
        assert_eq!(shorten_error(&explained.err().unwrap()), "unsupported date");
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
        assert_eq!(shorten_error("1 +"), "1 +");
    }

    #[test]
    fn shorten_error_division_and_modulo_by_zero() {
        assert_eq!(shorten_error("division by zero"), "\u{f7} by 0");
        assert_eq!(shorten_error("modulo by zero"), "mod by 0");
    }

    #[test]
    fn shorten_error_interrupted_reads_as_too_slow() {
        assert_eq!(shorten_error("interrupted"), "too slow");
    }

    #[test]
    fn shorten_error_too_large_and_undefined() {
        assert_eq!(shorten_error("exponent too large"), "too large");
        assert_eq!(shorten_error("value is too large"), "too large");
        assert_eq!(
            shorten_error("zero to the power of zero is undefined"),
            "undefined"
        );
    }

    #[test]
    fn shorten_error_negative_and_complex_roots() {
        assert_eq!(
            shorten_error("negative numbers are not allowed"),
            "no negatives"
        );
        assert_eq!(
            shorten_error("roots of negative numbers are not supported"),
            "no complex roots"
        );
        assert_eq!(
            shorten_error("cannot compute non-integer or negative roots"),
            "invalid root"
        );
    }

    #[test]
    fn shorten_error_integer_conversion_variants_share_one_label() {
        for msg in [
            "cannot convert fraction to integer",
            "number cannot be converted to an integer",
            "cannot convert complex number to integer",
            "cannot convert inexact number to integer",
        ] {
            assert_eq!(shorten_error(msg), "not an integer");
        }
    }

    #[test]
    fn shorten_error_expected_number_variants_share_invalid_number() {
        assert_eq!(
            shorten_error("expected a rational number"),
            "invalid number"
        );
        assert_eq!(shorten_error("expected a real number"), "invalid number");
        assert_eq!(
            shorten_error("expected a unitless number"),
            "no unit expected"
        );
    }

    #[test]
    fn shorten_error_string_and_base_variants() {
        assert_eq!(
            shorten_error("string cannot be longer than one codepoint"),
            "too long"
        );
        assert_eq!(shorten_error("string cannot be empty"), "empty string");
        assert_eq!(
            shorten_error("unterminated string literal"),
            "unclosed quote"
        );
        assert_eq!(shorten_error("base must be at least 2"), "invalid base");
        assert_eq!(
            shorten_error("base cannot be larger than 36"),
            "invalid base"
        );
        assert_eq!(
            shorten_error("unable to convert number to a valid base"),
            "invalid base"
        );
    }

    #[test]
    fn shorten_error_date_variants_share_invalid_date() {
        assert_eq!(
            shorten_error("Expected a date literal, e.g. @1970-01-01"),
            "invalid date"
        );
        assert_eq!(
            shorten_error("failed to convert 'xyz' to a date"),
            "invalid date"
        );
        assert_eq!(
            shorten_error("February 30, 2026 does not exist, did you mean March 1 or March 2?"),
            "invalid date"
        );
    }

    #[test]
    fn shorten_error_misc_syntax_variants_share_syntax_error() {
        for msg in [
            "unexpected character '$'",
            "'#' is not valid at the beginning of an identifier",
            "digit separators are not allowed",
            "digit separators can only occur between digits",
            "expected a digit separator, found 'x'",
            "unknown escape sequence: \\q",
            "expected an escape sequence between \\x00 and \\x7f",
            "invalid Unicode escape sequence, expected e.g. \\u{7e}",
            "expected an uppercase letter, or one of @[\\]^_? (e.g. \\^H or \\^@)",
        ] {
            assert_eq!(shorten_error(msg), "syntax error");
        }
    }

    #[test]
    fn shorten_error_unmatched_long_message_falls_back_to_cant_compute() {
        assert_eq!(
            shorten_error("zero cannot be represented as a roman numeral"),
            "can't compute"
        );
        assert_eq!(
            shorten_error("invalid dice syntax, try e.g. `4d6`"),
            "can't compute"
        );
    }
}
