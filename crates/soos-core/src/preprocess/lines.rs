//! Sorting a raw line: blank, header, label or an expression, with its
//! label, `//` comment and `"quoted notes"` taken off.

use std::sync::LazyLock;

use regex::Regex;

use crate::LineResult;

/// A `//` comment, to the end of the line. This and the next are
/// `pub(crate)` so [`crate::highlight`] colours exactly what is stripped.
pub(crate) static TRAILING_LINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//.*$").unwrap());

pub(crate) static INLINE_QUOTED_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[^"]*""#).unwrap());

/// What a label may say before its colon: a letter first, then letters,
/// digits, spaces and a little punctuation (`Week 2`, `Rent (monthly)`,
/// `Food & drinks`). No `=`, `/`, quotes or operators, so a line with a `:`
/// in a real expression (`x = 1 // note: yes`) is never taken for one.
const LABEL_NAME: &str = r"\p{L}[\p{L}\p{M}\p{N} &'\u{2019}().,_-]*";

static LEADING_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?P<name>{LABEL_NAME}):(?P<gap>\s*)(?P<rest>\S.*)$"
    ))
    .unwrap()
});

/// The expression after a leading `Label: `, for a line with its
/// indentation removed. With no space after the colon (`Rent:1800`), the
/// label can't end in a digit, since `Meeting 3:30PM` is a time, and the
/// expression can't start with `/`, so `http://` isn't a label.
pub(crate) fn after_label(trimmed: &str) -> Option<regex::Match<'_>> {
    let caps = LEADING_LABEL.captures(trimmed)?;
    let rest = caps.name("rest")?;
    let tight = caps["gap"].is_empty();
    let tight_but_not_a_label = tight
        && (caps["name"].ends_with(|c: char| c.is_ascii_digit()) || rest.as_str().starts_with('/'));
    (!tight_but_not_a_label).then_some(rest)
}

static LABEL_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^{LABEL_NAME}:$")).unwrap());

/// Classify a raw line: an expression for the engine, with its label
/// prefix, `//` comment and `"quoted notes"` stripped, or the result of a
/// line that has none (blank, header or label).
pub(crate) fn classify(raw: &str) -> Result<String, LineResult> {
    let raw = plain_characters(raw);
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return Err(LineResult::Blank);
    }
    if trimmed.starts_with('#') {
        return Err(LineResult::Header);
    }
    if is_label(trimmed) {
        return Err(LineResult::Label);
    }
    let trimmed = after_label(trimmed).map_or(trimmed, |rest| rest.as_str());

    let mut expr = TRAILING_LINE_COMMENT.replace(trimmed, "").into_owned();
    expr = INLINE_QUOTED_NOTE.replace_all(&expr, "").into_owned();
    let expr = expr.trim().to_string();
    if expr.is_empty() {
        return Err(LineResult::Blank);
    }
    Ok(expr)
}

/// A line as typed, not as pasted: a bank statement or a Word document
/// brings no-break and thin spaces (`5\u{a0}300`), an invisible BOM or
/// zero-width space, and en dashes for minus, and fend rejects each of them.
fn plain_characters(raw: &str) -> String {
    raw.chars()
        .filter_map(|c| match c {
            '\u{a0}' | '\u{2007}' | '\u{2009}' | '\u{202f}' => Some(' '),
            '\u{feff}' | '\u{200b}' => None,
            '\u{2013}' => Some('-'),
            c => Some(c),
        })
        .collect()
}

/// A line that's only a label (`Costs:`, `Week 2:`). Left to fend, a
/// `name: ...` line is a lambda: `Q1: 500` would answer `\Q1.500`, and a
/// heading like `Week 2:` would be an error that spoils the block's `sum`.
pub(crate) fn is_label(trimmed: &str) -> bool {
    LABEL_ONLY.is_match(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preprocess::rewrite;

    #[test]
    fn classify_blank_and_comments() {
        assert_eq!(classify(""), Err(LineResult::Blank));
        assert_eq!(classify("   "), Err(LineResult::Blank));
        assert_eq!(classify("// just a comment"), Err(LineResult::Blank));
    }

    #[test]
    fn classify_header_and_label() {
        assert_eq!(classify("# Totals"), Err(LineResult::Header));
        assert_eq!(classify("Costs:"), Err(LineResult::Label));
    }

    #[test]
    fn classify_strips_trailing_and_inline_comments() {
        assert_eq!(classify("1 + 1 // trailing"), Ok("1 + 1".into()));
        assert_eq!(classify(r#"1 + 1 "inline note""#), Ok("1 + 1".into()));
    }

    #[test]
    fn classify_strips_a_leading_label_in_any_script() {
        assert_eq!(classify("Price: $7 * 4"), Ok("$7 * 4".into()));
        assert_eq!(classify("Tiền nhà: 1800"), Ok("1800".into()));
    }

    #[test]
    fn classify_label_heuristic() {
        assert_eq!(classify("Costs:"), Err(LineResult::Label));
        assert_eq!(classify("x: 5"), Ok("5".into()));
    }

    /// A label may carry digits and a little punctuation, so a heading like
    /// `Week 2:` isn't evaluated.
    #[test]
    fn classify_labels_with_digits_and_punctuation() {
        for label in ["Week 2:", "Q1:", "Total 5:", "Rent (monthly):", "Car-wash:"] {
            assert_eq!(classify(label), Err(LineResult::Label), "{label}");
        }
        for (line, rest) in [
            ("Q1: 500", "500"),
            ("Rent 2024: 1800", "1800"),
            ("Rent (monthly): 1800", "1800"),
            ("Food & drinks: 600", "600"),
            ("Mom's gift: 50", "50"),
            ("Car-wash: 20", "20"),
        ] {
            assert_eq!(classify(line), Ok(rest.into()), "{line}");
        }
    }

    /// Where a `:` belongs to the expression, there is no label.
    #[test]
    fn classify_leaves_a_colon_inside_an_expression_alone() {
        assert_eq!(classify("x = 1 // note: yes"), Ok("x = 1".into()));
        assert_eq!(classify("3:30PM in Tokyo"), Ok("3:30PM in Tokyo".into()));
        assert_eq!(classify("Meeting 3:30PM"), Ok("Meeting 3:30PM".into()));
        assert_eq!(classify("2024:"), Ok("2024:".into()));
        assert_eq!(classify("x = 5:"), Ok("x = 5:".into()));
    }

    #[test]
    fn classify_turns_pasted_spaces_dashes_and_invisibles_into_plain_ones() {
        assert_eq!(classify("5\u{a0}300 + 1"), Ok("5 300 + 1".into()));
        assert_eq!(classify("1\u{2009}000\u{202f}000"), Ok("1 000 000".into()));
        assert_eq!(classify("\u{feff}1+1"), Ok("1+1".into()));
        assert_eq!(classify("1\u{200b}+1"), Ok("1+1".into()));
        assert_eq!(classify("5 \u{2013} 3"), Ok("5 - 3".into()));
        // So a spaced number pasted from a statement is one number.
        assert_eq!(rewrite(&classify("5\u{a0}300").unwrap()).expr, "5300");
    }

    /// A label may be followed straight by its expression, unless that
    /// would take a time (`Meeting 3:30PM`) or a URL for one.
    #[test]
    fn classify_a_label_with_no_space_after_the_colon() {
        for (line, rest) in [
            ("Rent:1800", "1800"),
            ("Rent :1800", "1800"),
            ("Rent (monthly):1800", "1800"),
            ("Food & drinks:$600", "$600"),
        ] {
            assert_eq!(classify(line), Ok(rest.into()), "{line}");
        }
        for line in ["Meeting 3:30PM", "Q1:500"] {
            assert_eq!(classify(line), Ok(line.into()), "{line}");
        }
        assert!(after_label("http://example.com").is_none());
    }
}
