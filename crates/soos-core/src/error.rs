//! Why a line has no result: the full message, and the short label the
//! result column shows in its place.

use std::fmt;

/// A line that couldn't be calculated. `Display` is the full message (the
/// app's hover text); [`LineError::short`] is the label for the result
/// column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineError {
    /// fend's own message, labelled by its wording -- see
    /// [`crate::format::shorten_error`].
    Fend(String),
    /// A line over the length cap. A variant of its own because a `sum` that
    /// fails this way has a block too long to add up, not a long line.
    TooLong,
    /// A message Soos writes itself, with the label that stands for it. The
    /// two sit together where the error is made, so neither can drift from
    /// the other.
    Own {
        message: String,
        label: &'static str,
    },
}

impl LineError {
    pub(crate) fn own(message: impl Into<String>, label: &'static str) -> Self {
        Self::Own {
            message: message.into(),
            label,
        }
    }

    /// A message short enough to be its own label.
    pub(crate) fn plain(message: &'static str) -> Self {
        Self::own(message, message)
    }

    /// fend refused to add or convert between different units.
    pub(crate) fn is_unit_mismatch(&self) -> bool {
        matches!(self, Self::Fend(message)
            if crate::format::shorten_error(message) == crate::format::UNIT_MISMATCH)
    }

    /// The full message.
    fn message(&self) -> &str {
        match self {
            Self::Fend(message) | Self::Own { message, .. } => message,
            Self::TooLong => "too long",
        }
    }

    /// The label for the result column.
    pub fn short(&self) -> String {
        match self {
            Self::Fend(message) => crate::format::shorten_error(message),
            Self::Own { label, .. } => (*label).to_string(),
            Self::TooLong => self.message().to_string(),
        }
    }
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// So `?` carries fend's `String` errors.
impl From<String> for LineError {
    fn from(message: String) -> Self {
        Self::Fend(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fend_error_is_labelled_by_its_wording_and_an_own_one_by_its_label() {
        let fend = LineError::from("unknown identifier 'metr'".to_string());
        assert_eq!(fend.short(), "unknown metr");
        assert_eq!(fend.to_string(), "unknown identifier 'metr'");

        let own = LineError::own("'x' is not a percent; 'on' needs one", "not a percent");
        assert_eq!(own.short(), "not a percent");
        assert_eq!(own.to_string(), "'x' is not a percent; 'on' needs one");

        assert_eq!(LineError::plain("too nested").short(), "too nested");
        assert_eq!(LineError::TooLong.short(), "too long");
        assert_eq!(LineError::TooLong.to_string(), "too long");
    }

    /// An own message is labelled by its label, whatever its wording says.
    #[test]
    fn an_own_message_is_not_labelled_by_matching_its_text() {
        let own = LineError::own("division by zero, in Soos's own words", "mine");
        assert_eq!(own.short(), "mine");
    }

    #[test]
    fn only_fend_can_make_a_unit_mismatch() {
        let mismatch = "cannot convert from m to kg: units differ";
        assert!(LineError::from(mismatch.to_string()).is_unit_mismatch());
        assert!(!LineError::own(mismatch, "something else").is_unit_mismatch());
        assert!(!LineError::TooLong.is_unit_mismatch());
    }
}
