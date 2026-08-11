//! Unit tests for `.docx` synthesis.
//!
//! The spec's validation, blank/aggregate rules, and JSON contract are tested
//! in `crate::spec` — they are format-independent and must pass in a build with
//! this feature off. What is left here is the OOXML mapping itself: container
//! shape, which text reaches `word/document.xml`, and the blank-filtering that
//! decides how many paragraphs are emitted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{DocumentSection, DocumentSpec, generate};
use crate::Error;

/// One valid section carrying a heading, a paragraph, and a bullet.
fn section() -> DocumentSection {
    DocumentSection {
        heading: Some("Overview".to_string()),
        paragraphs: vec!["A body paragraph.".to_string()],
        bullets: vec!["A bullet".to_string()],
    }
}

/// A minimal valid spec.
fn spec() -> DocumentSpec {
    DocumentSpec {
        title: "Charter".to_string(),
        author: Some("Alice".to_string()),
        sections: vec![section()],
    }
}

/// Entry names inside a produced `.docx` byte buffer.
fn entry_names(bytes: &[u8]) -> Vec<String> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("output is a valid zip");
    (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect()
}

/// One entry's UTF-8 body out of a produced `.docx`.
fn entry_body(bytes: &[u8], name: &str) -> String {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("output is a valid zip");
    let mut entry = zip.by_name(name).expect("entry present");
    let mut body = String::new();
    std::io::Read::read_to_string(&mut entry, &mut body).unwrap();
    body
}

#[test]
fn generate_produces_a_readable_ooxml_container() {
    let bytes = generate(&spec()).expect("generation should succeed");

    // A `.docx` is a zip; `PK` is the local-file-header signature. This is the
    // check that any OOXML reader can open the file at all.
    assert_eq!(&bytes[0..2], b"PK", "must start with the zip magic PK");
    assert!(
        bytes.len() > 200,
        "unexpectedly small ({} bytes)",
        bytes.len()
    );

    let names = entry_names(&bytes);
    for required in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
        assert!(
            names.iter().any(|n| n == required),
            "missing OOXML entry {required} (got {names:?})"
        );
    }
    // Numbering was used, so the numbering part must materialise.
    assert!(
        names.iter().any(|n| n == "word/numbering.xml"),
        "a bullet list should emit word/numbering.xml (got {names:?})"
    );
}

#[test]
fn generate_carries_every_text_field_into_the_body() {
    let s = DocumentSpec {
        title: "Project Charter".to_string(),
        author: Some("Alice".to_string()),
        sections: vec![
            DocumentSection {
                heading: Some("Overview".to_string()),
                paragraphs: vec!["This document describes the plan.".to_string()],
                bullets: vec![],
            },
            DocumentSection {
                heading: Some("Goals".to_string()),
                paragraphs: vec![],
                bullets: vec!["Ship v1".to_string(), "Delight users".to_string()],
            },
        ],
    };
    let body = entry_body(
        &generate(&s).expect("generation should succeed"),
        "word/document.xml",
    );
    for needle in [
        "Project Charter",
        "Alice",
        "Overview",
        "This document describes the plan.",
        "Goals",
        "Ship v1",
        "Delight users",
    ] {
        assert!(
            body.contains(needle),
            "document.xml missing text {needle:?}"
        );
    }
}

#[test]
fn generate_drops_blank_paragraphs_and_bullets() {
    // Whitespace-only entries must not fail generation and must not emit empty
    // runs — they are trimmed away.
    let s = DocumentSpec {
        title: "Trimmed".to_string(),
        author: Some("   ".to_string()),
        sections: vec![DocumentSection {
            heading: Some("Kept".to_string()),
            paragraphs: vec!["real".to_string(), "   ".to_string(), String::new()],
            bullets: vec!["item".to_string(), "\t\n".to_string()],
        }],
    };
    let body = entry_body(
        &generate(&s).expect("generation should succeed"),
        "word/document.xml",
    );
    assert!(body.contains("real"));
    assert!(body.contains("item"));
    assert!(body.contains("Kept"));

    // Presence alone would still pass if the blank-filtering guards in `build`
    // regressed and started emitting empty/whitespace runs alongside the kept
    // ones. Pin the exact paragraph count too: title, heading, one kept
    // paragraph, one kept bullet — no run for the blank author, the two blank
    // paragraphs, or the whitespace-only bullet.
    assert_eq!(
        body.matches("<w:p ").count(),
        4,
        "blank author/paragraphs/bullet must not emit paragraphs: {body}"
    );
    assert_eq!(
        body.matches("<w:t ").count(),
        4,
        "blank author/paragraphs/bullet must not emit text runs: {body}"
    );
    for dropped in ["\t\n", "   "] {
        assert!(
            !body.contains(dropped),
            "dropped whitespace-only content {dropped:?} leaked into document.xml"
        );
    }
}

#[test]
fn generate_validates_before_synthesising() {
    let mut s = spec();
    s.title = String::new();
    assert!(matches!(generate(&s), Err(Error::InvalidInput { .. })));
}
