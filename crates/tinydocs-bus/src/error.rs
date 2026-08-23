//! Structured errors shared by `TinyDocs` and its bus hosts.

/// Errors returned while validating or generating a document.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A document specification failed validation before generation began.
    #[error("invalid input for field '{field}': {reason}")]
    InvalidInput {
        /// Path of the offending field within the specification.
        field: String,
        /// The violated constraint.
        reason: String,
    },

    /// The OOXML writer failed to synthesize the requested document.
    #[error("document generation failed: {detail}")]
    GenerationFailed {
        /// Truncated writer error detail.
        detail: String,
    },
}

impl Error {
    /// Maximum length, in Unicode scalar values, of a generation error detail.
    pub const MAX_DETAIL_CHARS: usize = 500;

    /// Suffix appended when a generation error detail is truncated.
    const TRUNCATION_SUFFIX: &'static str = " […truncated]";

    /// Builds a generation error with `raw` truncated at a UTF-8 boundary.
    #[must_use]
    pub fn generation_failed(raw: &str) -> Self {
        Self::GenerationFailed {
            detail: Self::truncate_detail(raw),
        }
    }

    /// Truncates `raw` to the bounded generation-error detail length.
    #[must_use]
    pub fn truncate_detail(raw: &str) -> String {
        if raw.chars().count() <= Self::MAX_DETAIL_CHARS {
            return raw.to_string();
        }
        let keep = Self::MAX_DETAIL_CHARS.saturating_sub(Self::TRUNCATION_SUFFIX.chars().count());
        let mut out: String = raw.chars().take(keep).collect();
        out.push_str(Self::TRUNCATION_SUFFIX);
        out
    }

    /// Builds an invalid-input error for `field` violating `reason`.
    #[must_use]
    pub fn invalid_input(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidInput {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

/// Standard result type returned by fallible `TinyDocs` APIs.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test {
    //! Tests for the `TinyDocs` error contract.

    #![allow(clippy::panic)]

    use super::Error;

    #[test]
    fn long_details_are_truncated_without_splitting_utf8() {
        let raw = "🦀".repeat(Error::MAX_DETAIL_CHARS * 2);
        let Error::GenerationFailed { detail } = Error::generation_failed(&raw) else {
            panic!("expected GenerationFailed");
        };
        assert_eq!(detail.chars().count(), Error::MAX_DETAIL_CHARS);
        assert!(detail.ends_with("[…truncated]"));
    }

    #[test]
    fn invalid_input_preserves_the_field_path() {
        let error = Error::invalid_input("sections[2].bullets[0]", "must be ≤ 10 chars");
        assert_eq!(
            error,
            Error::InvalidInput {
                field: "sections[2].bullets[0]".to_string(),
                reason: "must be ≤ 10 chars".to_string(),
            }
        );
    }
}
