//! Intake wire round trips and strict field handling.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
#[test]
fn request_and_result_payloads_round_trip() {
    let extraction = ExtractDocumentSpec::new(DocumentFormat::Xlsx);
    assert_eq!(
        serde_json::from_str::<ExtractDocumentSpec>(&serde_json::to_string(&extraction).unwrap())
            .unwrap(),
        extraction
    );
    let result = ExtractedDocument {
        format: DocumentFormat::Pdf,
        section_count: 2,
        sections: vec![DocumentSectionText {
            source: "page:2".into(),
            index: 2,
            text: String::new(),
            scanned_candidate: true,
        }],
        truncated: true,
    };
    assert_eq!(
        serde_json::from_str::<ExtractedDocument>(&serde_json::to_string(&result).unwrap())
            .unwrap(),
        result
    );
    let render = RenderPdfSpec {
        pages: vec![2],
        max_dimension: 1024,
        max_total_pixels: 4_000_000,
        max_output_bytes: 8_000_000,
    };
    assert_eq!(
        serde_json::from_str::<RenderPdfSpec>(&serde_json::to_string(&render).unwrap()).unwrap(),
        render
    );
    let output = RenderedPdf {
        page_count: 2,
        pages: vec![RenderedPdfPage {
            page: 2,
            width: 10,
            height: 20,
            output: OutputRef {
                output_id: "capability".into(),
                total_bytes: 42,
                sha256: "abc".into(),
            },
        }],
    };
    assert_eq!(
        serde_json::from_str::<RenderedPdf>(&serde_json::to_string(&output).unwrap()).unwrap(),
        output
    );
}
#[test]
fn rejects_unknown_wire_fields_and_formats() {
    assert!(
        serde_json::from_str::<ExtractDocumentSpec>(
            r#"{"format":"docx","max_text_bytes":100,"max_sections":1,"extra":true}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<DocumentFormat>(r#""doc""#).is_err());
    assert!(
        serde_json::from_str::<OutputRef>(
            r#"{"output_id":"x","total_bytes":1,"sha256":"y","bytes":[]}"#
        )
        .is_err()
    );
}

#[test]
fn complete_conversion_keeps_its_payload_and_latest_contract_requires_version_four() {
    assert_eq!(crate::CONTRACT_VERSION, 4);
    assert!(crate::is_compatible(4));
    assert!(!crate::is_compatible(3));
    assert!(crate::METHODS.contains(&crate::names::methods::CONVERT_MARKDOWN));
    for (format, wire) in [
        (DocumentFormat::Pdf, "pdf"),
        (DocumentFormat::Docx, "docx"),
        (DocumentFormat::Pptx, "pptx"),
        (DocumentFormat::Xlsx, "xlsx"),
    ] {
        assert_eq!(
            serde_json::to_value(format).unwrap(),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<DocumentFormat>(serde_json::json!(wire)).unwrap(),
            format
        );
    }
}
