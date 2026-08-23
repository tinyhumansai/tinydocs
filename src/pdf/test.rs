//! Unit tests for PDF text extraction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{MAX_DOCUMENT_BYTES, extract_text};
use crate::Error;

/// Build a valid single-page PDF whose text layer holds `text`.
///
/// Assembled here rather than checked in as a binary fixture so the structure
/// under test is readable, and so the cross-reference offsets are computed from
/// the bytes actually emitted instead of being transcribed and going stale. The
/// document uses Helvetica, one of the base-14 fonts every reader knows, so no
/// font program has to be embedded.
fn pdf_with_text(text: &str) -> Vec<u8> {
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

#[test]
fn extracts_the_text_layer_of_a_valid_document() {
    let text = extract_text(&pdf_with_text("Hello tinydocs")).expect("extraction should succeed");
    assert!(
        text.contains("Hello tinydocs"),
        "extracted text missing the content: {text:?}"
    );
}

#[test]
fn rejects_empty_input() {
    match extract_text(&[]) {
        Err(Error::InvalidInput { field, .. }) => assert_eq!(field, "bytes"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn rejects_input_without_a_pdf_signature() {
    // A JPEG handed to the PDF path should name the offending field rather than
    // surfacing a parser's phrasing as an extraction failure.
    match extract_text(b"\xFF\xD8\xFFnot a pdf") {
        Err(Error::InvalidInput { field, reason }) => {
            assert_eq!(field, "bytes");
            assert!(reason.contains("PDF"), "unhelpful reason: {reason}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn rejects_an_oversize_document() {
    // A real signature followed by filler, so the rejection is the size check
    // rather than the signature check.
    let mut bytes = b"%PDF-1.4\n".to_vec();
    bytes.resize(MAX_DOCUMENT_BYTES + 1, b' ');
    match extract_text(&bytes) {
        Err(Error::InvalidInput { field, .. }) => assert_eq!(field, "bytes"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn a_damaged_document_fails_extraction_rather_than_validation() {
    // Correct signature, nothing else: it is a PDF as far as the boundary check
    // can tell, and the parser is the thing that has to reject it. This is the
    // branch that distinguishes `ExtractionFailed` from `InvalidInput`.
    match extract_text(b"%PDF-1.4\nthis is not a cross-reference table\n") {
        Err(Error::ExtractionFailed { detail }) => {
            assert!(!detail.is_empty(), "extraction error carried no detail");
        }
        other => panic!("expected ExtractionFailed, got {other:?}"),
    }
}

#[test]
fn a_document_with_no_text_layer_yields_empty_text_rather_than_an_error() {
    // A valid page carrying no text object at all — the shape a scanned page
    // has. Nothing to extract is not a failure to retry.
    let text = extract_text(&pdf_with_text("")).expect("extraction should succeed");
    assert!(text.trim().is_empty(), "expected no text, got {text:?}");
}
