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
//! - A unit power over [`MAX_POWER`]. A unit that needs converting, raised
//!   to a large power and then combined with another dimension (`cm^400000
//!   kg`), takes seconds. Allowed powers add up, so the whole line is
//!   counted: `cm^10^5`, `(cm^5000)^20`, `u^20` with `u = cm^5000`, and ten
//!   `cm^9999` multiplied together all make a power of about 100,000.
//!
//! A count can be a variable, a bracketed sum or `100000000 to hex`, so it
//! is evaluated, on a copy of the context, rather than read off as digits.
//! A function whose body calls another one (`twice = \x. f(f(x))`) shows no
//! power of its own, so the functions it calls are looked up and their
//! powers multiplied in: with `f = \x. x^200`, `twice(cm) kg` is
//! `cm^40000 kg`. A function passed in as an argument (`\g. g(g(x))`) can't
//! be looked up, so each call to it counts as the strongest function the
//! line could pass.
//!
//! A function that calls itself recurses until the stack overflows, which
//! ends the process. `crate::document` refuses one that calls itself by
//! name; this refuses one that calls itself without a name (`\x. x(x)`),
//! and a function that calls the last answer (`_`), which can be itself.

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

/// A hex literal or a base from 14 up (`16#1d5`), in which `d` is a digit.
static BASE_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"0[xX][0-9a-fA-F_]*|\b(?:1[4-9]|2[0-9]|3[0-6])#[0-9A-Za-z_]*").unwrap()
});
/// A `d` fend reads as dice: after a digit (`4d6`, also in `0o14d5`) or at
/// the start of a word (`d20`), and followed by a digit.
static DIE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[0-9]|(?:^|[^0-9A-Za-z_.]))d[0-9]").unwrap());

/// A name in a line, `m` in `5m` too.
static NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[^\p{L}\p{N}_])(?P<name>[\p{L}_][\p{L}\p{N}_]*)|\d(?P<unit>[\p{L}_][\p{L}\p{N}_]*)",
    )
    .unwrap()
});

/// A function's argument: `x` in `\x.`, `x:` and `x =>`.
static ARGUMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:\\|\u{3bb})\s*(?P<a>[\p{L}_][\p{L}\p{N}_]*)|(?P<b>[\p{L}_][\p{L}\p{N}_]*)\s*(?::|=>)",
    )
    .unwrap()
});

/// A power in fend's display of a value (`cm^5000`, `s^-2`).
static POWER_SHOWN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^-?(\d+)").unwrap());

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
/// language rewrite), or the error to show instead. A line with two names
/// or a power costs a copy of the context and an evaluation per name (see
/// [`unit_power`]); other lines cost a few regex scans.
pub(super) fn check(ctx: &fend_core::Context, expr: &str) -> Result<(), LineError> {
    let without_literals = BASE_LITERAL.replace_all(expr, " ");
    if DIE.is_match(&without_literals) {
        return Err(LineError::own("dice like 4d6 aren't supported", "no dice"));
    }
    // (where the operator is, where the count starts, which one it is)
    let mut hazards: Vec<(usize, usize, Hazard)> = expr
        .match_indices("<<")
        .map(|(at, _)| (at, at + 2, Hazard::Shift))
        .chain(
            expr.match_indices("**")
                .map(|(at, _)| (at, at + 2, Hazard::Power)),
        )
        .chain(
            expr.match_indices('^')
                .map(|(at, _)| (at, at + 1, Hazard::Power)),
        )
        .collect();
    let after = after_assignment(expr);
    // A probe evaluates a count on a copy of the context, so what the line
    // assigns or defines before the shift must not change that count.
    if hazards.iter().any(|(_, _, h)| matches!(h, Hazard::Shift))
        && after.contains([';', '\\', '\u{3bb}', ':', '='])
    {
        return Err(LineError::own(
            "a line with << can't also have a function, a ; or a second =",
            "can't shift",
        ));
    }
    // Right to left, so a count that holds another shift or power is only
    // evaluated after that one has passed.
    hazards.sort_by_key(|&(_, start, _)| std::cmp::Reverse(start));
    // In a function, a name in an exponent can be its argument, which only
    // has a value once the function is applied.
    let in_function = after.contains(['\\', '\u{3bb}', ':']) || after.contains("=>");
    let arguments = if in_function {
        arguments(after)
    } else {
        Vec::new()
    };
    if in_function && LAST_ANSWER_CALLED.is_match(after) {
        return Err(LineError::own(CALLS_ITSELF, "calls itself"));
    }
    match applied_to_itself(after, &arguments) {
        Some(SelfCall::Bracketed) => return Err(LineError::own(CALLS_ITSELF, "calls itself")),
        Some(SelfCall::Bare) => {
            return Err(LineError::own(
                "write x * x to multiply; x x calls x on itself, which can't be calculated",
                "calls itself",
            ))
        }
        None => {}
    }
    let deadline = Deadline::new(EVAL_TIMEOUT);
    // (where the operator is, the exponent)
    let mut powers = Vec::new();
    for (op, start, hazard) in hazards {
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
        if in_function
            && matches!(hazard, Hazard::Power)
            && NAME
                .captures_iter(count)
                .any(|caps| caps.name("name").is_some())
        {
            if could_hold_a_unit(ctx, base_of(expr, op), &arguments) {
                return Err(LineError::own(
                    "a power of a unit inside a function needs a number, like x^2",
                    "needs a number",
                ));
            }
            // A plain number's power, which fend can interrupt.
            continue;
        }
        if let Some(size) = size_of(ctx, count, what, &deadline)? {
            if size > limit {
                return Err(LineError::own(too_large, "too large"));
            }
            if matches!(hazard, Hazard::Power) {
                powers.push((op, size));
            }
        }
    }
    if unit_power(ctx, expr, &without_literals, &powers, &deadline) > MAX_POWER {
        return Err(LineError::own(POWER_TOO_LARGE, "too large"));
    }
    Ok(())
}

/// Roughly the largest power of a unit `expr` makes, its terms added up:
/// each name counts the units its value holds (`u = cm^5000` is 5,000), a
/// `^` multiplies what its base holds, and a function multiplies the whole
/// line by the power in its body (`f = \x. x^1000`, then `f(cm^1000)`). It
/// only has to be an upper bound.
/// `names` is `expr` without its base literals (`0xd6`), whose letters are
/// digits. `powers` are the operators and exponents [`check`] found. A
/// bracket raised to a power is evaluated last, innermost first, and only
/// while the line is still under [`MAX_POWER`], so its own powers are
/// counted and allowed before fend calculates it.
fn unit_power<'e>(
    ctx: &fend_core::Context,
    expr: &'e str,
    names: &'e str,
    powers: &[(usize, u64)],
    deadline: &Deadline,
) -> u64 {
    let names: Vec<&str> = NAME
        .captures_iter(names)
        .filter_map(|caps| caps.name("name").or_else(|| caps.name("unit")))
        .map(|m| m.as_str())
        .collect();
    if names.len() < 2 && powers.is_empty() {
        return 0;
    }
    let mut probe = ctx.clone();
    let mut known: Vec<(String, Held)> = Vec::new();
    // A function's power only counts once the line holds a unit for it to
    // raise, so the functions it calls are only looked up then.
    let mut total = 0u64;
    let mut functions = Vec::new();
    for name in &names {
        let shown = fend_core::evaluate_with_interrupt(name, &mut probe, deadline)
            .map_or(String::new(), |r| r.get_main_result().to_string());
        match held(&shown) {
            Held::Units(units) => total = total.saturating_add(units),
            Held::Function(_) => functions.push(*name),
        }
    }
    if total == 0 && powers.is_empty() {
        return 0;
    }
    // A function that calls its own argument (`t2 = \g. \x. g(g(x))`) can
    // be handed the strongest function on the line, or one written there
    // (`\x. x^700`), so each such call counts at that function's power.
    let strongest = if functions.is_empty() {
        1
    } else {
        let mut first: Vec<(String, Held)> = Vec::new();
        for name in &functions {
            held_by(name, &mut probe, deadline, &mut first, &mut Vec::new(), 1);
        }
        first
            .iter()
            .filter_map(|(_, held)| match held {
                Held::Function(power) => Some(*power),
                Held::Units(_) => None,
            })
            .chain(powers.iter().map(|&(_, exponent)| exponent))
            .max()
            .unwrap_or(1)
    };
    let mut held_by = |text: &str| {
        held_by(
            text,
            &mut probe,
            deadline,
            &mut known,
            &mut Vec::new(),
            strongest,
        )
    };
    let mut times = 1u64;
    for name in functions {
        if let Held::Function(power) = held_by(name) {
            times = times.saturating_mul(power);
        }
    }
    let mut units_in = |text: &'e str| match held_by(text) {
        Held::Units(units) => units,
        Held::Function(_) => 0,
    };
    let mut bases: Vec<(&str, u64)> = powers
        .iter()
        .filter_map(|&(op, exponent)| Some((base_of(expr, op)?, exponent)))
        .collect();
    // Names before brackets, and a bracket before the one around it.
    bases.sort_by_key(|(base, _)| (base.starts_with('('), base.len()));
    for (base, exponent) in bases {
        if total.saturating_mul(times) > MAX_POWER {
            break;
        }
        total = total.saturating_add(units_in(base).saturating_mul(exponent.saturating_sub(1)));
    }
    total.saturating_mul(times)
}

/// Whether the base of a power in a function could hold a unit: a name in
/// it is one of the function's `arguments`, the last answer (`_`, `ans`),
/// which is whatever the line before the call made, or holds a unit itself.
/// A plain number (`1.05^y`) can't, and fend can interrupt any power of one.
/// A variable that holds a number now could hold a unit when the function
/// is called; the watchdog is all that stops that.
fn could_hold_a_unit(ctx: &fend_core::Context, base: Option<&str>, arguments: &[&str]) -> bool {
    base.is_some_and(|base| {
        NAME.captures_iter(base)
            .filter_map(|caps| caps.name("name").or_else(|| caps.name("unit")))
            .any(|name| {
                arguments.contains(&name.as_str())
                    || matches!(name.as_str(), "_" | "ans")
                    || super::holds_a_unit(ctx, name.as_str())
            })
    })
}

/// The message for a function that calls itself, here and in
/// `crate::document`.
pub(crate) const CALLS_ITSELF: &str =
    "a function can't call itself, directly or through another one";

/// The last answer called as a function (`_(x)`, `ans (x)`).
static LAST_ANSWER_CALLED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^\p{L}\p{N}_])(?:_|ans)\s*\(").unwrap());

/// How a name is applied to itself (see [`applied_to_itself`]).
enum SelfCall {
    /// `x(x)` or `x (x)`.
    Bracketed,
    /// `x x`, which fend also reads as a call; with a number it would be a
    /// product, but nothing tells the two apart before the function runs.
    Bare,
}

/// Whether `expr` applies one of the functions' `arguments`, or the last
/// answer, to itself: `x(x)`, `x (x)` or `x x`. That is how a function
/// calls itself without a name (`(\x. x(x))(\x. x(x))`).
fn applied_to_itself(expr: &str, arguments: &[&str]) -> Option<SelfCall> {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}_][\p{L}\p{N}_]*").unwrap());
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    // `rest` starts with `name` as a whole word.
    let starts_with_word = |rest: &str, name: &str| {
        rest.strip_prefix(name)
            .is_some_and(|after| !after.starts_with(is_word))
    };
    if arguments.is_empty() && !expr.contains('_') && !expr.contains("ans") {
        return None;
    }
    WORD.find_iter(expr).find_map(|word| {
        let name = word.as_str();
        if !(arguments.contains(&name) || matches!(name, "_" | "ans")) {
            return None;
        }
        let rest = &expr[word.end()..];
        let after_space = rest.trim_start();
        match after_space.strip_prefix('(') {
            Some(inner) => {
                let inner = inner.trim_start();
                (starts_with_word(inner, name) && inner[name.len()..].trim_start().starts_with(')'))
                    .then_some(SelfCall::Bracketed)
            }
            None => (after_space.len() < rest.len() && starts_with_word(after_space, name))
                .then_some(SelfCall::Bare),
        }
    })
}

/// How many times `shown`, fend's display of a function, calls one of its
/// own `arguments`: the name, a space, then what it's called with. fend
/// shows `\g. \x. g(g(x))` as `\g.\x.(g (((g ((x))))))`, two calls, and a
/// product as `(x*x)`, none.
fn argument_calls(shown: &str, arguments: &[&str]) -> u32 {
    static CALL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?P<name>[\p{L}_][\p{L}\p{N}_]*) [(\p{L}\p{N}_]").unwrap());
    let calls = CALL
        .captures_iter(shown)
        .filter(|caps| arguments.contains(&&caps["name"]))
        .count();
    u32::try_from(calls).unwrap_or(u32::MAX)
}

/// The arguments of the functions in `text`.
pub(crate) fn arguments(text: &str) -> Vec<&str> {
    ARGUMENT
        .captures_iter(text)
        .filter_map(|caps| caps.name("a").or_else(|| caps.name("b")))
        .map(|m| m.as_str())
        .collect()
}

/// What a value does to the power of a unit, from fend's display of it.
#[derive(Clone, Copy)]
enum Held {
    /// The powers it holds: `1 cm^5000 kg` is 5,001, `5 m` 1, a number 0,
    /// and a converter its base's power.
    Units(u64),
    /// A function (`\x.(x^1000)`) multiplies its argument's power by the
    /// powers in its body, at least 2.
    Function(u64),
}

/// The most functions deep the lookup in [`held_by`] goes: past that,
/// `crate::document` has refused the chain as too nested anyway.
const MAX_CALL_DEPTH: usize = 256;

/// What `text` holds, evaluated on `probe`, remembered in `known`. A
/// function's power is the powers in its own body times those of the
/// functions its body calls, once per call, since a call inside another
/// multiplies: `twice = \x. f(f(x))`, with `f = \x. x^700`, makes 490,000
/// times its argument's power. A call to one of its own arguments counts
/// at `strongest`, the most any function on the line could make.
/// `calling` is the chain being looked up; a function that calls itself, or
/// a chain too long to follow, has no bound.
fn held_by(
    text: &str,
    probe: &mut fend_core::Context,
    deadline: &Deadline,
    known: &mut Vec<(String, Held)>,
    calling: &mut Vec<String>,
    strongest: u64,
) -> Held {
    if let Some(&(_, held)) = known.iter().find(|(t, _)| t == text) {
        return held;
    }
    if calling.iter().any(|c| c == text) || calling.len() >= MAX_CALL_DEPTH {
        return Held::Function(u64::MAX);
    }
    let shown = match fend_core::evaluate_with_interrupt(text, probe, deadline) {
        Ok(result) => result.get_main_result().to_string(),
        Err(_) => String::new(),
    };
    let held = match held(&shown) {
        Held::Function(own) => {
            calling.push(text.to_string());
            let own_arguments = arguments(&shown);
            let calls = argument_calls(&shown, &own_arguments);
            let mut power = own.saturating_mul(strongest.saturating_pow(calls));
            for caps in NAME.captures_iter(&shown) {
                let Some(name) = caps.name("name").or_else(|| caps.name("unit")) else {
                    continue;
                };
                if own_arguments.contains(&name.as_str()) {
                    continue;
                }
                if let Held::Function(called) =
                    held_by(name.as_str(), probe, deadline, known, calling, strongest)
                {
                    power = power.saturating_mul(called);
                }
            }
            calling.pop();
            Held::Function(power)
        }
        units => units,
    };
    known.push((text.to_string(), held));
    held
}

fn held(shown: &str) -> Held {
    let powers = POWER_SHOWN
        .captures_iter(shown)
        .map(|caps| caps[1].parse().unwrap_or(u64::MAX))
        .fold(0, u64::saturating_add);
    if shown.starts_with(['\\', '\u{3bb}']) {
        Held::Function(powers.max(2))
    } else if super::shows_a_unit(shown) {
        Held::Units(super::units_shown(shown, &super::converter_power).max(1))
    } else {
        Held::Units(0)
    }
}

/// What the `^` or `**` at `op` raises: a bracket, or a name (`m` in `5m`).
/// `None` for a plain number, which holds no unit.
fn base_of(expr: &str, op: usize) -> Option<&str> {
    let head = expr[..op].trim_end();
    if head.ends_with(')') {
        let mut depth = 0usize;
        for (i, b) in head.bytes().enumerate().rev() {
            match b {
                b')' => depth += 1,
                b'(' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&head[i..]);
                    }
                }
                _ => {}
            }
        }
        return None;
    }
    let start = head
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let word = head[start..].trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    word.starts_with(|c: char| c.is_alphabetic() || c == '_')
        .then_some(word)
}

/// The line after a leading `name =` (but not `==` or `=>`), or all of it.
pub(super) fn after_assignment(expr: &str) -> &str {
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
/// operand -- a bracket, or a number or name with the call that follows it
/// -- and any power of it, since fend's `^` groups to the right (`10^5` in
/// `cm^10^5`).
fn power_exponent(expr: &str, start: usize) -> &str {
    let rest = &expr[start..];
    let mut end = operand_end(rest, 0);
    loop {
        let after = rest[end..].trim_start();
        let next = rest.len() - after.len();
        let op = if after.starts_with('^') {
            1
        } else if after.starts_with("**") {
            2
        } else {
            return &rest[..end];
        };
        let to = operand_end(rest, next + op);
        if to == next + op {
            return &rest[..end];
        }
        end = to;
    }
}

/// Where the operand that starts at `from` in `rest` ends: signs, then a
/// bracket, or a number or name with the call that follows it. `from` if
/// there is none.
fn operand_end(rest: &str, from: usize) -> usize {
    let bytes = rest.as_bytes();
    let mut i = from;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'-' | b'+') {
        i += 1;
    }
    let operand = i;
    let mut end = i;
    if bytes.get(i) == Some(&b'(') {
        return closing(bytes, i);
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
        return from;
    }
    if bytes.get(end) == Some(&b'(') {
        end = closing(bytes, end);
    }
    end
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
    let shown = super::in_target_base(count, result.get_main_result());
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
            "16#1d5",
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

    /// Powers that are each allowed but make a large one together.
    #[test]
    fn powers_are_added_up_across_the_line() {
        let ctx = ctx_with(&[
            "u = cm^5000",
            "w = cm^9999",
            "x = 5 m",
            "area = 20 m^2",
            r"f = \x. x^1000",
            r"sq = \x. x^2",
        ]);
        for power in [
            "cm^10^5 kg",
            "cm^632^2 kg",
            "cm**10**5 kg",
            "(cm^5000)^20 kg",
            "((cm^100)^100)^100 kg",
            "u^20 kg",
            "w * w kg",
            "w w kg",
            "cm^9999 * cm^9999 * cm^9999 kg",
            "v = u * u * u",
            "f(cm^1000) kg",
            "f(f(cm)) kg",
            "sq(sq(sq(sq(w)))) kg",
        ] {
            assert_eq!(
                refused(&ctx, power).as_deref(),
                Some("too large"),
                "{power}"
            );
        }
        for fine in [
            "u kg",
            "w",
            "cm^9999 kg",
            "x * x",
            "area * 3 m",
            "(x + 1 m)^2",
            "2^10000 * 3^2",
            "10^6 * 2^20 m",
            "0xff^2",
            "w kg",
            "f(2) + f(3)",
            "sq(x) * sq(3 m)",
            "pi^5001 * pi^5001",
        ] {
            assert_eq!(refused(&ctx, fine), None, "{fine}");
        }
    }

    /// A function's argument has no value until it is applied, so it can't
    /// be measured as an exponent of something that could hold a unit. A
    /// plain number's power is one fend can interrupt.
    #[test]
    fn a_power_that_could_hold_a_unit_is_refused_in_a_function() {
        let ctx = ctx_with(&["r = 0.05", "len = 2 m"]);
        for function in [
            r"f = \x. cm^x kg",
            "(k => cm^k kg)(400000)",
            r"g = \x. \y. x^y",
            r"h = \y. (2 m)^y",
            r"k = \y. len^y",
            "y: (1 + y)^y",
            r"growth = \y. _^y",
            r"growth = \y. ans^y",
        ] {
            assert_eq!(
                refused(&ctx, function).as_deref(),
                Some("needs a number"),
                "{function}"
            );
        }
        for fine in [
            r"sq = \x. x^2",
            r"h = \x. x^(1/2)",
            "x: x^0.5",
            r"g = \x. x^1e2",
            r"pow2 = \x. 2^x",
            "x: 2^x",
            r"growth = \y. 1000 * 1.05^y",
            r"compound = \y. 1000 * (1 + r)^y",
            r"wave = \t. e^t",
        ] {
            assert_eq!(refused(&ctx, fine), None, "{fine}");
        }
    }

    /// A function's power counts the functions its body calls, once per
    /// call: `twice` of `x^700` raises its argument to 490,000.
    #[test]
    fn powers_multiply_through_the_functions_a_body_calls() {
        let ctx = ctx_with(&[r"f = \x. x^700", r"twice = \x. f(f(x))", r"g = \x. x^2"]);
        assert_eq!(refused(&ctx, "twice(cm) kg").as_deref(), Some("too large"));
        assert_eq!(refused(&ctx, "twice(cm)").as_deref(), Some("too large"));
        let ctx = ctx_with(&[r"f = \x. x^2", r"twice = \x. f(f(x))"]);
        assert_eq!(refused(&ctx, "twice(cm) kg"), None);
    }

    /// A function that calls itself without a name recurses until the
    /// stack overflows.
    /// A function that calls its own argument can be handed the strongest
    /// function on the line, or one written there; calls that only use the
    /// argument (`x * x`) don't count.
    #[test]
    fn a_call_to_a_functions_argument_counts_the_strongest_function() {
        let ctx = ctx_with(&[r"f = \x. x^700", r"t2 = \g. \x. g(g(x))", r"sq = \x. x * x"]);
        assert_eq!(refused(&ctx, "t2(f)(cm) kg").as_deref(), Some("too large"));
        assert_eq!(
            refused(&ctx, r"t2(\x. x^700)(cm) kg").as_deref(),
            Some("too large")
        );
        assert_eq!(refused(&ctx, "t2(sq)(cm) kg"), None);
        assert_eq!(refused(&ctx, "sq(cm) kg"), None);
    }

    /// `x x` is refused as a call of `x` on itself, with a hint for the
    /// product it may have meant.
    #[test]
    fn a_bare_self_call_suggests_a_product() {
        let ctx = ctx_with(&[]);
        let error = check(&ctx, r"sq = \x. x x").unwrap_err();
        assert_eq!(error.short(), "calls itself");
        assert!(error.to_string().contains("write x * x"), "{error}");
        let error = check(&ctx, r"w = \x. x(x)").unwrap_err();
        assert!(!error.to_string().contains("x * x"), "{error}");
    }

    #[test]
    fn a_function_applied_to_itself_is_refused() {
        let ctx = ctx_with(&[]);
        for expr in [
            r"w = \x. x(x)",
            r"(\x. x(x))(\x. x(x))",
            r"w = \x. x (x)",
            r"w = \f. (\x. f(x(x)))(\x. f(x(x)))",
            "_(_)",
            r"w = \x. _(x)",
            r"w = \x. ans (x)",
        ] {
            assert_eq!(
                refused(&ctx, expr).as_deref(),
                Some("calls itself"),
                "{expr}"
            );
        }
        for fine in [
            r"sq = \x. x * x",
            r"t2 = \g. \x. g(g(x))",
            "_ + 1",
            "x_1(2)",
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
            ("cm^10^5 kg", "10^5"),
            ("2 ^ 3 ^ 2", " 3 ^ 2"),
            ("2^(2)^3", "(2)^3"),
            ("2^3^", "3"),
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
