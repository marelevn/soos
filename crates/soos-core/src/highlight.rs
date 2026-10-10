//! Syntax spans for the app's editor to colour: byte ranges and a
//! [`TokenKind`], no UI types. Comments, labels, `prev`/`sum`/`avg` and the
//! rewritten words are found with the evaluator's own regexes. Anything
//! outside a span, numbers and names included, is plain text.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::document::{AVG, PREV, SUM};
use crate::preprocess::{
    after_label, is_label, INLINE_QUOTED_NOTE, INTO_WORD, OPERATOR_WORD, TRAILING_LINE_COMMENT,
};

/// What a highlighted span is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A `//` comment, or a `"quoted note"`.
    Comment,
    /// A whole `# heading` line.
    Header,
    /// A whole `Label:` line, or a leading `Label: ` prefix inside an
    /// expression line.
    Label,
    /// `prev`, `sum`/`total`, `avg`/`average`, `today`/`tomorrow`/`yesterday`/`now`.
    Keyword,
    /// `in`, `to`, `of`, `on`, `off`, `as`, `before`, `after`, `into`, and the
    /// operator words (`times`, `plus`, `divide by`, ...).
    ConversionWord,
}

/// `in`/`to`/`of`/`on`/`off`/`as`/`before`/`after`. [`crate::document`] also
/// uses this and [`DATE_WORD`] to reserve these names.
pub(crate) static CONVERSION_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(in|to|of|on|off|as|before|after)\b").unwrap());
/// `today`/`tomorrow`/`yesterday`/`now`.
pub(crate) static DATE_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(today|tomorrow|yesterday|now)\b").unwrap());

fn overlaps_any_of(spans: &[(Range<usize>, TokenKind)], r: &Range<usize>) -> bool {
    spans
        .iter()
        .any(|(c, _)| c.start < r.end && r.start < c.end)
}

/// Tokenize one raw document line as typed, comments and labels included
/// -- this is a display concern, unlike [`crate::preprocess::classify`],
/// which strips them for evaluation. Spans are byte ranges into `line`,
/// sorted by start; earlier kinds win overlaps (a keyword inside a comment
/// stays part of the comment).
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
    if is_label(trimmed) {
        return vec![(0..line.len(), TokenKind::Label)];
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
    // `after_label` is anchored on a line with its indentation removed;
    // the span starts at 0 so the indentation is coloured with the label.
    let indent = line.len() - line.trim_start().len();
    if let Some(rest) = after_label(line.trim_start()) {
        let label = 0..indent + rest.start();
        if !overlaps_any_of(&spans, &label) {
            spans.push((label, TokenKind::Label));
        }
    }
    for re in [&*PREV, &*SUM, &*AVG, &*DATE_WORD] {
        for m in re.find_iter(line) {
            if !overlaps_any_of(&spans, &m.range()) {
                spans.push((m.range(), TokenKind::Keyword));
            }
        }
    }
    for re in [&*INTO_WORD, &*OPERATOR_WORD, &*CONVERSION_WORD] {
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
        assert_eq!(kinds("price = $8 times 3"), vec![TokenKind::ConversionWord]);
        assert_eq!(kinds("4 GBP in Euro"), vec![TokenKind::ConversionWord]);
    }

    #[test]
    fn trailing_comment_and_quote_claim_their_range_first() {
        let spans = tokens("1 + 1 // 5% note");
        assert_eq!(spans, vec![(6..16, TokenKind::Comment)]);
    }

    #[test]
    fn operator_words_and_before_after_are_conversion_words() {
        assert_eq!(
            kinds("4 plus 4 divided by 2"),
            vec![TokenKind::ConversionWord, TokenKind::ConversionWord]
        );
        assert_eq!(
            kinds("35 days before 15 nov"),
            vec![TokenKind::ConversionWord]
        );
    }
}
