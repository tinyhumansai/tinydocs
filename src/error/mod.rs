//! Crate-wide error and result types.
//!
//! Every fallible public function in this crate returns [`Result`], and every
//! failure mode is a distinct [`Error`] variant. Add a variant rather than
//! encoding new context into an existing message: callers match on variants,
//! and message text is not a stable API.
//!
//! The variants are deliberately *host-agnostic*. A host that surfaces these
//! to an LLM (the reason [`Error::InvalidInput`] carries a structured
//! `field` / `reason` pair rather than a formatted sentence) maps them onto
//! its own tool-error shape; a host writing to disk maps them onto its own.
//! Nothing here knows about artifacts, timeouts, or async runtimes — those are
//! the host's concerns, because only the host knows its own deadline policy.

/// Errors returned by this crate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A document spec failed validation before any synthesis was attempted.
    ///
    /// `field` names the offending path in the spec using the same dotted /
    /// indexed notation the JSON input uses (`sections[2].bullets[0]`), so an
    /// LLM that produced the spec can self-correct without re-reading the
    /// whole schema. `reason` states the violated constraint.
    #[error("invalid input for field '{field}': {reason}")]
    InvalidInput {
        /// Path of the offending field within the spec.
        field: String,
        /// The constraint that was violated.
        reason: String,
    },

    /// The underlying document library failed to synthesise the output.
    ///
    /// `detail` is the library's own error rendered as text and truncated to a
    /// bounded length, so the variant never carries an unbounded payload back
    /// to a caller that forwards it to a model.
    #[error("document generation failed: {detail}")]
    GenerationFailed {
        /// Truncated underlying library error.
        detail: String,
    },

    /// The underlying library failed to extract text from an input document.
    ///
    /// Distinct from [`Error::GenerationFailed`] because the two have opposite
    /// causes and opposite remedies: generation fails on *our* output path and
    /// usually means a bug or an exhausted resource, whereas extraction fails on
    /// *someone else's* input and usually means the document is damaged,
    /// encrypted, or carries no extractable text layer at all. A caller that
    /// retries one should not retry the other.
    ///
    /// `detail` is truncated on the same bound as `GenerationFailed`.
    #[error("text extraction failed: {detail}")]
    ExtractionFailed {
        /// Truncated underlying library error.
        detail: String,
    },
}

impl Error {
    /// Maximum length, in Unicode scalar values, of a [`Error::GenerationFailed`]
    /// detail string.
    pub const MAX_DETAIL_CHARS: usize = 500;

    /// Suffix appended when a detail string is truncated.
    const TRUNCATION_SUFFIX: &'static str = " […truncated]";

    /// Build a [`Error::GenerationFailed`] with `raw` truncated (UTF-8-safe) to
    /// [`Error::MAX_DETAIL_CHARS`].
    ///
    /// Truncation counts characters, not bytes, so a multi-byte error message
    /// can never be cut mid-codepoint.
    #[must_use]
    pub fn generation_failed(raw: &str) -> Self {
        Self::GenerationFailed {
            detail: Self::truncate_detail(raw),
        }
    }

    /// Truncate `raw` to [`Error::MAX_DETAIL_CHARS`] characters, appending the
    /// standard truncation suffix when anything was dropped.
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

    /// Build an [`Error::ExtractionFailed`] with `raw` truncated (UTF-8-safe) to
    /// [`Error::MAX_DETAIL_CHARS`].
    #[must_use]
    pub fn extraction_failed(raw: &str) -> Self {
        Self::ExtractionFailed {
            detail: Self::truncate_detail(raw),
        }
    }

    /// Build an [`Error::InvalidInput`] for `field` violating `reason`.
    #[must_use]
    pub fn invalid_input(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidInput {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

/// The crate's standard result type.
///
/// Use this alias in public signatures instead of spelling out
/// `std::result::Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;
