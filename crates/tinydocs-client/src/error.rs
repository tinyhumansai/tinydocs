//! `TinyDocs` call errors kept distinct for the host's tool and reporting layers.

use std::fmt::{Debug, Display, Formatter};

/// Why a `TinyDocs` operation did not produce its requested result.
///
/// Remote text is retained for an explicit caller policy, but [`Debug`] and
/// [`Display`] never include it. Module-provided text is untrusted and may
/// contain caller input or credentials.
#[derive(Clone, PartialEq, Eq)]
pub enum DocumentCallError {
    /// The module is unavailable or cannot serve this contract.
    Unavailable(String),
    /// `TinyDocs` rejected caller-provided input.
    InvalidInput(String),
    /// The operation or its output transfer failed.
    Failed(String),
}

impl Debug for DocumentCallError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let (kind, message) = match self {
            Self::Unavailable(_) => ("Unavailable", "<redacted>"),
            Self::InvalidInput(_) => ("InvalidInput", "<redacted>"),
            Self::Failed(_) => ("Failed", "<redacted>"),
        };
        formatter.debug_tuple(kind).field(&message).finish()
    }
}

impl DocumentCallError {
    /// Return the untrusted message received from `TinyDocs`, when one exists.
    ///
    /// Apply an explicit product redaction policy before displaying, storing,
    /// or logging this text. [`Debug`] and [`Display`] use only the error
    /// category and never include this value.
    #[must_use]
    pub fn raw_message(&self) -> &str {
        match self {
            Self::Unavailable(message) | Self::InvalidInput(message) | Self::Failed(message) => {
                message
            }
        }
    }
}

impl Display for DocumentCallError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let category = match self {
            Self::Unavailable(_) => "unavailable",
            Self::InvalidInput(_) => "input rejected",
            Self::Failed(_) => "execution failed",
        };
        write!(formatter, "MODULE_CALL_REPORTED: module {category}")
    }
}

impl std::error::Error for DocumentCallError {}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
