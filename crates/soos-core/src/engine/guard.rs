//! Lines fend can't be interrupted on. [`super::Deadline`] stops most long
//! calculations, but three paths in fend-core 1.5.8 never look at it, so a
//! line that reaches one holds the recalculation thread for good (in the app,
//! every tab with it) or for many seconds. They are refused here, before
//! fend sees the line:
//!
//! - A die (`4d6`, `d20`). Combining two dice multiplies their outcomes:
//!   `d1000 + d1000` never returns and `d80 / d80` takes seconds, so no size
//!   limit is enough. Soos has no use for dice -- the answer is a table of
//!   odds, not a number.
//! - A `<<` by more than [`MAX_SHIFT`]. fend inserts a zero at the front of
//!   the number once per 64 places, so the cost is quadratic in the count.
//! - A `^` by more than [`MAX_POWER`]. A unit that needs converting, raised
//!   to a large power and then combined with another dimension (`cm^400000
//!   kg`), takes seconds.
//!
//! A count can be a variable, a bracketed sum or `100000000 to hex`, so it
//! is evaluated, on a copy of the context, rather than read off as digits.

use std::sync::LazyLock;

use regex::Regex;

use super::{Deadline, EVAL_TIMEOUT};
use crate::error::LineError;

/// The most places a `<<` may shift. Past a million the loop takes longer
/// than [`EVAL_TIMEOUT`]; past a hundred million it doesn't finish. A number
/// this long (30,000 digits) can't be shown anyway.
const MAX_SHIFT: u64 = 100_000;
/// The largest `^` exponent. `cm^40000 kg` takes 0.3 s, `mile^30000 kg`
/// 0.4 s, and the cost grows with the square.
const MAX_POWER: u64 = 10_000;
/// The errors for those two limits, in words the result column's hover shows.
pub(super) const SHIFT_TOO_LARGE: &str = "a shift over 100,000 places is too large to calculate";
pub(super) const POWER_TOO_LARGE: &str = "a power over 10,000 is too large to calculate";

/// A hex literal, in which `d` is a digit.
static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"0[xX][0-9a-fA-F_]*").unwrap());
/// A `d` fend reads as dice: after a digit (`4d6`, also in `0o14d5`) or at
/// the start of a word (`d20`), and followed by a digit.
static DIE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[0-9]|(?:^|[^0-9A-Za-z_.]))d[0-9]").unwrap());

/// Operator words that bind more loosely than a shift in fend's parser.
const LOOSER_THAN_SHIFT: [&str; 10] = [
    "xor", "XOR", "and", "AND", "or", "OR", "nCr", "choose", "nPr", "permute",
];

#[derive(Clone, Copy)]
enum Hazard {
    Shift,
    Power,
}

/// `Ok` if fend can be handed `expr` (the expression after the natural-
/// language rewrite), or the error to show instead. Fast for ordinary lines:
/// fend is only asked to evaluate a count that isn't plain digits.
pub(super) fn check(ctx: &fend_core::Context, expr: &str) -> Result<(), LineError> {
    if DIE.is_match(&HEX.replace_all(expr, " ")) {
        return Err(LineError::own("dice like 4d6 aren't supported", "no dice"));
    }
    // (where the count starts, which operator it belongs to)
    let mut hazards: Vec<(usize, Hazard)> = expr
        .match_indices("<<")
        .map(|(at, _)| (at + 2, Hazard::Shift))
        .chain(
            expr.match_indices("**")
                .map(|(at, _)| (at + 2, Hazard::Power)),
        )
        .chain(
            expr.match_indices('^')
                .map(|(at, _)| (at + 1, Hazard::Power)),
        )
        .collect();
    if hazards.is_empty() {
        return Ok(());
    }
    // A probe evaluates a count on a copy of the context, so what the line
    // assigns or defines before the shift must not change that count.
    if hazards.iter().any(|(_, h)| matches!(h, Hazard::Shift))
        && after_assignment(expr).contains([';', '\\', '\u{3bb}', ':', '='])
    {
        return Err(LineError::own(
            "a line with << can't also have a function, a ; or a second =",
            "can't shift",
        ));
    }
    // Right to left, so a count that holds another shift or power is only
    // evaluated after that one has passed.
    hazards.sort_by_key(|&(start, _)| std::cmp::Reverse(start));
    let deadline = Deadline::new(EVAL_TIMEOUT);
    for (start, hazard) in hazards {
        let (count, limit, too_large, what) = match hazard {
            Hazard::Shift => (
                shift_count(expr, start),
                MAX_SHIFT,
                SHIFT_TOO_LARGE,
                "shift",
            ),
            Hazard::Power => (
                power_exponent(expr, start),
                MAX_POWER,
                POWER_TOO_LARGE,
                "power",
            ),
        };
        if let Some(size) = size_of(ctx, count, what, &deadline)? {
            if size > limit {
                return Err(LineError::own(too_large, "too large"));
            }
        }
    }
    Ok(())
}

/// The line after a leading `name =` (but not `==` or `=>`), or all of it.
fn after_assignment(expr: &str) -> &str {
    let trimmed = expr.trim_start();
    let name = trimmed
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    match trimmed[name..].trim_start().strip_prefix('=') {
        Some(after) if name > 0 && !after.starts_with(['=', '>']) => after,
        _ => expr,
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The text of the count after a `<<` at `start`. fend reads an additive
/// expression there (`parse_bitshifts`), which ends at the next `<<`, `>>`,
/// `&`, `|`, `xor`-like word, or the bracket that closes it.
fn shift_count(expr: &str, start: usize) -> &str {
    let rest = &expr[start..];
    let mut depth = 0usize;
    // Where the current word began, and the bracket depth it began at.
    let mut word: Option<(usize, usize)> = None;
    // The extra space ends a word that runs to the end of the line.
    for (i, c) in rest.char_indices().chain([(rest.len(), ' ')]) {
        let in_word = c.is_alphanumeric() || c == '_';
        match (word, in_word) {
            (None, true) => word = Some((i, depth)),
            (Some((from, at_depth)), false) => {
                if at_depth == 0 && LOOSER_THAN_SHIFT.contains(&&rest[from..i]) {
                    return &rest[..from];
                }
                word = None;
            }
            _ => {}
        }
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return &rest[..i],
            ')' => depth -= 1,
            '&' | '|' | '<' | '>' | '\u{2260}' if depth == 0 => return &rest[..i],
            _ => {}
        }
    }
    rest
}

/// The text of the exponent after a `^` or `**` at `start`: signs, then one
/// operand -- a bracket, or a number or name with the call that follows it.
fn power_exponent(expr: &str, start: usize) -> &str {
    let rest = &expr[start..];
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'-' | b'+') {
        i += 1;
    }
    let operand = i;
    let mut end = i;
    if bytes.get(i) == Some(&b'(') {
        return &rest[..closing(bytes, i)];
    }
    // A comma is a thousands separator when digits are on both sides.
    let separator = |at: usize| {
        bytes[at] == b','
            && bytes[at - 1].is_ascii_digit()
            && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)
    };
    while end < bytes.len()
        && (is_word_byte(bytes[end]) || bytes[end] == b'.' || (end > operand && separator(end)))
    {
        end += 1;
        // The sign in `1e+5` belongs to the number.
        if matches!(bytes[end - 1], b'e' | b'E')
            && matches!(bytes.get(end), Some(b'+' | b'-'))
            && bytes[operand].is_ascii_digit()
        {
            end += 1;
        }
    }
    if end == operand {
        return "";
    }
    if bytes.get(end) == Some(&b'(') {
        end = closing(bytes, end);
    }
    &rest[..end]
}

/// The index after the bracket that closes the one at `open`, or the end.
fn closing(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

/// How large the whole number `count` is, if it is one. `None` when fend
/// would refuse it at once anyway (a fraction, a quantity, nothing at all).
/// An error from evaluating it is the line's error: it is the one fend
/// gives (`unknown identifier`), and a count fend can't evaluate in time is
/// too large to wait for.
fn size_of(
    ctx: &fend_core::Context,
    count: &str,
    what: &str,
    deadline: &Deadline,
) -> Result<Option<u64>, LineError> {
    let count = count.trim();
    let digits = count.trim_start_matches(['-', '+', ' ']);
    if digits.is_empty() {
        return Ok(None);
    }
    if digits.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(Some(digits.parse().unwrap_or(u64::MAX)));
    }
    let result = fend_core::evaluate_with_interrupt(count, &mut ctx.clone(), deadline)?;
    // fend prints `to hex` bare, so the base is put back as `eval_line_at` does.
    let shown = match super::base_prefix(count) {
        Some(prefix) => super::with_base_prefix(result.get_main_result(), &prefix),
        None => result.get_main_result().to_string(),
    };
    match whole_number(&shown) {
        Some(size) => Ok(Some(size)),
        None if shown.contains(['.', ' ', '\u{2248}', '/']) => Ok(None),
        None => Err(LineError::own(
            format!("can't tell how large the {what} is, so it isn't calculated"),
            "unclear",
        )),
    }
}

/// The size of fend's display of a whole number: plain digits, a `0x`/`0b`/
/// `0o` or `3#` base, a `%` -- anything fend can show one in and still use
/// as a count. Past `u64` is `u64::MAX`.
fn whole_number(shown: &str) -> Option<u64> {
    let (shown, percent) = match shown.strip_suffix('%') {
        Some(rest) => (rest, true),
        None => (shown, false),
    };
    let unsigned = shown.strip_prefix('-').unwrap_or(shown);
    let (radix, digits) = if let Some((base, digits)) = unsigned.split_once('#') {
        (
            base.parse::<u32>().ok().filter(|r| (2..=36).contains(r))?,
            digits,
        )
    } else if let Some(digits) = unsigned.strip_prefix("0x") {
        (16, digits)
    } else if let Some(digits) = unsigned.strip_prefix("0b") {
        (2, digits)
    } else if let Some(digits) = unsigned.strip_prefix("0o") {
        (8, digits)
    } else {
        (10, unsigned)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let size = u64::from_str_radix(digits, radix).unwrap_or(u64::MAX);
    Some(if percent { size / 100 } else { size })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(setup: &[&str]) -> fend_core::Context {
        let mut ctx = fend_core::Context::new();
        for line in setup {
            fend_core::evaluate(line, &mut ctx).unwrap();
        }
        ctx
    }

    /// The message of the error `expr` is refused with.
    fn refused(ctx: &fend_core::Context, expr: &str) -> Option<String> {
        let start = std::time::Instant::now();
        let result = check(ctx, expr).err().map(|e| e.short());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "{expr}: {:?}",
            start.elapsed()
        );
        result
    }

    #[test]
    fn the_messages_state_the_limits() {
        assert_eq!(
            SHIFT_TOO_LARGE.replace(',', ""),
            format!("a shift over {MAX_SHIFT} places is too large to calculate")
        );
        assert_eq!(
            POWER_TOO_LARGE.replace(',', ""),
            format!("a power over {MAX_POWER} is too large to calculate")
        );
    }

    #[test]
    fn dice_are_refused_but_hex_and_words_are_not() {
        let ctx = ctx_with(&[]);
        for dice in [
            "4d6",
            "d20",
            "3 + d6",
            "2d1000",
            "d1000 + d1000",
            "0o14d5343",
            "1e3d6",
            "5#42d3",
            "(2d6)",
            "x = 3d4",
        ] {
            assert_eq!(refused(&ctx, dice).as_deref(), Some("no dice"), "{dice}");
        }
        for fine in [
            "0x1d6",
            "0xd6",
            "0XD6 + 1",
            "5d",
            "3 days",
            "10 dm",
            "ad6",
            "x1",
            "5 d",
            "3d",
            "2 hours + 3d + 4",
        ] {
            assert_eq!(refused(&ctx, fine), None, "{fine}");
        }
    }

    #[test]
    fn a_huge_shift_is_refused_however_the_count_is_written() {
        let ctx = ctx_with(&["n = 100000000", "small = 3"]);
        for shift in [
            "4 << 100000000",
            "0 << 100000000",
            "0xff << 100000000",
            "1 << 99999999999999999999999",
            "1 << 1e8",
            "1 << 10^8",
            "1 << (10^8)",
            "1 << (100000000 m / m)",
            "1 << n",
            "1 << 100000000 to hex",
            "1 << (100000000 in base 3)",
            "1 << 100000000000%",
            "1 << 0x5f5e100",
            "1 << 100001",
            "1 << 100,000,000",
            "1 << 100_000_000",
            "2 << 3 << 100000000",
            "(1 << 100000000) + 1",
            "1 << (2 << 100000000)",
            "x = 1 << 100000000",
            "5 + 1 << 100000000 & 1",
        ] {
            assert_eq!(
                refused(&ctx, shift).as_deref(),
                Some("too large"),
                "{shift}"
            );
        }
        for fine in [
            "4 << 5",
            "1 << 100000",
            "1 << small",
            "1 << (small + 2)",
            "1 << 2 + 3",
            "(1 << 3) << 4",
            "1 << 3 & 1 << 4",
            "x = 1 << 3",
            "1 << 100000000 m",
            "1 << 1.5",
            "4 >> 100000000",
        ] {
            assert_eq!(refused(&ctx, fine), None, "{fine}");
        }
    }

    /// A count is only trusted if nothing earlier on the line can change it.
    #[test]
    fn a_shift_line_with_a_function_or_a_second_assignment_is_refused() {
        let ctx = ctx_with(&["n = 3"]);
        for exotic in [
            "(n = 100000000; 1 << n)",
            "n = 100000000; 1 << n",
            r"(\k. 1 << k)(100000000)",
            "(k => 1 << k)(100000000)",
            "(n = 100000000) + (1 << n)",
            "1 << 3 == 8",
        ] {
            assert_eq!(
                refused(&ctx, exotic).as_deref(),
                Some("can't shift"),
                "{exotic}"
            );
        }
    }

    #[test]
    fn a_huge_power_is_refused_and_an_ordinary_one_is_not() {
        let ctx = ctx_with(&["big = 400000", "small = 12"]);
        for power in [
            "cm^400000 kg",
            "cm^212,345 CAD",
            "2^1,000,000 m",
            "cm ** 400000 kg",
            "cm^-400000 kg",
            "cm^+400000 kg",
            "cm^big kg",
            "cm^(2 * 200000) kg",
            "2^1e+9",
            "2 ^ 10001",
            "cm^400000 in m",
            "2^(10^100000)",
            "2^3^100000",
            "3 + 2^(1 << 100000000)",
        ] {
            assert_eq!(
                refused(&ctx, power).as_deref(),
                Some("too large"),
                "{power}"
            );
        }
        for fine in [
            "2^1,000",
            "2^10000",
            "m^2",
            "10^-3",
            "2^small",
            "2^(1/2)",
            "2^0.5",
            "e^2",
            "2^pi",
            "(a + b)^2",
            "2 ^ 3 ^ 2",
            "cm^2 kg",
            "5 m^3",
            "10^6 * 2",
            "1e3^2",
            "2^sqrt(4)",
            "2^",
        ] {
            assert_eq!(refused(&ctx, fine), None, "{fine}");
        }
    }

    #[test]
    fn a_count_that_cannot_be_evaluated_gives_fends_own_error() {
        let ctx = ctx_with(&[]);
        assert_eq!(
            refused(&ctx, "1 << missing").as_deref(),
            Some("unknown missing")
        );
        assert_eq!(refused(&ctx, "2 ^ (1 / 0)").as_deref(), Some("÷ by 0"));
    }

    #[test]
    fn the_count_is_cut_where_fend_cuts_it() {
        for (expr, count) in [
            ("1 << 3", " 3"),
            ("1 << 3 + 4 << 5", " 3 + 4 "),
            ("1 << (3 + 4) & 5", " (3 + 4) "),
            ("(1 << 3 + 4) + 2", " 3 + 4"),
            ("1 << 3 xor 5", " 3 "),
            ("1 << 3 in m, 4", " 3 in m, 4"),
            ("1 << 100,000,000 & 1", " 100,000,000 "),
            ("1 << a_xor", " a_xor"),
        ] {
            let at = expr.find("<<").unwrap() + 2;
            assert_eq!(shift_count(expr, at), count, "{expr}");
        }
        for (expr, exponent) in [
            ("2^3", "3"),
            ("2 ^ -3 m", " -3"),
            ("2^(3 + 4) * 5", "(3 + 4)"),
            ("2^f(3) + 1", "f(3)"),
            ("2^1e+5 + 1", "1e+5"),
            ("2^x2 y", "x2"),
            ("cm^212,345 CAD", "212,345"),
            ("2^1,000,000 m", "1,000,000"),
            ("2^3, 4", "3"),
            ("2^", ""),
        ] {
            let at = expr.find('^').unwrap() + 1;
            assert_eq!(power_exponent(expr, at), exponent, "{expr}");
        }
    }

    #[test]
    fn whole_numbers_are_read_in_every_form_fend_shows_them() {
        for (shown, size) in [
            ("5", 5),
            ("-5", 5),
            ("0x5f5e100", 100_000_000),
            ("0b101", 5),
            ("0o17", 15),
            ("3#12", 5),
            ("500%", 5),
            ("99999999999999999999999", u64::MAX),
        ] {
            assert_eq!(whole_number(shown), Some(size), "{shown}");
        }
        for not_whole in ["", "0x", "1.5", "5 m", "abc", "1#0", "99#1", "%"] {
            assert_eq!(whole_number(not_whole), None, "{not_whole}");
        }
    }
}
