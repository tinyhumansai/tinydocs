//! Unit tests for the `TinyBus` service declaration.
//!
//! The manifest and the generated dispatch table are two lists that must stay
//! identical, and nothing but a test connects them: the macro takes method names
//! as string literals, so a method added to the `impl` without a matching literal
//! is admitted by the loader and then fails to dispatch.
//!
//! The streaming paths are exercised in `tests/module_e2e.rs`, over a real
//! broker. A stream needs two connected peers, so there is no honest way to
//! unit-test one against a bare struct.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybus::Interface;

use super::*;
use crate::outputs::hex_digest;

/// The methods the manifest declares, in declaration order.
const DECLARED_METHODS: &[&str] = &[
    "GenerateDocx",
    "GeneratePptx",
    "ExtractText",
    "ReadOutput",
    "ReleaseOutput",
];

/// A service attached to a broker nothing else is on.
///
/// Enough for every method that does not read a stream.
async fn service() -> Documents {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    Documents {
        connection,
        outputs: Arc::new(OutputStore::new()),
    }
}

#[test]
fn service_identity_is_valid() {
    assert!(tinybus::BusName::new(BUS_NAME).is_ok());
    assert!(tinybus::ObjectPath::new(OBJECT_PATH).is_ok());
    // TinyBus derives a module's object path from its bus name by replacing dots
    // with slashes, and admission compares the two. A mismatch here would be
    // rejected by the loader rather than by anything in this crate.
    assert_eq!(OBJECT_PATH, format!("/{}", BUS_NAME.replace('.', "/")));
}

#[test]
fn every_declared_method_name_is_a_valid_member_name() {
    for method in DECLARED_METHODS {
        assert!(
            tinybus::MemberName::new(*method).is_ok(),
            "{method} is not a valid member name"
        );
    }
}

#[tokio::test]
async fn dispatch_members_match_the_manifest_exactly() {
    let members: Vec<String> = service()
        .await
        .members()
        .iter()
        .map(|member| member.as_str().to_string())
        .collect();
    let declared: Vec<String> = DECLARED_METHODS.iter().map(|m| (*m).to_string()).collect();
    assert_eq!(
        members, declared,
        "the interface impl and the module_export! methods list have drifted"
    );
}

#[test]
fn library_errors_keep_distinct_wire_names() {
    assert_eq!(
        map_error(&Error::invalid_input("title", "must not be empty")).wire_name(),
        INVALID_INPUT_ERROR
    );
    assert_eq!(
        map_error(&Error::generation_failed("writer stopped")).wire_name(),
        GENERATION_FAILED_ERROR
    );
    assert_eq!(
        map_error(&Error::extraction_failed("damaged xref")).wire_name(),
        EXTRACTION_FAILED_ERROR
    );
}

#[test]
fn output_errors_are_grouped_by_what_the_caller_should_do() {
    // Gone: make the call again.
    assert_eq!(
        map_output_error(&OutputError::UnknownOutput).wire_name(),
        UNKNOWN_OUTPUT_ERROR
    );
    // Full: the same request may succeed later.
    for refused in [
        OutputError::StoreFull,
        OutputError::TooManyOutputs,
        OutputError::OutputTooLarge,
    ] {
        assert_eq!(
            map_output_error(&refused).wire_name(),
            OUTPUT_REFUSED_ERROR,
            "{refused:?} should read as retryable"
        );
    }
    // Malformed read: fix the request.
    for failed in [OutputError::ChunkTooLarge, OutputError::ReadPastEnd] {
        assert_eq!(
            map_output_error(&failed).wire_name(),
            TRANSFER_FAILED_ERROR,
            "{failed:?} should not read as retryable"
        );
    }
}

#[tokio::test]
async fn generate_docx_holds_a_readable_document() {
    use tinydocs::spec::DocumentSection;

    let service = service().await;
    let handle = service
        .generate_docx(DocumentSpec {
            title: "Charter".to_string(),
            author: Some("Alice".to_string()),
            sections: vec![DocumentSection {
                heading: Some("Goals".to_string()),
                paragraphs: vec!["Ship it.".to_string()],
                bullets: vec![],
            }],
        })
        .await
        .expect("should generate");
    assert!(handle.total_bytes > 0);

    let encoded = service
        .read_output(handle.output_id.clone(), 0, handle.total_bytes)
        .await
        .expect("held output should be readable");
    let bytes = BASE64.decode(encoded).expect("output is base64");
    assert_eq!(&bytes[..2], b"PK", "a .docx is a zip container");
    assert_eq!(hex_digest(&bytes), handle.sha256);

    service
        .release_output(handle.output_id.clone())
        .await
        .expect("release should succeed");
    assert!(
        service.release_output(handle.output_id).await.is_err(),
        "releasing twice should report the output is gone"
    );
}

#[tokio::test]
async fn generate_docx_rejects_an_invalid_spec_without_holding_anything() {
    let service = service().await;
    let err = service
        .generate_docx(DocumentSpec {
            title: String::new(),
            author: None,
            sections: vec![],
        })
        .await
        .expect_err("a blank title should be rejected");
    assert_eq!(err.wire_name(), INVALID_INPUT_ERROR);
    assert_eq!(
        service.outputs.live_count(),
        0,
        "a rejected call must not leave an output behind"
    );
}

#[tokio::test]
async fn a_deck_with_no_images_needs_no_stream() {
    // The common case. Requiring a caller to open an empty stream to render a
    // text-only deck would be a pointless round trip.
    let service = service().await;
    let handle = service
        .generate_pptx(
            WirePresentationSpec {
                title: "Quarterly".to_string(),
                author: None,
                theme: None,
                slides: vec![WireSlideSpec {
                    title: "Text only".to_string(),
                    body: Some("No pictures.".to_string()),
                    bullets: vec![],
                    speaker_notes: None,
                    images: vec![],
                }],
            },
            None,
        )
        .await
        .expect("should generate");

    let encoded = service
        .read_output(handle.output_id, 0, handle.total_bytes)
        .await
        .unwrap();
    let bytes = BASE64.decode(encoded).unwrap();
    assert_eq!(&bytes[..2], b"PK", "a .pptx is a zip container");
}

#[tokio::test]
async fn a_deck_declaring_images_without_a_stream_is_refused() {
    // The spec and the transfer have to agree. Rendering the deck without the
    // pictures it asked for would be a silently wrong document.
    let service = service().await;
    let err = service
        .generate_pptx(
            WirePresentationSpec {
                title: "Quarterly".to_string(),
                author: None,
                theme: None,
                slides: vec![WireSlideSpec {
                    title: "With a chart".to_string(),
                    body: None,
                    bullets: vec![],
                    speaker_notes: None,
                    images: vec![WireSlideImage {
                        byte_len: 128,
                        caption: None,
                    }],
                }],
            },
            None,
        )
        .await
        .expect_err("a declared image with no stream should be refused");
    assert_eq!(err.wire_name(), INVALID_INPUT_ERROR);
}

#[tokio::test]
async fn reading_an_unknown_output_is_refused_by_name() {
    let service = service().await;
    let err = service
        .read_output("out-nope".to_string(), 0, 16)
        .await
        .expect_err("unknown output");
    assert_eq!(err.wire_name(), UNKNOWN_OUTPUT_ERROR);
}

#[tokio::test]
async fn a_malformed_read_is_refused_by_name() {
    let service = service().await;
    let handle = service
        .hold(b"small".to_vec())
        .expect("hold should succeed");

    let err = service
        .read_output(
            handle.output_id.clone(),
            0,
            crate::outputs::MAX_CHUNK_BYTES as u64 + 1,
        )
        .await
        .expect_err("oversize read");
    assert_eq!(err.wire_name(), TRANSFER_FAILED_ERROR);

    let err = service
        .read_output(handle.output_id, 999, 16)
        .await
        .expect_err("read past the end");
    assert_eq!(err.wire_name(), TRANSFER_FAILED_ERROR);
}
