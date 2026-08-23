//! Unit tests for the shared `TinyDocs` error contract.

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
fn extraction_errors_use_the_shared_truncation_bound() {
    let raw = "x".repeat(Error::MAX_DETAIL_CHARS * 2);
    let Error::ExtractionFailed { detail } = Error::extraction_failed(&raw) else {
        panic!("expected ExtractionFailed");
    };
    assert_eq!(detail.chars().count(), Error::MAX_DETAIL_CHARS);
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
