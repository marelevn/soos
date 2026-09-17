//! Per-line syntax token spans for the app's editor to colour. Stays
//! UI-agnostic -- byte ranges and a `TokenKind`, no `egui` types, no
//! colours -- and reuses `document.rs`'s own `prev`/`sum`/`avg` regexes, so
//! highlighting those keywords can't drift out of sync with what a line
//! actually does. `LEADING_LABEL_PREFIX` below is its own separate check,
//! not shared with `preprocess.rs`'s equivalent -- the two can drift.
//!
//! Any byte not covered by a returned span is plain text -- there's no
//! `TokenKind::Plain` variant to emit for it. That includes numbers and
//! bare identifiers: checked against `ref/*.png`, `$7`, `30 cm` and a
//! variable name used on its own (`Euro`, `price`) all render in the same
//! plain colour as everything else -- only the reserved words below, and
//! `Label:` lines, get a colour of their own.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::document::{AVG, PREV, SUM};
use crate::preprocess::{INLINE_QUOTED_NOTE, INTO_WORD, TIMES_WORD, TRAILING_LINE_COMMENT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    Header,
    /// A whole `Label:` line, or a leading `Label: ` prefix inside an
    /// expression line.
    Label,
    Keyword,
    ConversionWord,
}

// pub(crate): also reused by `document.rs` to reject a converter name that
// would collide with this same conversion-word vocabulary.
pub(crate) static CONVERSION_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(in|to|of|on|off|as)\b").unwrap());
static LEADING_LABEL_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[A-Za-z][A-Za-z ]*:\s+").unwrap());
// pub(crate): see CONVERSION_WORD above.
pub(crate) static DATE_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(today|now)\b").unwrap());

fn overlaps_any_of(spans: &[(Range<usize>, TokenKind)], r: &Range<usize>) -> bool {
    spans
        .iter()
        .any(|(c, _)| c.start < r.end && r.start < c.end)
}

/// Tokenize one raw document line (as typed, comment/label decoration
/// included -- this is a display concern, unlike `preprocess::classify`
/// which strips that decoration for evaluation).
pub fn tokens(line: &str) -> Vec<(Range<usize>, TokenKind)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with("//") {
        return vec![(0..line.len(), TokenKind::Comment)];
    }
    if trimmed.starts_with('#') {
        return vec![(0..line.len(), TokenKind::Header)];
    }
    if let Some(rest) = trimmed.strip_suffix(':') {
        if !rest.chars().any(|c| c.is_ascii_digit()) {
            return vec![(0..line.len(), TokenKind::Label)];
        }
    }

    let mut spans: Vec<(Range<usize>, TokenKind)> = Vec::new();

    for m in TRAILING_LINE_COMMENT.find_iter(line) {
        spans.push((m.range(), TokenKind::Comment));
    }
    for m in INLINE_QUOTED_NOTE.find_iter(line) {
        if !overlaps_any_of(&spans, &m.range()) {
            spans.push((m.range(), TokenKind::Comment));
        }
    }
    if let Some(m) = LEADING_LABEL_PREFIX.find(line) {
        if !overlaps_any_of(&spans, &m.range()) {
            spans.push((m.range(), TokenKind::Label));
        }
    }
    for re in [&*PREV, &*SUM, &*AVG, &*DATE_WORD] {
        for m in re.find_iter(line) {
            if !overlaps_any_of(&spans, &m.range()) {
                spans.push((m.range(), TokenKind::Keyword));
            }
        }
    }
    for re in [&*INTO_WORD, &*TIMES_WORD, &*CONVERSION_WORD] {
        for m in re.find_iter(line) {
            if !overlaps_any_of(&spans, &m.range()) {
                spans.push((m.range(), TokenKind::ConversionWord));
            }
        }
    }

    spans.sort_by_key(|(r, _)| r.start);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        tokens(line).into_iter().map(|(_, k)| k).collect()
    }

    #[test]
    fn whole_line_categories() {
        assert_eq!(tokens("// a comment"), vec![(0..12, TokenKind::Comment)]);
        assert_eq!(tokens("# Totals"), vec![(0..8, TokenKind::Header)]);
        assert_eq!(tokens("Costs:"), vec![(0..6, TokenKind::Label)]);
    }

    #[test]
    fn leading_label_prefix_is_labelled() {
        let line = "Price: $7 * 4";
        let spans = tokens(line);
        assert_eq!(&line[spans[0].0.clone()], "Price: ");
        assert_eq!(spans[0].1, TokenKind::Label);
    }

    #[test]
    fn keywords_and_conversion_words_only() {
        assert_eq!(
            kinds("sum in USD - 4%"),
            vec![TokenKind::Keyword, TokenKind::ConversionWord]
        );
        assert_eq!(kinds("today + 17 days"), vec![TokenKind::Keyword]);
    }

    #[test]
    fn numbers_and_bare_identifiers_are_untokenized() {
        // "price" and "Euro" stay plain -- see the module doc comment.
        assert_eq!(kinds("price = $8 times 3"), vec![TokenKind::ConversionWord]);
        assert_eq!(kinds("4 GBP in Euro"), vec![TokenKind::ConversionWord]);
    }

    #[test]
    fn trailing_comment_and_quote_claim_their_range_first() {
        let spans = tokens("1 + 1 // 5% note");
        assert_eq!(spans, vec![(6..16, TokenKind::Comment)]);
    }
}
