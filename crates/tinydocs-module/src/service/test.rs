//! Unit tests for the `TinyBus` service declaration.

#![allow(clippy::unwrap_used)]

use tinybus::Interface;

use super::*;

#[test]
fn service_identity_is_valid_and_dispatch_matches_the_manifest() {
    assert!(tinybus::BusName::new(BUS_NAME).is_ok());
    assert!(tinybus::ObjectPath::new(OBJECT_PATH).is_ok());

    let members = TinyDocs.members();
    assert_eq!(
        members,
        &[tinybus::MemberName::new("GenerateDocx").unwrap()]
    );
}

#[test]
fn domain_errors_keep_distinct_wire_names() {
    let invalid_error = Error::invalid_input("title", "must not be empty");
    let invalid = map_error(&invalid_error);
    assert_eq!(invalid.wire_name(), INVALID_INPUT_ERROR);

    let generation_error = Error::generation_failed("writer stopped");
    let failed = map_error(&generation_error);
    assert_eq!(failed.wire_name(), GENERATION_FAILED_ERROR);
}
