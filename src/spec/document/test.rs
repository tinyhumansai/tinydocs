//! Unit tests for the wire contracts: validation, the blank/aggregate rules,
//! and JSON round-tripping.
//!
//! These are deliberately separate from the format modules' tests. They must
//! pass in a build with every format feature off, because the spec is the half
//! of the crate a bus- or process-boundary host shares without the codec.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    DocumentSection, DocumentSpec, MAX_BULLETS_PER_SECTION, MAX_PARAGRAPH_CHARS,
    MAX_PARAGRAPHS_PER_SECTION, MAX_SECTIONS, MAX_TEXT_CHARS, MAX_TOTAL_CHARS,
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
    // Each individual field is within its own limit; only the sum is not. One
    // section with just enough max-length paragraphs to cross MAX_TOTAL_CHARS
    // reproduces that without allocating hundreds of megabytes: repeating a
    // whole section MAX_SECTIONS times (the original fixture) built ~512 MB
    // of paragraph text before validation ever ran.
    let paragraph_count = MAX_TOTAL_CHARS / MAX_PARAGRAPH_CHARS + 1;
    assert!(paragraph_count <= MAX_PARAGRAPHS_PER_SECTION);
    let paragraph = "x".repeat(MAX_PARAGRAPH_CHARS);
    let big = DocumentSection {
        heading: Some("Heading".to_string()),
        paragraphs: vec![paragraph; paragraph_count],
        bullets: vec![],
    };
    let s = DocumentSpec {
        title: "Huge".to_string(),
        author: None,
        sections: vec![big],
    };
    // Sanity: this spec passes every per-field check.
    assert!(s.sections.len() <= MAX_SECTIONS);
    assert_rejects(&s, "sections");
}

#[test]
fn rejects_an_aggregate_overrun_that_a_bullet_crosses() {
    // The heading and paragraph loops each carry their own budget check; so does
    // the bullet loop, and only a spec whose overrun lands on a bullet drives
    // that third branch.
    let bullet = "b".repeat(MAX_PARAGRAPH_CHARS);
    let bullet_count = MAX_TOTAL_CHARS / MAX_PARAGRAPH_CHARS + 1;
    assert!(bullet_count <= MAX_BULLETS_PER_SECTION);
    let s = DocumentSpec {
        title: "Bullets".to_string(),
        author: None,
        sections: vec![DocumentSection {
            heading: None,
            paragraphs: vec![],
            bullets: vec![bullet; bullet_count],
        }],
    };
    assert_rejects(&s, "sections");
}

#[test]
fn rejects_an_aggregate_overrun_that_a_heading_crosses() {
    // Headings cannot reach the aggregate cap on their own: MAX_SECTIONS ×
    // MAX_TEXT_CHARS is 256_000, two orders of magnitude under MAX_TOTAL_CHARS.
    // Driving the heading branch therefore means spending the budget down to a
    // single character of headroom in an earlier section, then letting a
    // perfectly legal heading cross it.
    let title = "Headings";
    let filler_count = MAX_TOTAL_CHARS / MAX_PARAGRAPH_CHARS - 1;
    assert!(filler_count <= MAX_PARAGRAPHS_PER_SECTION);
    let used = title.chars().count() + filler_count * MAX_PARAGRAPH_CHARS;
    // Leave exactly one character of headroom.
    let tail = MAX_TOTAL_CHARS - used - 1;
    assert!(tail <= MAX_PARAGRAPH_CHARS);

    let mut paragraphs = vec!["p".repeat(MAX_PARAGRAPH_CHARS); filler_count];
    paragraphs.push("p".repeat(tail));

    let s = DocumentSpec {
        title: title.to_string(),
        author: None,
        sections: vec![
            DocumentSection {
                heading: None,
                paragraphs,
                bullets: vec![],
            },
            DocumentSection {
                // Two characters against one character of headroom.
                heading: Some("hh".to_string()),
                paragraphs: vec![],
                bullets: vec![],
            },
        ],
    };
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
