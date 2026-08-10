//! Integration tests for the public crate surface.
//!
//! These tests link against the crate as a downstream consumer would: they can
//! only use what `src/lib.rs` re-exports. Treat them as the regression suite
//! for the crate's public contract — if a change breaks a test here, it is a
//! breaking change for users.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// The whole public surface exercised here is `.docx` synthesis, so the file is
// empty (and must still compile) in a build without that gate.
#![cfg(feature = "docx")]

use tinydocs::{
    Error,
    docx::{self, DocumentSection, DocumentSpec},
};

fn spec() -> DocumentSpec {
    DocumentSpec {
        title: "Consumer Doc".to_string(),
        author: None,
        sections: vec![DocumentSection {
            heading: Some("Section".to_string()),
            paragraphs: vec!["Body text.".to_string()],
            bullets: vec!["A bullet".to_string()],
        }],
    }
}

#[test]
fn consumers_can_generate_a_docx() {
    let bytes = docx::generate(&spec()).expect("generation should succeed");
    assert_eq!(&bytes[0..2], b"PK");
}

#[test]
fn consumers_can_validate_a_spec_without_generating() {
    // A host rejecting a malformed tool call at its own boundary should not
    // have to pay for synthesis to find out the spec is bad.
    assert!(spec().validate().is_ok());

    let mut invalid = spec();
    invalid.sections.clear();
    assert!(matches!(
        invalid.validate(),
        Err(Error::InvalidInput { .. })
    ));
}

#[test]
fn invalid_input_names_the_offending_field() {
    let mut invalid = spec();
    invalid.title = String::new();
    match docx::generate(&invalid) {
        Err(Error::InvalidInput { field, .. }) => assert_eq!(field, "title"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn limits_are_visible_to_consumers() {
    // Hosts quote these in their own tool descriptions, so they are public and
    // must stay in lockstep with what validation enforces.
    let mut invalid = spec();
    invalid.title = "t".repeat(docx::MAX_TEXT_CHARS + 1);
    assert!(invalid.validate().is_err());
}
