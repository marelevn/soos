//! The rewrites that turn a line in the words people write into one fend
//! parses: operator words, percent phrases, currency symbols and units.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use super::dates::{clock_time, CLOCK};
use crate::format::CURRENCY_STYLES;

/// The words that introduce a conversion (`5 cm in inches`), for the
/// patterns here and in [`crate::engine`] that look for one. A time-zone
/// line takes only `in` and `to`, and [`crate::highlight`] keeps a list of
/// its own, with `of`, `on`, `off`, `before` and `after`.
pub(crate) const CONVERT_KW: &str = "in|to|as";

/// `into`, which fend lacks. `pub(crate)` with [`OPERATOR_WORD`] so
/// [`crate::highlight`] colours the words that are rewritten.
pub(crate) static INTO_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\binto\b").unwrap());

/// The words for `+` and `-`, which the date lines also read before
/// [`rewrite`] does.
pub(super) const ADD_WORDS: &str = "plus|with|and";
pub(super) const SUBTRACT_WORDS: &str = "minus|subtract|without";

/// A word for an operator. `with` and `without` are told apart by the
/// closing `\b`, which `with` fails inside `without`.
pub(crate) static OPERATOR_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        concat!(
            r"\b(?:(?P<add>{ADD_WORDS})|(?P<sub>{SUBTRACT_WORDS})",
            r"|(?P<mul>times|multiplied\s+by|mul)|(?P<div>divided\s+by|divide\s+by|divide))\b",
        ),
        ADD_WORDS = ADD_WORDS,
        SUBTRACT_WORDS = SUBTRACT_WORDS,
    ))
    .unwrap()
});

/// An amount with a symbol before it (`$840`, `A$1,234.56`) or after it
/// (`26125 ₫`), for every symbol in [`CURRENCY_STYLES`], so each result Soos
/// shows reads back in. fend reads `,` between digits as a thousands
/// separator, so the number keeps them.
static SYMBOL_BEFORE: LazyLock<Regex> = LazyLock::new(|| symbol_regex(true));

static SYMBOL_AFTER: LazyLock<Regex> = LazyLock::new(|| symbol_regex(false));

/// A bare symbol as the target of a conversion (`5 EUR in $`, `$1 in ₫`).
/// fend knows `$` and `€` only as units of their own, which skip Soos's
/// rounding, and `₫` not at all, so the target becomes the currency's code.
static SYMBOL_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?P<kw>\b(?i:into|{CONVERT_KW})\s+)(?P<sym>{})(?P<post>$|[^\p{{L}}\p{{N}}_])",
        symbols(false)
    ))
    .unwrap()
});

static TEA_SPOON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btea\s+spoon(s?)\b").unwrap());

/// fend names the inverse trig functions `asin`, `acos` and `atan`.
static ARC_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\barc(?P<f>sin|cos|tan)(?P<h>h?)\b").unwrap());

/// `root 3` and `cbrt`: an odd root, which is real for a negative number.
/// `post` is what follows the index, so `root 3.5` is left alone.
static ROOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:root\s+(?P<n>\d+)(?P<post>[^\d.]|$)|(?P<cbrt>cbrt)\b\s*)").unwrap()
});

/// `sq cm` and `cu ft`: fend only knows `square` and `cubic`.
static SQ_CU: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?P<k>sq|cu)\s+(?P<u>[A-Za-z]+)\b").unwrap());

/// `6 m / 2 s`: a unit on the left of `/`, then a number and a unit. fend
/// reads that as `(6 m / 2) s`, a silent `3 m s`, and `$40 / 2 hours` as
/// `20 USD hours`. The quantity is grouped so the unit divides too. A number
/// on the left (`1/2 cup`, `10 / 2 m`) keeps fend's reading. `next` is the
/// `(` of a function (`x / 2 sqrt(4)`), which isn't a unit.
static DIVIDED_BY_A_QUANTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?P<left>\p{L}(?:\^-?\d+)?\)*\s*)/(?P<gap>\s*)",
        r"(?P<qty>\d[\d,]*(?:\.\d+)?(?:[eE][+-]?\d+)?\s*",
        r"(?P<unit>[\p{L}\u{b0}\u{b5}][\p{L}\p{N}_\u{b0}\u{b5}]*(?:\^-?\d+)?(?:/\p{L}+(?:\^-?\d+)?)?))",
        r"(?P<next>\(?)",
    ))
    .unwrap()
});

/// Words that follow a number without being its unit.
const NOT_A_UNIT: &[&str] = &["in", "to", "as", "of", "on", "off", "into", "mod", "xor"];

/// `5 300` or `1 000 000`: a number written with spaces between its
/// thousands, which fend rejects. A group must be exactly three digits, so
/// `5 3000` stays an error. The `pre` and `post` characters stand in for
/// the lookaround `regex` doesn't have.
static SPACED_DIGITS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<pre>^|[^\p{L}\p{N}_.,])(?P<num>\d{1,3}(?: \d{3})+)(?P<post>$|[^\p{L}\p{N}_,])")
        .unwrap()
});

static PERCENT_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+on\s+(?P<x>.+)$").unwrap());

/// `fee on price`, for a variable holding a percent (`fee = 8%`). fend's
/// `of` only takes a literal `N%`, so this becomes `price + fee * price`,
/// and [`crate::engine`] checks that `fee` really is a percent first.
static VAR_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>[A-Za-z_][A-Za-z0-9_]*)\s+on\s+(?P<x>.+)$").unwrap());

static PERCENT_OFF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<p>.+?)%\s+off\s+(?P<x>.+)$").unwrap());

static PERCENT_AS_OF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?P<x>.+?)\s+as\s+a\s*%\s+of\s+(?P<y>.+)$").unwrap());

/// `P% of what is X`: the whole that P% of is X. `on` and `off` are the
/// inverses of `P% on X` and `P% off X`: the price that P% on top, or P% off,
/// makes X.
static PERCENT_OF_WHAT_IS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?P<p>.+?)%\s+(?P<how>of|on|off)\s+what\s+is\s+(?P<x>.+)$").unwrap()
});

/// `name = rhs`, kept out of a percent phrase, which would otherwise take
/// the `name =` into its percent: `price = 15% off $100` assigns the result.
/// `==` is a comparison.
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<lhs>[A-Za-z_][A-Za-z0-9_]*\s*=\s*)(?P<rhs>[^=\s].*)$").unwrap()
});

/// `sin 30 deg`: fend applies the function to the bare number and the unit
/// to the result, `(sin 30 rad) deg` -- a silent -0.988. The angle goes
/// inside, as in `sin(30 deg)`.
static TRIG_WITH_UNIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?P<f>sin|cos|tan)\s+(?P<n>-?\d+(?:\.\d+)?)\s*",
        r"(?P<u>\u{b0}|(?:deg|degs|degrees?|rad|rads|radians?)\b)",
    ))
    .unwrap()
});

/// What may follow a length for its `in` to be the unit and not a
/// conversion: the end, an operator, or a conversion of its own
/// (`20 in in cm`).
fn after_a_length() -> String {
    format!(r"(?P<after>\s*(?:$|[-+*/)\u{{d7}}\u{{f7}}]|(?:{CONVERT_KW})\s))")
}

/// `5 ft 11 in` or `5 ft 11`: fend reads a trailing `in` as a conversion,
/// and takes `5 ft 11` as feet and inches only at the very end of a line.
static FEET_AND_INCHES: LazyLock<Regex> = LazyLock::new(|| {
    let after = after_a_length();
    Regex::new(&format!(
        r"(?i)\b(?P<ft>\d+(?:\.\d+)?)\s*(?:ft|feet|foot)\s+(?P<in>\d+(?:\.\d+)?)(?:\s*(?:inches|inch|in|\x22))?{after}"
    ))
    .unwrap()
});

/// `5 cm to in`: a conversion whose target is the unit inches.
static IN_AS_TARGET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\b(?P<kw>{CONVERT_KW})\s+in\s*$")).unwrap());

/// `3 in` on its own: inches, where fend expects a conversion target.
static BARE_INCHES: LazyLock<Regex> = LazyLock::new(|| {
    let after = after_a_length();
    Regex::new(&format!(
        r"(?P<n>(?:^|[^\w.])\d[\d,]*(?:\.\d+)?)\s*in\b{after}"
    ))
    .unwrap()
});

/// A literal percent, for [`percent_factors`].
static PERCENT_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<p>\d+(?:\.\d+)?)\s*%").unwrap());

/// One clock time taken from another, anywhere in a line. By the time
/// [`rewrite`] looks, the operator words have become `-`.
static CLOCK_DIFFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\b(?P<a>{CLOCK})\s*-\s*(?P<b>{CLOCK})\b")).unwrap());

/// The currency symbols as alternatives in a regex, longest first, so `A$`
/// wins over `$`. `before`: only those written before the amount.
fn symbols(before: bool) -> String {
    let mut symbols: Vec<&str> = CURRENCY_STYLES
        .iter()
        .filter(|s| s.symbol_first || !before)
        .map(|s| s.symbol)
        .collect();
    symbols.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let symbols: Vec<String> = symbols.into_iter().map(regex::escape).collect();
    symbols.join("|")
}

fn symbol_regex(before: bool) -> Regex {
    let symbols = symbols(before);
    // fend's own `k` and `M` scale suffixes (`$50k`); any other letter after
    // the number is a unit (`5m` is metres).
    let number = r"\d[\d,]*(?:\.\d+)?(?:[eE][+-]?\d+)?(?:[kM]\b)?";
    let pattern = if before {
        // `sign` is a minus between symbol and number (`$-5`); `tail` the
        // letter that makes the number a unit's (`$5m`).
        format!(
            r"(?P<pre>^|[^\p{{L}}\p{{N}}_])(?P<sym>{symbols})(?P<sign>\s?-?)(?P<num>{number})(?P<tail>[\p{{L}}_]?)"
        )
    } else {
        format!(r"(?P<num>{number})\s?(?P<sym>{symbols})(?P<post>$|[^\p{{L}}\p{{N}}_])")
    };
    Regex::new(&pattern).unwrap()
}

fn currency_code(symbol: &str) -> &'static str {
    CURRENCY_STYLES
        .iter()
        .find(|s| s.symbol == symbol)
        .map_or("", |s| s.code)
}

/// An expression rewritten into a form fend parses.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rewritten {
    pub expr: String,
    /// The variable in `var on X`, which must hold a percent.
    pub percent_var: Option<String>,
}

/// Rewrite the natural-language phrasing fend doesn't parse: one percent
/// phrase in each bracket (see [`percent_in_brackets`]) and one outside
/// them. `holds_a_unit` tells a unit from a variable or constant that holds
/// a plain number (see [`group_divisors`]).
pub(crate) fn rewrite(
    expr: &str,
    holds_a_unit: &dyn Fn(&str) -> bool,
    is_percent: &dyn Fn(&str) -> bool,
) -> Rewritten {
    // Before the currency symbols, so `$5 300` is one amount.
    let expr = SPACED_DIGITS.replace_all(expr, |caps: &Captures| {
        format!(
            "{}{}{}",
            &caps["pre"],
            caps["num"].replace(' ', ""),
            &caps["post"]
        )
    });
    // An amount is one operand: `$1200 / $40` is 30, not `30 USD^2`, and
    // `$5^2` is 25 USD^2. A letter after the number (`$5m`) leaves it bare,
    // so it stays the unknown unit it is.
    let expr = SYMBOL_BEFORE.replace_all(&expr, |caps: &Captures| {
        let amount = format!(
            "{}{} {}",
            caps["sign"].trim(),
            &caps["num"],
            currency_code(&caps["sym"])
        );
        if caps["tail"].is_empty() {
            format!("{}({amount})", &caps["pre"])
        } else {
            format!("{}{amount}{}", &caps["pre"], &caps["tail"])
        }
    });
    let expr = SYMBOL_AFTER.replace_all(&expr, |caps: &Captures| {
        format!(
            "({} {}){}",
            &caps["num"],
            currency_code(&caps["sym"]),
            &caps["post"]
        )
    });
    let expr = SYMBOL_TARGET
        .replace_all(&expr, |caps: &Captures| {
            format!(
                "{}{}{}",
                &caps["kw"],
                currency_code(&caps["sym"]),
                &caps["post"]
            )
        })
        .into_owned();
    let expr = INTO_WORD.replace_all(&expr, "to").into_owned();
    let expr = OPERATOR_WORD
        .replace_all(&expr, |caps: &Captures| {
            if caps.name("add").is_some() {
                "+"
            } else if caps.name("sub").is_some() {
                "-"
            } else if caps.name("mul").is_some() {
                "*"
            } else {
                "/"
            }
        })
        .into_owned();
    let expr = TEA_SPOON.replace_all(&expr, "teaspoon$1").into_owned();
    let expr = ARC_FUNCTION.replace_all(&expr, "a${f}${h}").into_owned();
    let expr = SQ_CU
        .replace_all(&expr, |caps: &Captures| {
            let kind = if &caps["k"] == "sq" {
                "square"
            } else {
                "cubic"
            };
            // `sq in` is square inches, not a conversion.
            let unit = if &caps["u"] == "in" {
                "inch"
            } else {
                &caps["u"]
            };
            format!("{kind} {unit}")
        })
        .into_owned();
    // After the two-word unit rewrites: `/ 5 sq m` groups as `/ (5 square) m`,
    // which fend reads as 5 m^2.
    let expr = group_divisors(&expr, holds_a_unit);
    let expr = ROOT
        .replace_all(&expr, |caps: &Captures| {
            if caps.name("cbrt").is_some() {
                return "__soos_real_root 3 ".to_string();
            }
            let odd = caps["n"].bytes().next_back().is_some_and(|d| d % 2 == 1);
            let name = if odd { "__soos_real_root" } else { "root" };
            format!("{name} {}{}", &caps["n"], &caps["post"])
        })
        .into_owned();
    let expr = TRIG_WITH_UNIT
        .replace_all(&expr, "${f}(${n} ${u})")
        .into_owned();
    let expr = IN_AS_TARGET.replace(&expr, "${kw} inch").into_owned();
    let expr = FEET_AND_INCHES
        .replace_all(&expr, "(${ft} ft + ${in} inch)${after}")
        .into_owned();
    let expr = BARE_INCHES
        .replace_all(&expr, "${n} inch${after}")
        .into_owned();
    let expr = CLOCK_DIFFERENCE
        .replace_all(&expr, |caps: &Captures| {
            match (clock_time(&caps["a"]), clock_time(&caps["b"])) {
                (Some(a), Some(b)) => {
                    let minutes = (a - b).num_minutes();
                    // Whole hours read as hours; the rest stay in minutes.
                    if minutes % 60 == 0 {
                        format!("({} hours)", minutes / 60)
                    } else {
                        format!("({minutes} minutes)")
                    }
                }
                _ => caps[0].to_string(),
            }
        })
        .into_owned();
    let (assign, rhs) = match ASSIGNMENT.captures(&expr) {
        Some(caps) => (caps["lhs"].to_string(), caps["rhs"].to_string()),
        None => (String::new(), expr.clone()),
    };
    // A phrase in brackets that hold the whole line is the phrase:
    // `(fee on 100)`. Unwrapped before the brackets inside are rewritten, so
    // `(10 as a % of 40)` is read as the line it is, and passes on its
    // number (`prev + 1` is 26), not a fraction. A line that isn't a phrase
    // keeps its brackets (see the end).
    let rhs = bracket_chained_phrases(&rhs);
    let mut phrase = rhs.clone();
    while let Some(inner) = unbracketed(&phrase) {
        phrase = inner.to_string();
    }
    let phrase = percent_in_brackets(&phrase);
    let done = |rewritten: String| Rewritten {
        expr: format!("{assign}{}", percent_of_the_left(&rewritten, is_percent)),
        percent_var: None,
    };

    // A percent, as if typed: `10 as a % of 40` is 25%, and `prev + 1` 125%.
    if let Some(caps) = PERCENT_AS_OF.captures(&phrase) {
        let (x, y) = (caps["x"].trim(), caps["y"].trim());
        if whole_part(x) && whole_part(y) {
            return done(format!("(({x}) / ({y}) * 100)%"));
        }
    }
    if let Some(rewritten) = percent_phrase(&phrase) {
        return done(rewritten);
    }
    if let Some(caps) = VAR_ON.captures(&phrase) {
        let (p, x) = (caps["p"].trim(), caps["x"].trim());
        if whole_part(x) {
            return Rewritten {
                percent_var: Some(p.to_string()),
                ..done(format!("({x}) + ({p}) * ({x})"))
            };
        }
    }
    Rewritten {
        expr: format!(
            "{assign}{}",
            percent_factors(&percent_of_the_left(&percent_in_brackets(&rhs), is_percent))
        ),
        percent_var: None,
    }
}

/// `P% of/on/off what is X`, `P% on X` or `P% off X` in fend's terms, or
/// `None` if `phrase` isn't one of them, or a part of it isn't whole (see
/// [`whole_part`]).
fn percent_phrase(phrase: &str) -> Option<String> {
    if let Some(caps) = PERCENT_OF_WHAT_IS.captures(phrase) {
        let (p, x) = (caps["p"].trim(), caps["x"].trim());
        let whole = match caps["how"].to_ascii_lowercase().as_str() {
            "on" => format!("1 + ({p})/100"),
            "off" => format!("1 - ({p})/100"),
            _ => format!("({p})/100"),
        };
        return (whole_part(p) && whole_part(x)).then(|| format!("({x}) / ({whole})"));
    }
    let (caps, sign) = PERCENT_ON
        .captures(phrase)
        .map(|caps| (caps, "+"))
        .or_else(|| PERCENT_OFF.captures(phrase).map(|caps| (caps, "-")))?;
    let (p, x) = (caps["p"].trim(), caps["x"].trim());
    (whole_part(p) && whole_part(x)).then(|| format!("({x}) {sign} ({p})% of ({x})"))
}

/// Whether a percent phrase's part can be used as it is. Its brackets must
/// match: a phrase that starts inside a bracket ends there, which
/// [`percent_in_brackets`] handles. And it must hold no second phrase:
/// [`bracket_chained_phrases`] brackets two in a row first, so one that's
/// left couldn't be read.
fn whole_part(part: &str) -> bool {
    static SECOND_PHRASE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)%\s+(?:on|off)\s").unwrap());
    let mut depth = 0usize;
    for b in part.bytes() {
        match b {
            b'(' => depth += 1,
            b')' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            _ => {}
        }
    }
    depth == 0 && !SECOND_PHRASE.is_match(part)
}

/// Two percent phrases in a row, read right to left: the rest of the
/// bracket (or line) after an `on` or `off` is bracketed when it holds
/// another phrase, so `5% on 20% off 100` is `5% on (20% off 100)`, 84.
fn bracket_chained_phrases(expr: &str) -> String {
    static ON_OR_OFF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)%\s+(?:on|off)\s+").unwrap());
    static ANOTHER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)%\s+(?:on|off|of\s+what\s+is)\s").unwrap());
    // Where the bracket that `start` is in closes, or the end.
    let closing = |text: &str, start: usize| {
        let mut depth = 0usize;
        for (i, b) in text.bytes().enumerate().skip(start) {
            match b {
                b'(' => depth += 1,
                b')' if depth == 0 => return i,
                b')' => depth -= 1,
                _ => {}
            }
        }
        text.len()
    };
    // Whether `rest` holds another phrase outside its own brackets.
    let holds_another = |rest: &str| {
        ANOTHER.find_iter(rest).any(|m| {
            let before = &rest[..m.start()];
            before.matches('(').count() == before.matches(')').count()
        })
    };
    let mut out = expr.to_string();
    let mut from = 0;
    while let Some(phrase) = ON_OR_OFF.find_at(&out, from) {
        let start = phrase.end();
        let end = closing(&out, start);
        if holds_another(&out[start..end]) {
            out.insert(end, ')');
            out.insert(start, '(');
        }
        from = start;
    }
    out
}

/// What is inside `text` when one pair of brackets holds all of it:
/// `10 as a % of 40` in `(10 as a % of 40)`, but nothing in `(1) + (2)`.
fn unbracketed(text: &str) -> Option<&str> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    whole_part(inner).then_some(inner)
}

/// [`percent_phrase`] inside each bracket that holds one, inner brackets
/// first, so the phrase takes the rest of its bracket rather than of the
/// line: `(20% off 100) + 20` is 100. A rewrite repeats `X`, so brackets
/// nested in `X` double the line each time; past the longest line the
/// engine takes it stops, and the line is refused as too long.
fn percent_in_brackets(expr: &str) -> String {
    let mut out = expr.to_string();
    'scan: while out.len() <= crate::engine::MAX_EXPR_LEN {
        let mut opens = Vec::new();
        for (i, b) in out.bytes().enumerate() {
            match b {
                b'(' => opens.push(i),
                b')' => {
                    let Some(open) = opens.pop() else { continue };
                    let inner = &out[open + 1..i];
                    let rewritten = percent_phrase(inner).or_else(|| {
                        // A fraction, like any percent in brackets:
                        // `(10 as a % of 40) * 2` is 0.5.
                        let caps = PERCENT_AS_OF.captures(inner)?;
                        let (x, y) = (caps["x"].trim(), caps["y"].trim());
                        (whole_part(x) && whole_part(y)).then(|| format!("({x}) / ({y})"))
                    });
                    if let Some(rewritten) = rewritten {
                        out.replace_range(open + 1..i, &rewritten);
                        continue 'scan;
                    }
                }
                _ => {}
            }
        }
        break;
    }
    out
}

/// A percent typed right after `+` or `-` is that percent of everything to
/// its left, in its bracket or on the line, as on a desk calculator:
/// `30 + 5%` is 31.5, `$100 - 15%` is $85 and `10 + 20 + 5%` is 31.5. Only
/// a typed percent: one in brackets or a variable is the plain fraction
/// fend makes it. A percent in a product (`10 + 5% * 2`) is left to
/// [`percent_factors`], and one added to a percent (`5% + 3%`, or `fee + 3%`
/// with `fee = 5%`: `is_percent` says what the left side holds) to fend,
/// which adds them.
fn percent_of_the_left(expr: &str, is_percent: &dyn Fn(&str) -> bool) -> String {
    static PERCENT_AFTER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?P<op>[+-])\s*(?P<p>\d*\.?\d+)\s*%").unwrap());
    /// What may follow the percent: the end of its term.
    static TERM_ENDS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"^(?:$|[-+),;]|(?:{CONVERT_KW}|into)\b)")).unwrap());
    let mut out = String::with_capacity(expr.len());
    // Where the current bracket, argument or line starts in `out`.
    let mut starts = vec![0];
    let mut chars = expr.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if let Some(caps) = PERCENT_AFTER
            .captures(&expr[i..])
            .filter(|caps| TERM_ENDS.is_match(expr[i + caps[0].len()..].trim_start()))
        {
            let start = *starts.last().unwrap();
            let left = out[start..].trim();
            // A sign (`2 * -5%`) or an exponent's (`1e-5%`) isn't a `-` of
            // its own.
            let after_an_operand = left.chars().next_back().is_some_and(|c| {
                c.is_alphanumeric() || matches!(c, ')' | '%' | '_' | '.' | '\u{b0}' | '\'' | '!')
            }) && !(out.ends_with(['e', 'E'])
                && out[..out.len() - 1].ends_with(|c: char| c.is_ascii_digit()));
            if after_an_operand && !is_percent(left) {
                let percent_of_left = format!("({left}) * (1 {} {}/100)", &caps["op"], &caps["p"]);
                out.truncate(start);
                out.push_str(&percent_of_left);
                while chars.next_if(|&(at, _)| at < i + caps[0].len()).is_some() {}
                continue;
            }
        }
        out.push(c);
        match c {
            '(' => starts.push(out.len()),
            ')' if starts.len() > 1 => {
                starts.pop();
            }
            // Not a thousands separator (`1,000`).
            ',' if chars.peek().is_none_or(|&(_, next)| !next.is_ascii_digit()) => {
                *starts.last_mut().unwrap() = out.len();
            }
            ';' => *starts.last_mut().unwrap() = out.len(),
            _ => {}
        }
    }
    out
}

/// `expr` cut at its first `in`, `to` or `as` outside parentheses: the sum
/// and the conversion after it (with its leading space).
pub(crate) fn split_conversion(expr: &str) -> (&str, &str) {
    static CONVERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"\s(?:{CONVERT_KW})\s")).unwrap());
    for found in CONVERSION.find_iter(expr) {
        let before = &expr[..found.start()];
        if before.matches('(').count() == before.matches(')').count() {
            return expr.split_at(found.start());
        }
    }
    (expr, "")
}

/// [`DIVIDED_BY_A_QUANTITY`] on every `/` in `expr`. A match uses up the
/// letter that the next one's left side needs (`a / 2 m / 3 s`), so it is
/// applied again until nothing changes; three passes cover any chain. The
/// name on the left must hold a unit: with `x = 10`, `x / 2 m` is 5 m, as
/// `10 / 2 m` is, and `pi / 2 rad` keeps fend's reading too.
fn group_divisors(expr: &str, holds_a_unit: &dyn Fn(&str) -> bool) -> String {
    let mut current = expr.to_string();
    for _ in 0..3 {
        let next = DIVIDED_BY_A_QUANTITY.replace_all(&current, |caps: &Captures| {
            let first_word: String = caps["unit"]
                .chars()
                .take_while(|c| c.is_alphabetic())
                .collect();
            let left = caps.name("left").unwrap();
            let after_letter = left.start() + left.as_str().chars().next().unwrap().len_utf8();
            let name_start = current[..left.start()]
                .char_indices()
                .rev()
                .take_while(|&(_, c)| c.is_alphanumeric() || c == '_')
                .last()
                .map_or(left.start(), |(i, _)| i);
            if !caps["next"].is_empty()
                || NOT_A_UNIT.contains(&first_word.to_lowercase().as_str())
                || !holds_a_unit(&current[name_start..after_letter])
            {
                return caps[0].to_string();
            }
            format!("{}/{}({})", &caps["left"], &caps["gap"], &caps["qty"])
        });
        if next == current {
            break;
        }
        current = next.into_owned();
    }
    current
}

/// A literal percent multiplying, dividing or raising a plain value becomes
/// a fraction: `100 * 15%` is 15, `100 / 20%` is 500 and `4 ^ 50%` is 2.
/// fend applies `%` to the whole product, quotient or power, so it shows
/// `1500%` for the first and a wrong `5%` and `1600%` for the others. A
/// percent times a percent (`50% * 50%`), a percent divided by a number
/// (`20% / 2`) and a variable holding a percent (`2 * fee`) stay percents,
/// and `10 % 3` is modulo, not a percent.
fn percent_factors(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut copied = 0;
    for caps in PERCENT_LITERAL.captures_iter(expr) {
        let whole = caps.get(0).unwrap();
        let before = &expr[..whole.start()];
        let after = expr[whole.end()..].trim_start();
        let inside_a_token = before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '_');
        let is_percent = after.is_empty()
            || after.starts_with(['+', '-', '*', '/', ')', ',', '^', '\u{d7}', '\u{f7}']);
        if inside_a_token || !is_percent {
            continue;
        }
        let before = before.trim_end();
        let divides = before.ends_with(['/', '\u{f7}', '^']) || before.ends_with("**");
        let times_a_value = before
            .strip_suffix(['*', '\u{d7}'])
            .is_some_and(|left| !left.trim_end().ends_with(['%', '*']));
        let times_by_a_value = after.strip_prefix(['*', '\u{d7}']).is_some_and(|right| {
            let right = right.trim_start();
            !right.starts_with('*') && PERCENT_LITERAL.find(right).is_none_or(|m| m.start() != 0)
        });
        if divides || times_a_value || times_by_a_value {
            out.push_str(&expr[copied..whole.start()]);
            out.push_str(&format!("({}/100)", &caps["p"]));
            copied = whole.end();
        }
    }
    out.push_str(&expr[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`super::rewrite`] with every name read as a unit, and only a typed
    /// percent read as one.
    fn rewrite(expr: &str) -> Rewritten {
        super::rewrite(expr, &|_| true, &|left| {
            left.replace(['(', ')'], "").trim_end().ends_with('%')
        })
    }

    /// The rewrites are regex passes over the whole line, so each one is a
    /// chance to change maths fend already reads. Lines with none of Soos's
    /// words in them must come out as they went in.
    #[test]
    fn rewrite_leaves_plain_fend_maths_alone() {
        for expr in [
            "1 + 1",
            "2 * (3 + 4)",
            "sqrt(16)",
            "2^10",
            "10 % 3",
            "10 mod 3",
            "12 / 4",
            "1e3 + 1",
            "x = 5",
            "a = 1 + 2",
            "5 m + 3 m",
            "3 kg * 2",
            "2 ft + 3 ft",
            "100 km to miles",
            "0xff + 1",
            "sin(1) + cos(1)",
            "6(3)",
            "-5 + 3",
        ] {
            assert_eq!(rewrite(expr).expr, expr, "{expr}");
        }
    }

    #[test]
    fn rewrite_into_as_to() {
        assert_eq!(rewrite("20 inches into cm").expr, "20 inches to cm");
    }

    /// A phrase in brackets ends at its bracket.
    #[test]
    fn rewrite_percent_on_off_in_brackets() {
        assert_eq!(
            rewrite("(20% off 100) + 20").expr,
            "((100) - (20)% of (100)) + 20"
        );
        assert_eq!(
            rewrite("20% off (5% on 100)").expr,
            "(((100) + (5)% of (100))) - (20)% of (((100) + (5)% of (100)))"
        );
    }

    /// Two phrases in a row read right to left, as if bracketed.
    #[test]
    fn chained_percent_phrases_are_bracketed() {
        assert_eq!(
            bracket_chained_phrases("5% on 20% off 100"),
            "5% on (20% off 100)"
        );
        assert_eq!(
            bracket_chained_phrases("10% off 5% on 20% off 100"),
            "10% off (5% on (20% off 100))"
        );
        assert_eq!(
            bracket_chained_phrases("(5% on 20% off 100) + 1"),
            "(5% on (20% off 100)) + 1"
        );
        for single in ["5% on 100", "5% on (20% off 100)", "20% off 100 + 20"] {
            assert_eq!(bracket_chained_phrases(single), single);
        }
    }

    #[test]
    fn rewrite_percent_on_off() {
        assert_eq!(rewrite("5% on 30").expr, "(30) + (5)% of (30)");
        assert_eq!(rewrite("6% off 40 EUR").expr, "(40 EUR) - (6)% of (40 EUR)");
    }

    /// A typed percent after `+` or `-` is that percent of what is left of
    /// it, in its bracket or on the line. A sign, an exponent, a modulo and
    /// a percent added to percents stay as typed.
    #[test]
    fn rewrite_takes_a_percent_after_plus_or_minus_of_the_left() {
        for (expr, rewritten) in [
            ("100 - 15%", "(100) * (1 - 15/100)"),
            ("50+10%", "(50) * (1 + 10/100)"),
            ("100 + 5% + 5%", "((100) * (1 + 5/100)) * (1 + 5/100)"),
            ("(100 - 5%) * 2", "((100) * (1 - 5/100)) * 2"),
            ("$100 - 15%", "((100 USD)) * (1 - 15/100)"),
            ("x = 10 + 20 + 5%", "x = (10 + 20) * (1 + 5/100)"),
            ("100 m - 15% in cm", "(100 m) * (1 - 15/100) in cm"),
            ("30 + 5% - 2", "(30) * (1 + 5/100) - 2"),
            ("1,000 + 10%", "(1,000) * (1 + 10/100)"),
            ("100 + .5%", "(100) * (1 + .5/100)"),
            ("90\u{b0} + 5%", "(90\u{b0}) * (1 + 5/100)"),
            ("5! + 10%", "(5!) * (1 + 10/100)"),
        ] {
            assert_eq!(rewrite(expr).expr, rewritten, "{expr}");
        }
        for expr in [
            "20% + 10%",
            "2 * -5%",
            "1e-5%",
            "-5%",
            "10 % 3",
            "x = -5%",
            "30 + 5% of 200",
            "100 + (15%)",
        ] {
            assert_eq!(rewrite(expr).expr, expr);
        }
        // A percent in a product is a fraction of that product.
        assert_eq!(rewrite("10 + 5% * 2").expr, "10 + (5/100) * 2");
    }

    #[test]
    fn rewrite_percent_phrases_keep_an_assignment_in_front() {
        assert_eq!(
            rewrite("price = 15% off $100").expr,
            "price = ((100 USD)) - (15)% of ((100 USD))"
        );
        assert_eq!(rewrite("tip = 5% on 30").expr, "tip = (30) + (5)% of (30)");
        let share = rewrite("share = 30 as a % of 120");
        assert_eq!(share.expr, "share = ((30) / (120) * 100)%");
    }

    /// fend reads `sin 30 deg` as `(sin 30) deg`.
    #[test]
    fn rewrite_puts_a_trig_angle_unit_inside_the_function() {
        assert_eq!(rewrite("sin 30 deg").expr, "sin(30 deg)");
        assert_eq!(rewrite("cos 60\u{b0} + 1").expr, "cos(60 \u{b0}) + 1");
        assert_eq!(rewrite("tan 1 rad").expr, "tan(1 rad)");
        assert_eq!(rewrite("sin 30").expr, "sin 30");
        assert_eq!(rewrite("sin(30) deg").expr, "sin(30) deg");
    }

    #[test]
    fn rewrite_reads_in_after_a_length_as_inches() {
        assert_eq!(rewrite("5 ft 11 in").expr, "(5 ft + 11 inch)");
        assert_eq!(rewrite("5 ft 11").expr, "(5 ft + 11 inch)");
        assert_eq!(rewrite("5 ft 11 in cm").expr, "(5 ft + 11 inch) in cm");
        assert_eq!(rewrite("3 in + 2 in").expr, "3 inch + 2 inch");
        assert_eq!(rewrite("20 in in cm").expr, "20 inch in cm");
        // `in` as a conversion stays one.
        assert_eq!(rewrite("20 inches in cm").expr, "20 inches in cm");
        assert_eq!(rewrite("255 in hex").expr, "255 in hex");
        assert_eq!(rewrite("12 in cm").expr, "12 in cm");
    }

    /// fend applies `%` to a whole product or quotient: `100 / 20%` is 5%.
    #[test]
    fn rewrite_percent_factors_as_fractions() {
        assert_eq!(rewrite("100 * 15%").expr, "100 * (15/100)");
        assert_eq!(rewrite("15% * 100").expr, "(15/100) * 100");
        assert_eq!(rewrite("1 + 100 / 20%").expr, "1 + 100 / (20/100)");
        assert_eq!(rewrite("4 ^ 50%").expr, "4 ^ (50/100)");
        assert_eq!(rewrite("x = 100 * 15%").expr, "x = 100 * (15/100)");
    }

    #[test]
    fn rewrite_keeps_percent_arithmetic_and_modulo() {
        for expr in [
            "50% * 50%",
            "20% / 2",
            "2 * 10 % 3",
            "2 * 15% of 80",
            "2 * fee",
            "15%",
        ] {
            assert_eq!(rewrite(expr).expr, expr);
        }
    }

    #[test]
    fn rewrite_currency_symbols_keep_scale_suffixes() {
        assert_eq!(rewrite("$50k").expr, "(50k USD)");
        assert_eq!(rewrite("2 * \u{20ac}1.5M").expr, "2 * (1.5M EUR)");
        // A unit's letter after the number: left bare, so it stays unknown.
        assert_eq!(rewrite("$5m").expr, "5 USDm");
        assert_eq!(rewrite("$5bn").expr, "5 USDbn");
    }

    #[test]
    fn rewrite_percent_as_of() {
        assert_eq!(rewrite("50 as a % of 100").expr, "((50) / (100) * 100)%");
    }

    #[test]
    fn rewrite_percent_of_what_is() {
        assert_eq!(rewrite("20% of what is 30 cm").expr, "(30 cm) / ((20)/100)");
    }

    #[test]
    fn rewrite_times_word() {
        assert_eq!(rewrite("$8 times 3").expr, "(8 USD) * 3");
    }

    #[test]
    fn rewrite_currency_symbols_anywhere() {
        assert_eq!(rewrite("2 * $840").expr, "2 * (840 USD)");
        assert_eq!(rewrite("$5 * $2").expr, "(5 USD) * (2 USD)");
        assert_eq!(rewrite("$1,234.56 * 2").expr, "(1,234.56 USD) * 2");
        assert_eq!(rewrite("-$5.50").expr, "-(5.50 USD)");
        assert_eq!(rewrite("\u{20ac}37.60 * 2").expr, "(37.60 EUR) * 2");
        assert_eq!(rewrite("A$150.00 in USD").expr, "(150.00 AUD) in USD");
        assert_eq!(rewrite("CN\u{a5}780.00").expr, "(780.00 CNY)");
        assert_eq!(rewrite("\u{a5}16000").expr, "(16000 JPY)");
        assert_eq!(rewrite("26,125 \u{20ab} + 1").expr, "(26,125 VND) + 1");
        // Not a symbol when it's part of a word.
        assert_eq!(rewrite("ARM5").expr, "ARM5");
    }

    /// A quantity after a unit's `/` is divided whole, so `6 m / 2 s` is a
    /// speed and not `3 m s`.
    #[test]
    fn rewrite_groups_a_quantity_divided_into_a_unit() {
        for (typed, grouped) in [
            ("6 m / 2 s", "6 m / (2 s)"),
            ("6 m/2 s", "6 m/(2 s)"),
            ("100 km / 50 km/h", "100 km / (50 km/h)"),
            ("500 kcal / 2 hours", "500 kcal / (2 hours)"),
            ("10 m^3 / 2 m^2", "10 m^3 / (2 m^2)"),
            ("$40 / 2 hours", "(40 USD) / (2 hours)"),
            ("6 m divided by 2 s", "6 m / (2 s)"),
            ("a / 2 m / 3 s", "a / (2 m) / (3 s)"),
            ("x = 6 m / 2 s", "x = 6 m / (2 s)"),
            ("100 m^2 / 5 sq m", "100 m^2 / (5 square) m"),
            ("10 ml / 2 tea spoons", "10 ml / (2 teaspoons)"),
        ] {
            assert_eq!(rewrite(typed).expr, grouped, "{typed}");
        }
    }

    /// A number on the left, a conversion, a function or a group already
    /// there: fend's own reading stands.
    #[test]
    fn rewrite_leaves_other_divisions_alone() {
        for expr in [
            "1/2 cup",
            "10 / 2 m",
            "6 m / 2 in cm",
            "6 m / 2 to cm",
            "6 m / 2 mod 3",
            "6 m / (2 s)",
            "x / 2 sqrt(4)",
            "6 m / 2",
        ] {
            assert_eq!(rewrite(expr).expr, expr, "{expr}");
        }
    }

    /// A sign after the symbol, and an exponent, belong to the amount.
    #[test]
    fn rewrite_currency_symbols_take_a_sign_and_an_exponent() {
        assert_eq!(rewrite("$-5").expr, "(-5 USD)");
        assert_eq!(rewrite("$ -5").expr, "(-5 USD)");
        assert_eq!(rewrite("1 + $-5").expr, "1 + (-5 USD)");
        assert_eq!(rewrite("$1e6").expr, "(1e6 USD)");
        assert_eq!(rewrite("\u{20ac}1.5e-3").expr, "(1.5e-3 EUR)");
    }

    /// `in $` is a currency, not fend's own `$`: it gets Soos's rounding,
    /// and `€`, `₫` and the rest are known at all.
    #[test]
    fn rewrite_a_bare_symbol_after_a_conversion_word_is_its_code() {
        assert_eq!(rewrite("5 EUR in $").expr, "5 EUR in USD");
        assert_eq!(rewrite("5 EUR in \u{20ac}").expr, "5 EUR in EUR");
        assert_eq!(rewrite("$1 in \u{20ab}").expr, "(1 USD) in VND");
        assert_eq!(rewrite("S$5 in CN\u{a5}").expr, "(5 SGD) in CNY");
        assert_eq!(rewrite("5 EUR as A$").expr, "5 EUR as AUD");
        // A symbol that is part of a word is left alone.
        assert_eq!(rewrite("5 RMB").expr, "5 RMB");
    }

    /// `in` as the target of a conversion is the unit inches.
    #[test]
    fn rewrite_in_as_a_conversion_target_is_inches() {
        assert_eq!(rewrite("5 cm to in").expr, "5 cm to inch");
        assert_eq!(rewrite("72 pt in in").expr, "72 pt in inch");
        assert_eq!(rewrite("20 in in cm").expr, "20 inch in cm");
    }

    #[test]
    fn rewrite_tea_spoon() {
        assert_eq!(rewrite("20 ml in tea spoons").expr, "20 ml in teaspoons");
        assert_eq!(rewrite("1 tea spoon").expr, "1 teaspoon");
    }

    #[test]
    fn rewrite_var_on_names_the_variable_to_check() {
        let out = rewrite("fee on price");
        assert_eq!(out.expr, "(price) + (fee) * (price)");
        assert_eq!(out.percent_var.as_deref(), Some("fee"));
    }

    /// One clock time taken from another is how long between them, a value
    /// `sum` can add up; whole hours read as hours.
    #[test]
    fn rewrite_clock_difference_is_a_duration() {
        for (typed, rewritten) in [
            ("3PM - 10AM", "(5 hours)"),
            ("5PM minus 9:00", "(8 hours)"),
            ("10:30AM - 10AM", "(30 minutes)"),
            ("10AM - 3PM", "(-5 hours)"),
            ("shift = 3PM - 10AM", "shift = (5 hours)"),
            ("(3PM - 10AM) * 2", "((5 hours)) * 2"),
            ("3PM - 10AM in minutes", "(5 hours) in minutes"),
        ] {
            assert_eq!(rewrite(typed).expr, rewritten, "{typed}");
        }
        // Not a difference of two times.
        assert_eq!(rewrite("3PM - 10").expr, "3PM - 10");
        // Lowercase is fend's picometre and attometre, not a time.
        assert_eq!(rewrite("3pm - 10am").expr, "3pm - 10am");
    }

    #[test]
    fn rewrite_operator_words() {
        for (words, symbol) in [
            ("plus", "+"),
            ("with", "+"),
            ("and", "+"),
            ("minus", "-"),
            ("subtract", "-"),
            ("without", "-"),
            ("times", "*"),
            ("multiplied by", "*"),
            ("mul", "*"),
            ("divide", "/"),
            ("divide by", "/"),
            ("divided by", "/"),
        ] {
            assert_eq!(
                rewrite(&format!("4 {words} 4")).expr,
                format!("4 {symbol} 4"),
                "{words}"
            );
        }
        // Only whole words.
        for expr in ["withdrawal", "android", "plusone", "x_and_y"] {
            assert_eq!(rewrite(expr).expr, expr);
        }
    }

    #[test]
    fn rewrite_arc_functions() {
        assert_eq!(rewrite("arcsin(1)").expr, "asin(1)");
        assert_eq!(rewrite("arctanh 0.5").expr, "atanh 0.5");
        assert_eq!(rewrite("arcsine").expr, "arcsine");
    }

    #[test]
    fn rewrite_sq_and_cu_as_square_and_cubic() {
        assert_eq!(rewrite("20 sq cm").expr, "20 square cm");
        assert_eq!(rewrite("5 cu ft").expr, "5 cubic ft");
        assert_eq!(rewrite("20 sq in").expr, "20 square inch");
        for expr in ["sq * 2", "11 sqm", "sqrt 4"] {
            assert_eq!(rewrite(expr).expr, expr);
        }
    }

    #[test]
    fn rewrite_joins_a_number_spaced_into_thousands() {
        assert_eq!(rewrite("5 300 in sci").expr, "5300 in sci");
        assert_eq!(rewrite("1 000 000 + 1").expr, "1000000 + 1");
        assert_eq!(rewrite("5 300.5").expr, "5300.5");
        assert_eq!(rewrite("$5 300").expr, "(5300 USD)");
        // Not groups of three, or part of a name or a unit.
        for expr in ["5 3000", "5 30", "x5 300", "5 300m"] {
            assert_eq!(rewrite(expr).expr, expr);
        }
    }

    #[test]
    fn rewrite_percent_on_and_off_what_is() {
        assert_eq!(
            rewrite("5% on what is 6 eur").expr,
            "(6 eur) / (1 + (5)/100)"
        );
        assert_eq!(
            rewrite("5% off what is 6 eur").expr,
            "(6 eur) / (1 - (5)/100)"
        );
    }
}
