//! Unit tests for the crate-wide error type.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::Error;

#[test]
fn short_details_are_left_intact() {
    let err = Error::generation_failed("boom");
    assert_eq!(
        err,
        Error::GenerationFailed {
            detail: "boom".to_string()
        }
    );
}

#[test]
fn long_details_are_truncated_with_a_suffix() {
    let raw = "x".repeat(Error::MAX_DETAIL_CHARS * 2);
    let Error::GenerationFailed { detail } = Error::generation_failed(&raw) else {
        panic!("expected GenerationFailed");
    };
    assert_eq!(detail.chars().count(), Error::MAX_DETAIL_CHARS);
    assert!(detail.ends_with("[…truncated]"));
}

#[test]
fn truncation_never_splits_a_multi_byte_character() {
    // Every character is 4 bytes, so a byte-based truncation would panic or
    // produce invalid UTF-8. Counting characters keeps the boundary valid.
    let raw = "🦀".repeat(Error::MAX_DETAIL_CHARS * 2);
    let detail = Error::truncate_detail(&raw);
    assert_eq!(detail.chars().count(), Error::MAX_DETAIL_CHARS);
    assert!(detail.starts_with('🦀'));
}

#[test]
fn detail_at_exactly_the_cap_is_not_truncated() {
    let raw = "y".repeat(Error::MAX_DETAIL_CHARS);
    assert_eq!(Error::truncate_detail(&raw), raw);
}

#[test]
fn invalid_input_carries_the_field_path_verbatim() {
    let err = Error::invalid_input("sections[2].bullets[0]", "must be ≤ 10 chars");
    assert_eq!(
        err,
        Error::InvalidInput {
            field: "sections[2].bullets[0]".to_string(),
            reason: "must be ≤ 10 chars".to_string(),
        }
    );
}
