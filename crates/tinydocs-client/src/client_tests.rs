//! Focused classification and digest tests for the `TinyDocs` client.

use super::*;

fn failure(kind: ModuleCallError, wire_name: &str, message: &str) -> ModuleCallFailure {
    ModuleCallFailure {
        kind,
        wire_name: wire_name.to_string(),
        message: message.to_string(),
    }
}

#[test]
fn only_the_tinydocs_invalid_input_wire_name_is_model_correctable() {
    let invalid = classify_failure(failure(
        ModuleCallError::ModuleFault,
        INVALID_INPUT_WIRE_NAME,
        "invalid section heading",
    ));
    assert_eq!(
        invalid,
        DocumentCallError::InvalidInput("invalid section heading".to_string())
    );

    for wire_name in [
        "ai.tinyhumans.tinydocs.Error.GenerationFailed",
        "ai.tinyhumans.tinydocs.Error.ExtractionFailed",
        "ai.tinyhumans.tinydocs.Error.TransferFailed",
        "ai.tinyhumans.tinydocs.Error.UnknownOutput",
        "ai.tinyhumans.tinydocs.Error.SomethingNew",
    ] {
        assert!(matches!(
            classify_failure(failure(
                ModuleCallError::ModuleFault,
                wire_name,
                "untrusted remote text",
            )),
            DocumentCallError::Failed(_)
        ));
    }
}

#[test]
fn unavailable_sdk_outcomes_stay_distinct_from_module_failures() {
    assert!(matches!(
        classify_failure(failure(
            ModuleCallError::Unavailable,
            "ai.tinyhumans.tinybus.Error.ModuleUnavailable",
            "module is unavailable",
        )),
        DocumentCallError::Unavailable(_)
    ));
    assert!(matches!(
        classify_sdk_error(ModuleCallError::Unavailable),
        DocumentCallError::Unavailable(_)
    ));
    assert!(matches!(
        classify_sdk_error(ModuleCallError::TransportFailed),
        DocumentCallError::Failed(_)
    ));
}

fn assert_unavailable_without_remote_detail(error: &DocumentCallError, remote_detail: &str) {
    assert!(matches!(error, DocumentCallError::Unavailable(_)));
    assert_eq!(
        error.raw_message(),
        "MODULE_CALL_REPORTED: module incompatible contract"
    );
    assert!(!format!("{error:?}").contains(remote_detail));
    assert!(!error.to_string().contains(remote_detail));
}

#[test]
fn detailed_incompatible_contract_maps_to_unavailable_without_exposing_peer_text() {
    let remote_detail = "untrusted compatibility detail";
    let error = classify_failure(failure(
        ModuleCallError::IncompatibleContract,
        "ai.tinyhumans.tinybus.Error.IncompatibleVersion",
        remote_detail,
    ));
    assert_unavailable_without_remote_detail(&error, remote_detail);
}

#[test]
fn sdk_incompatible_contract_maps_to_unavailable_without_exposing_peer_text() {
    let error = classify_sdk_error(ModuleCallError::IncompatibleContract);
    assert_unavailable_without_remote_detail(&error, "untrusted compatibility detail");
}

#[test]
fn the_digest_matches_the_empty_document_vector() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}
