//! Rewrites the natural-language phrasing fend doesn't parse, and answers
//! the date and time-zone lines fend can't.
//!
//! fend-core 1.5.8 already handles `in`/`to`/`as` conversions, scale words
//! (`k`, `million`), implicit multiplication (`6(3)`), `P% of X`,
//! `@2026-12-25` date literals and `×`/`−`/`÷`. Handled here instead:
//!  - `into`, `tea spoon(s)`, `arcsin`, `sq cm` and `cu cm`, and the operator
//!    words (`times`, `plus`, `minus`, `divide by`, ...), where `and` is a
//!    plus and not fend's bitwise AND;
//!  - a number with spaces between its thousands (`5 300`);
//!  - `sin 30 deg`, which fend reads as `(sin 30) deg`;
//!  - `5 ft 11 in` and `3 in`, where fend reads `in` as a conversion;
//!  - `100 * 15%` and `100 / 20%`: fend applies `%` after the product or
//!    quotient, giving `1500%` and `5%`;
//!  - `P% on X`, `P% off X`, `P% of/on/off what is X`, `X as a % of Y` and
//!    `var on X`;
//!  - currency symbols anywhere in a line (`$840`, `€5`, `A$3`, `26125 ₫`):
//!    fend reads some as units of their own, which skip Soos's rounding
//!    (`5€`), and the rest not at all (`₫`);
//!  - leading `Label: ` prefixes, `//` comments and `"quoted notes"`;
//!  - `today`/`tomorrow`/`yesterday`/`now` and offsets from them (also
//!    `35 days before 15 nov`): fend's own `today` fails with "unable to get
//!    the current date";
//!  - time-zone conversion, which fend has no notion of.
//!
//! Split by what the code does: [`lines`] sorts a raw line, [`phrasing`]
//! rewrites it, and [`dates`] answers the date and time-zone lines.

mod dates;
mod lines;
mod phrasing;

pub(crate) use dates::eval_date;
pub(crate) use lines::{
    after_label, classify, is_label, INLINE_QUOTED_NOTE, TRAILING_LINE_COMMENT,
};
pub(crate) use phrasing::{rewrite, split_conversion, CONVERT_KW, INTO_WORD, OPERATOR_WORD};
