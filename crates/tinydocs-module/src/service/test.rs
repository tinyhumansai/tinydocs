//! Unit tests for the `TinyBus` service declaration.
//!
//! The manifest and the generated dispatch table are two lists that have to stay
//! identical, and nothing but a test connects them: the macro takes the method
//! names as string literals, so a method added to the `impl` without a matching
//! literal is admitted by the loader and then fails to dispatch. That is the
//! invariant this file exists for.
//!
//! Bytes moving over a real broker is covered by `tests/module_e2e.rs`, which
//! loads the built artifact through the actual dynamic loader.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybus::Interface;

use super::*;
use crate::blobs::hex_digest;

/// The methods the manifest declares, in declaration order.
const DECLARED_METHODS: &[&str] = &[
    "BeginBlob",
    "PutChunk",
    "GetChunk",
    "ReleaseBlob",
    "GenerateDocx",
    "GeneratePptx",
    "ExtractText",
];

fn service() -> Documents {
    Documents {
        blobs: Arc::new(BlobStore::new()),
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
fn dispatch_members_match_the_manifest_exactly() {
    let members: Vec<String> = service()
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
fn every_declared_method_name_is_a_valid_member_name() {
    for method in DECLARED_METHODS {
        assert!(
            tinybus::MemberName::new(*method).is_ok(),
            "{method} is not a valid member name"
        );
    }
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
fn transfer_errors_are_grouped_by_what_the_caller_should_do() {
    // Gone: restart the transfer.
    assert_eq!(
        map_blob_error(&BlobError::UnknownBlob).wire_name(),
        UNKNOWN_BLOB_ERROR
    );
    // Full: the same request may succeed later.
    for refused in [BlobError::StagingFull, BlobError::TooManyBlobs] {
        assert_eq!(
            map_blob_error(&refused).wire_name(),
            TRANSFER_REFUSED_ERROR,
            "{refused:?} should be retryable"
        );
    }
    // Caller error: re-send, do not retry verbatim.
    for failed in [
        BlobError::MalformedDigest,
        BlobError::BlobTooLarge,
        BlobError::ChunkTooLarge,
        BlobError::OutOfOrderChunk {
            expected: 1,
            actual: 2,
        },
        BlobError::OverlongBlob,
        BlobError::DigestMismatch,
        BlobError::IncompleteBlob,
        BlobError::ReadPastEnd,
    ] {
        assert_eq!(
            map_blob_error(&failed).wire_name(),
            TRANSFER_FAILED_ERROR,
            "{failed:?} should not be reported as retryable"
        );
    }
}

#[test]
fn malformed_base64_is_an_invalid_input_and_does_not_echo_the_payload() {
    let err = decode_base64("this is not base64!!").expect_err("should reject");
    assert_eq!(err.wire_name(), INVALID_INPUT_ERROR);
    assert!(
        !format!("{err}").contains("not base64!!"),
        "the rejected payload leaked into the error message: {err}"
    );
}

#[test]
fn valid_base64_decodes() {
    assert_eq!(
        decode_base64(&BASE64.encode(b"round trip")).unwrap(),
        b"round trip"
    );
    assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
}

#[tokio::test]
async fn generate_docx_stages_a_readable_document() {
    use tinydocs::spec::DocumentSection;

    let service = service();
    let spec = DocumentSpec {
        title: "Charter".to_string(),
        author: Some("Alice".to_string()),
        sections: vec![DocumentSection {
            heading: Some("Goals".to_string()),
            paragraphs: vec!["Ship it.".to_string()],
            bullets: vec![],
        }],
    };

    let handle = service.generate_docx(spec).await.expect("should generate");
    assert!(handle.total_bytes > 0);

    let bytes = service
        .blobs
        .get_chunk(&handle.blob_id, 0, handle.total_bytes, Instant::now())
        .expect("staged output should be readable");
    assert_eq!(&bytes[..2], b"PK", "a .docx is a zip container");
    assert_eq!(hex_digest(&bytes), handle.sha256);
}

#[tokio::test]
async fn generate_docx_rejects_an_invalid_spec_without_staging_anything() {
    let service = service();
    let spec = DocumentSpec {
        title: String::new(),
        author: None,
        sections: vec![],
    };
    let err = service
        .generate_docx(spec)
        .await
        .expect_err("a blank title should be rejected");
    assert_eq!(err.wire_name(), INVALID_INPUT_ERROR);
    assert_eq!(
        service.blobs.live_count(),
        0,
        "a rejected call must not leave a blob behind"
    );
}

#[tokio::test]
async fn generate_pptx_consumes_the_image_blobs_it_is_given() {
    let service = service();
    let now = Instant::now();
    let png = tiny_png();

    // Stage an image the way a caller would, then reference it by id.
    let blob_id = service
        .blobs
        .begin(png.len() as u64, &hex_digest(&png), now)
        .unwrap();
    service.blobs.put_chunk(&blob_id, 0, &png, now).unwrap();
    assert_eq!(service.blobs.live_count(), 1);

    let handle = service
        .generate_pptx(WirePresentationSpec {
            title: "Quarterly".to_string(),
            author: None,
            theme: None,
            slides: vec![WireSlideSpec {
                title: "With a chart".to_string(),
                body: None,
                bullets: vec![],
                speaker_notes: None,
                images: vec![WireSlideImage {
                    blob_id: blob_id.clone(),
                    caption: Some("A chart".to_string()),
                }],
            }],
        })
        .await
        .expect("should generate");

    let bytes = service
        .blobs
        .get_chunk(&handle.blob_id, 0, handle.total_bytes, Instant::now())
        .expect("staged deck should be readable");
    assert_eq!(&bytes[..2], b"PK", "a .pptx is a zip container");

    // The image blob was taken, not copied: only the output remains staged.
    assert_eq!(
        service.blobs.live_count(),
        1,
        "the consumed image blob should have been released"
    );
    assert!(
        service
            .blobs
            .get_chunk(&blob_id, 0, 1, Instant::now())
            .is_err()
    );
}

#[tokio::test]
async fn generate_pptx_reports_a_missing_image_blob_rather_than_dropping_the_image() {
    // The caller staged it, so its absence is a transfer bug worth reporting —
    // not a deck quietly missing a slide's illustration.
    let service = service();
    let err = service
        .generate_pptx(WirePresentationSpec {
            title: "Quarterly".to_string(),
            author: None,
            theme: None,
            slides: vec![WireSlideSpec {
                title: "With a chart".to_string(),
                body: None,
                bullets: vec![],
                speaker_notes: None,
                images: vec![WireSlideImage {
                    blob_id: "blob-does-not-exist".to_string(),
                    caption: None,
                }],
            }],
        })
        .await
        .expect_err("a missing image blob should fail the call");
    assert_eq!(err.wire_name(), UNKNOWN_BLOB_ERROR);
}

#[tokio::test]
async fn generate_pptx_rejects_image_bytes_that_are_not_an_embeddable_image() {
    let service = service();
    let now = Instant::now();
    let junk = b"definitely not a png".to_vec();
    let blob_id = service
        .blobs
        .begin(junk.len() as u64, &hex_digest(&junk), now)
        .unwrap();
    service.blobs.put_chunk(&blob_id, 0, &junk, now).unwrap();

    let err = service
        .generate_pptx(WirePresentationSpec {
            title: "Quarterly".to_string(),
            author: None,
            theme: None,
            slides: vec![WireSlideSpec {
                title: "Broken".to_string(),
                body: None,
                bullets: vec![],
                speaker_notes: None,
                images: vec![WireSlideImage {
                    blob_id,
                    caption: None,
                }],
            }],
        })
        .await
        .expect_err("unrecognisable image bytes should be rejected");
    assert_eq!(err.wire_name(), INVALID_INPUT_ERROR);
}

#[tokio::test]
async fn extract_text_consumes_the_document_and_stages_the_text() {
    let service = service();
    let now = Instant::now();
    let doc = tiny_pdf("Hello from the bus");
    let blob_id = service
        .blobs
        .begin(doc.len() as u64, &hex_digest(&doc), now)
        .unwrap();
    service.blobs.put_chunk(&blob_id, 0, &doc, now).unwrap();

    let handle = service
        .extract_text(blob_id.clone())
        .await
        .expect("should extract");
    let bytes = service
        .blobs
        .get_chunk(&handle.blob_id, 0, handle.total_bytes, Instant::now())
        .unwrap();
    let text = String::from_utf8(bytes).expect("extracted text is utf-8");
    assert!(
        text.contains("Hello from the bus"),
        "extracted text missing content: {text:?}"
    );

    // The input was taken, so only the extracted text stays staged.
    assert_eq!(service.blobs.live_count(), 1);
}

#[tokio::test]
async fn extract_text_refuses_an_unknown_or_incomplete_blob() {
    let service = service();
    let now = Instant::now();

    let err = service
        .extract_text("blob-nope".to_string())
        .await
        .expect_err("unknown blob");
    assert_eq!(err.wire_name(), UNKNOWN_BLOB_ERROR);

    let doc = tiny_pdf("partial");
    let blob_id = service
        .blobs
        .begin(doc.len() as u64, &hex_digest(&doc), now)
        .unwrap();
    service
        .blobs
        .put_chunk(&blob_id, 0, &doc[..doc.len() / 2], now)
        .unwrap();
    let err = service
        .extract_text(blob_id)
        .await
        .expect_err("incomplete blob");
    assert_eq!(err.wire_name(), TRANSFER_FAILED_ERROR);
}

#[tokio::test]
async fn the_blob_methods_round_trip_a_payload_over_the_declared_surface() {
    // Exercises the four transfer methods through the same signatures the bus
    // calls, including the base64 hop the store itself never sees.
    let service = service();
    let payload: Vec<u8> = (0..5_000u32).map(|i| (i % 253) as u8).collect();

    let blob_id = service
        .begin_blob(payload.len() as u64, hex_digest(&payload))
        .await
        .unwrap();
    let received = service
        .put_chunk(blob_id.clone(), 0, BASE64.encode(&payload))
        .await
        .unwrap();
    assert_eq!(received, payload.len() as u64);

    let encoded = service
        .get_chunk(blob_id.clone(), 0, payload.len() as u64)
        .await
        .unwrap();
    assert_eq!(BASE64.decode(encoded).unwrap(), payload);

    service.release_blob(blob_id.clone()).await.unwrap();
    assert_eq!(service.blobs.live_count(), 0);
    assert!(service.release_blob(blob_id).await.is_err());
}

/// A 1×1 PNG, built from its header so the fixture needs no dependency.
fn tiny_png() -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&[0x08, 0x06, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
    out
}

/// A valid single-page PDF whose text layer holds `text`.
fn tiny_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
    ];

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
