//! Unit tests for the presentation wire contract.
//!
//! Format-independent, like the spec itself: these must pass in a build with
//! every format feature off.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    MAX_BULLETS_PER_SLIDE, MAX_IMAGE_BYTES, MAX_IMAGES_PER_DECK, MAX_IMAGES_PER_SLIDE, MAX_SLIDES,
    MAX_TEXT_CHARS, PresentationSpec, SlideImage, SlideSpec,
};
use crate::Error;
use crate::spec::image::ImageFormat;

/// One valid slide carrying a title, a body, and a bullet.
fn slide() -> SlideSpec {
    SlideSpec {
        title: "Overview".to_string(),
        body: Some("The situation so far.".to_string()),
        bullets: vec!["A bullet".to_string()],
        speaker_notes: Some("Keep it short.".to_string()),
        images: vec![],
    }
}

/// A minimal valid spec; each test mutates one field to drive a single branch.
fn spec() -> PresentationSpec {
    PresentationSpec {
        title: "Quarterly Review".to_string(),
        author: Some("Alice".to_string()),
        theme: Some("plain".to_string()),
        slides: vec![slide()],
    }
}

/// A valid image built from real header bytes.
fn image() -> SlideImage {
    SlideImage {
        bytes: vec![1, 2, 3],
        format: ImageFormat::Png,
        width_px: 320,
        height_px: 200,
        caption: Some("A chart".to_string()),
    }
}

/// Assert `spec` is rejected with an `InvalidInput` naming `field`.
fn assert_rejects(spec: &PresentationSpec, field: &str) {
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
fn accepts_a_spec_with_images() {
    let mut s = spec();
    s.slides[0].images = vec![image()];
    assert!(s.validate().is_ok());
}

#[test]
fn rejects_a_blank_deck_title() {
    let mut s = spec();
    s.title = "   ".to_string();
    assert_rejects(&s, "title");
}

#[test]
fn rejects_over_long_deck_level_text() {
    for (field, mutate) in [("title", 0), ("author", 1), ("theme", 2)] {
        let mut s = spec();
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        match mutate {
            0 => s.title = long,
            1 => s.author = Some(long),
            _ => s.theme = Some(long),
        }
        assert_rejects(&s, field);
    }
}

#[test]
fn rejects_a_spec_with_no_slides() {
    let mut s = spec();
    s.slides.clear();
    assert_rejects(&s, "slides");
}

#[test]
fn rejects_too_many_slides() {
    let mut s = spec();
    s.slides = vec![slide(); MAX_SLIDES + 1];
    assert_rejects(&s, "slides");
}

#[test]
fn rejects_a_textless_slide() {
    // Every text entry is present but whitespace-only, so synthesis would drop
    // all of them and render an unlabelled slide.
    let mut s = spec();
    s.slides = vec![SlideSpec {
        title: "  ".to_string(),
        body: Some("\t".to_string()),
        bullets: vec![String::new()],
        speaker_notes: None,
        images: vec![],
    }];
    assert_rejects(&s, "slides[0]");
}

#[test]
fn rejects_a_slide_carrying_only_an_image() {
    // Images do not satisfy the "must have text" rule: an unlabelled slide
    // reads as a rendering bug rather than a design choice.
    let mut s = spec();
    s.slides = vec![SlideSpec {
        title: String::new(),
        body: None,
        bullets: vec![],
        speaker_notes: None,
        images: vec![image()],
    }];
    assert_rejects(&s, "slides[0]");
}

#[test]
fn rejects_over_long_slide_text_naming_its_index() {
    let long = || "x".repeat(MAX_TEXT_CHARS + 1);

    let mut s = spec();
    s.slides.push(SlideSpec {
        title: long(),
        ..slide()
    });
    assert_rejects(&s, "slides[1].title");

    let mut s = spec();
    s.slides[0].body = Some(long());
    assert_rejects(&s, "slides[0].body");

    let mut s = spec();
    s.slides[0].bullets = vec!["ok".to_string(), long()];
    assert_rejects(&s, "slides[0].bullets[1]");

    let mut s = spec();
    s.slides[0].speaker_notes = Some(long());
    assert_rejects(&s, "slides[0].speaker_notes");
}

#[test]
fn rejects_too_many_bullets() {
    let mut s = spec();
    s.slides[0].bullets = vec!["b".to_string(); MAX_BULLETS_PER_SLIDE + 1];
    assert_rejects(&s, "slides[0].bullets");
}

#[test]
fn rejects_too_many_images_on_one_slide() {
    let mut s = spec();
    s.slides[0].images = vec![image(); MAX_IMAGES_PER_SLIDE + 1];
    assert_rejects(&s, "slides[0].images");
}

#[test]
fn rejects_too_many_images_across_the_deck() {
    // Each slide is within the per-slide cap; only the deck total is not. The
    // per-slide cap bounds readability, the deck cap bounds the media payload.
    let per_slide = MAX_IMAGES_PER_SLIDE;
    let slides_needed = MAX_IMAGES_PER_DECK / per_slide + 1;
    let mut s = spec();
    s.slides = vec![
        SlideSpec {
            images: vec![image(); per_slide],
            ..slide()
        };
        slides_needed
    ];
    assert!(s.image_count() > MAX_IMAGES_PER_DECK);
    assert_rejects(&s, "slides[].images");
}

#[test]
fn image_count_sums_across_slides() {
    let mut s = spec();
    s.slides = vec![
        SlideSpec {
            images: vec![image(), image()],
            ..slide()
        },
        SlideSpec {
            images: vec![image()],
            ..slide()
        },
    ];
    assert_eq!(s.image_count(), 3);
}

#[test]
fn rejects_an_over_long_image_caption() {
    let mut s = spec();
    let mut img = image();
    img.caption = Some("c".repeat(MAX_TEXT_CHARS + 1));
    s.slides[0].images = vec![img];
    assert_rejects(&s, "slides[0].images[0].caption");
}

#[test]
fn validate_rejects_empty_and_oversize_image_bytes_without_parsing_them() {
    let mut s = spec();
    let mut img = image();
    img.bytes.clear();
    s.slides[0].images = vec![img];
    assert_rejects(&s, "slides[0].images[0].bytes");

    let mut s = spec();
    let mut img = image();
    img.bytes.resize(MAX_IMAGE_BYTES + 1, 0);
    s.slides[0].images = vec![img];
    assert_rejects(&s, "slides[0].images[0].bytes");
}

#[test]
fn validation_keeps_encoded_image_interpretation_out_of_the_contract() {
    let mut s = spec();
    let mut img = image();
    img.bytes = b"not parsed by the vocabulary crate".to_vec();
    img.width_px += 1;
    s.slides[0].images = vec![img];
    assert!(s.validate().is_ok());
}

#[test]
fn validation_rejects_zero_dimensions_as_a_declared_value_error() {
    let mut s = spec();
    let mut img = image();
    img.width_px = 0;
    s.slides[0].images = vec![img];
    assert_rejects(&s, "slides[0].images[0].width_px");
}

#[test]
fn is_textless_reflects_text_presence() {
    assert!(!slide().is_textless());
    assert!(
        SlideSpec {
            title: String::new(),
            body: None,
            bullets: vec![],
            speaker_notes: None,
            images: vec![],
        }
        .is_textless()
    );
    // A title alone is enough.
    assert!(
        !SlideSpec {
            title: "Only a title".to_string(),
            body: None,
            bullets: vec![],
            speaker_notes: None,
            images: vec![],
        }
        .is_textless()
    );
    // So is a body alone, or a bullet alone.
    assert!(
        !SlideSpec {
            title: String::new(),
            body: Some("Body".to_string()),
            bullets: vec![],
            speaker_notes: None,
            images: vec![],
        }
        .is_textless()
    );
    assert!(
        !SlideSpec {
            title: String::new(),
            body: None,
            bullets: vec!["Bullet".to_string()],
            speaker_notes: None,
            images: vec![],
        }
        .is_textless()
    );
}

#[test]
fn spec_round_trips_through_json() {
    let mut s = spec();
    s.slides[0].images = vec![image()];
    let json = serde_json::to_string(&s).expect("serialises");
    let back: PresentationSpec = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, s);
    assert!(back.validate().is_ok());
}

#[test]
fn spec_rejects_unknown_json_fields() {
    let json = r#"{"title":"T","slides":[],"tilte":"typo"}"#;
    assert!(serde_json::from_str::<PresentationSpec>(json).is_err());
}

#[test]
fn spec_defaults_optional_fields() {
    let s: PresentationSpec = serde_json::from_str(r#"{"title":"T"}"#).expect("deserialises");
    assert_eq!(s.author, None);
    assert_eq!(s.theme, None);
    assert_eq!(s.slides.len(), 0);
}
