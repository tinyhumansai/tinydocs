//! Unit tests for `.docx` validation and synthesis.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    DocumentSection, DocumentSpec, MAX_BULLETS_PER_SECTION, MAX_PARAGRAPH_CHARS,
    MAX_PARAGRAPHS_PER_SECTION, MAX_SECTIONS, MAX_TEXT_CHARS, MAX_TOTAL_CHARS, generate,
};
use crate::Error;

/// One valid section carrying a heading, a paragraph, and a bullet.
fn section() -> DocumentSection {
    DocumentSection {
        heading: Some("Overview".to_string()),
        paragraphs: vec!["A body paragraph.".to_string()],
        bullets: vec!["A bullet".to_string()],
    }
}

/// A minimal valid spec; each test mutates one field to drive a single branch.
fn spec() -> DocumentSpec {
    DocumentSpec {
        title: "Charter".to_string(),
        author: Some("Alice".to_string()),
        sections: vec![section()],
    }
}

/// Assert `spec` is rejected with an `InvalidInput` naming `field`.
fn assert_rejects(spec: &DocumentSpec, field: &str) {
    match spec.validate() {
        Err(Error::InvalidInput { field: f, .. }) => {
            assert_eq!(f, field, "unexpected rejected field");
        }
        other => panic!("expected InvalidInput({field}), got {other:?}"),
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
fn accepts_a_well_formed_spec() {
    assert!(spec().validate().is_ok());
}

#[test]
fn rejects_a_blank_title() {
    let mut s = spec();
    s.title = "   ".to_string();
    assert_rejects(&s, "title");
}

#[test]
fn rejects_an_over_long_title() {
    let mut s = spec();
    s.title = "t".repeat(MAX_TEXT_CHARS + 1);
    assert_rejects(&s, "title");
}

#[test]
fn rejects_an_over_long_author() {
    let mut s = spec();
    s.author = Some("a".repeat(MAX_TEXT_CHARS + 1));
    assert_rejects(&s, "author");
}

#[test]
fn rejects_a_spec_with_no_sections() {
    let mut s = spec();
    s.sections.clear();
    assert_rejects(&s, "sections");
}

#[test]
fn rejects_too_many_sections() {
    let mut s = spec();
    s.sections = vec![section(); MAX_SECTIONS + 1];
    assert_rejects(&s, "sections");
}

#[test]
fn rejects_a_wholly_blank_section() {
    // Every entry is present but whitespace-only, so synthesis would drop all
    // of them and render nothing. Validation catches it instead.
    let mut s = spec();
    s.sections = vec![DocumentSection {
        heading: Some("  ".to_string()),
        paragraphs: vec!["\t".to_string()],
        bullets: vec![String::new()],
    }];
    assert_rejects(&s, "sections[0]");
}

#[test]
fn rejects_an_over_long_heading_naming_its_index() {
    let mut s = spec();
    s.sections.push(DocumentSection {
        heading: Some("h".repeat(MAX_TEXT_CHARS + 1)),
        ..section()
    });
    assert_rejects(&s, "sections[1].heading");
}

#[test]
fn rejects_too_many_paragraphs() {
    let mut s = spec();
    s.sections[0].paragraphs = vec!["p".to_string(); MAX_PARAGRAPHS_PER_SECTION + 1];
    assert_rejects(&s, "sections[0].paragraphs");
}

#[test]
fn rejects_an_over_long_paragraph_naming_its_index() {
    let mut s = spec();
    s.sections[0].paragraphs = vec!["ok".to_string(), "p".repeat(MAX_PARAGRAPH_CHARS + 1)];
    assert_rejects(&s, "sections[0].paragraphs[1]");
}

#[test]
fn rejects_too_many_bullets() {
    let mut s = spec();
    s.sections[0].bullets = vec!["b".to_string(); MAX_BULLETS_PER_SECTION + 1];
    assert_rejects(&s, "sections[0].bullets");
}

#[test]
fn rejects_an_over_long_bullet_naming_its_index() {
    let mut s = spec();
    s.sections[0].bullets = vec!["ok".to_string(), "b".repeat(MAX_PARAGRAPH_CHARS + 1)];
    assert_rejects(&s, "sections[0].bullets[1]");
}

#[test]
fn rejects_a_spec_over_the_aggregate_character_budget() {
    // Each individual field is within its own limit; only the sum is not.
    let paragraph = "x".repeat(MAX_PARAGRAPH_CHARS);
    let big = DocumentSection {
        heading: Some("Heading".to_string()),
        paragraphs: vec![paragraph; MAX_PARAGRAPHS_PER_SECTION],
        bullets: vec![],
    };
    let s = DocumentSpec {
        title: "Huge".to_string(),
        author: None,
        sections: vec![big; MAX_SECTIONS],
    };
    // Sanity: this spec passes every per-field check.
    assert!(s.sections.len() <= MAX_SECTIONS);
    assert_rejects(&s, "sections");
}

#[test]
fn is_blank_reflects_content_presence() {
    assert!(!section().is_blank());
    assert!(
        DocumentSection {
            heading: None,
            paragraphs: vec![],
            bullets: vec![],
        }
        .is_blank()
    );
    // A heading alone is enough content.
    assert!(
        !DocumentSection {
            heading: Some("Only a heading".to_string()),
            paragraphs: vec![],
            bullets: vec![],
        }
        .is_blank()
    );
}

#[test]
fn total_chars_sums_every_text_field() {
    let s = DocumentSpec {
        title: "abcd".to_string(),      // 4
        author: Some("xy".to_string()), // 2
        sections: vec![DocumentSection {
            heading: Some("hij".to_string()),   // 3
            paragraphs: vec!["pq".to_string()], // 2
            bullets: vec!["b".to_string()],     // 1
        }],
    };
    assert_eq!(s.total_chars(), 12);
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

#[test]
fn spec_round_trips_through_json() {
    let s = spec();
    let json = serde_json::to_string(&s).expect("serialises");
    let back: DocumentSpec = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, s);
}

#[test]
fn spec_rejects_unknown_json_fields() {
    // `deny_unknown_fields` makes a typo'd key a loud rejection rather than a
    // silently ignored one — the whole point at an LLM tool boundary.
    let json = r#"{"title":"T","sections":[],"titel":"typo"}"#;
    assert!(serde_json::from_str::<DocumentSpec>(json).is_err());
}

#[test]
fn spec_defaults_optional_fields() {
    let s: DocumentSpec = serde_json::from_str(r#"{"title":"T"}"#).expect("deserialises");
    assert_eq!(s.author, None);
    assert!(s.sections.is_empty());
}
