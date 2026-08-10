//! Unit tests for `.pptx` synthesis.
//!
//! The spec's validation and JSON contract are tested in
//! `crate::spec::presentation` — they are format-independent and must pass in a
//! build with this feature off. What is left here is the deck shape: how many
//! slides are emitted, which text survives blank-filtering, the image geometry,
//! and the OOXML container itself.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{PresentationSpec, SlideImage, SlideSpec, build_slides, fit_within, generate};
use crate::Error;
use crate::spec::image::test::png;

fn slide() -> SlideSpec {
    SlideSpec {
        title: "Overview".to_string(),
        body: Some("The situation so far.".to_string()),
        bullets: vec!["A bullet".to_string()],
        speaker_notes: Some("Keep it short.".to_string()),
        images: vec![],
    }
}

fn spec() -> PresentationSpec {
    PresentationSpec {
        title: "Quarterly Review".to_string(),
        author: Some("Alice".to_string()),
        theme: None,
        slides: vec![slide()],
    }
}

fn image(width: u32, height: u32, caption: Option<&str>) -> SlideImage {
    SlideImage::from_bytes(png(width, height), caption.map(str::to_string)).expect("valid png")
}

/// Entry names inside a produced `.pptx` byte buffer.
fn entry_names(bytes: &[u8]) -> Vec<String> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("output is a valid zip");
    (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect()
}

/// One entry's UTF-8 body out of a produced `.pptx`.
fn entry_body(bytes: &[u8], name: &str) -> String {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("output is a valid zip");
    let mut entry = zip.by_name(name).expect("entry present");
    let mut body = String::new();
    std::io::Read::read_to_string(&mut entry, &mut body).unwrap();
    body
}

/// Whether `name` is a slide part rather than one of its relationship parts.
///
/// Slide parts live at `ppt/slides/slideN.xml` and their relationships at
/// `ppt/slides/_rels/slideN.xml.rels`, so excluding the `_rels` directory is
/// exact — and avoids comparing a file extension case-sensitively.
fn is_slide_part(name: &str) -> bool {
    name.starts_with("ppt/slides/slide") && !name.contains("_rels")
}

/// Concatenated bodies of every `ppt/slides/slideN.xml` part.
fn all_slide_xml(bytes: &[u8]) -> String {
    entry_names(bytes)
        .iter()
        .filter(|name| is_slide_part(name))
        .map(|name| entry_body(bytes, name))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn generate_produces_a_readable_ooxml_container() {
    let bytes = generate(&spec()).expect("generation should succeed");

    assert_eq!(&bytes[0..2], b"PK", "must start with the zip magic PK");
    let names = entry_names(&bytes);
    for required in ["[Content_Types].xml", "_rels/.rels"] {
        assert!(
            names.iter().any(|n| n == required),
            "missing OOXML entry {required} (got {names:?})"
        );
    }
    assert!(
        names.iter().any(|n| is_slide_part(n)),
        "no slide parts emitted (got {names:?})"
    );
}

#[test]
fn a_synthetic_title_slide_is_prepended() {
    // The writer treats its `title` argument as core.xml metadata only, so
    // without the prepend the deck would open on the first content slide. The
    // rendered deck therefore holds one more slide than the spec lists.
    let built = build_slides(&spec());
    assert_eq!(built.len(), spec().slides.len() + 1);

    let bytes = generate(&spec()).expect("generation should succeed");
    let slide_parts = entry_names(&bytes)
        .iter()
        .filter(|n| is_slide_part(n))
        .count();
    assert_eq!(slide_parts, 2, "one title slide plus one content slide");
}

#[test]
fn the_title_slide_carries_the_deck_title_and_author() {
    let xml = all_slide_xml(&generate(&spec()).expect("generation should succeed"));
    assert!(xml.contains("Quarterly Review"), "deck title missing");
    assert!(xml.contains("Alice"), "author byline missing");
}

#[test]
fn a_blank_author_emits_no_byline() {
    let mut s = spec();
    s.author = Some("   ".to_string());
    let built = build_slides(&s);
    // The title slide should hold the title and nothing else. Compare against a
    // deck with no author at all rather than reaching into the writer's types.
    let mut without = spec();
    without.author = None;
    assert_eq!(
        format!("{:?}", built[0]),
        format!("{:?}", build_slides(&without)[0]),
        "a whitespace-only author must render the same as no author"
    );
}

#[test]
fn generate_carries_every_text_field_into_the_slides() {
    let mut s = spec();
    s.slides = vec![SlideSpec {
        title: "Highlights".to_string(),
        body: Some("Throughput doubled.".to_string()),
        bullets: vec!["Shipped the parser".to_string(), "Cut latency".to_string()],
        speaker_notes: None,
        images: vec![],
    }];
    let xml = all_slide_xml(&generate(&s).expect("generation should succeed"));
    for needle in [
        "Highlights",
        "Throughput doubled.",
        "Shipped the parser",
        "Cut latency",
    ] {
        assert!(xml.contains(needle), "slide xml missing text {needle:?}");
    }
}

#[test]
fn generate_drops_blank_body_and_bullets() {
    let mut s = spec();
    s.slides = vec![SlideSpec {
        title: "Kept".to_string(),
        body: Some("   ".to_string()),
        bullets: vec!["real".to_string(), "\t\n".to_string(), String::new()],
        speaker_notes: Some("  ".to_string()),
        images: vec![],
    }];
    let bytes = generate(&s).expect("generation should succeed");
    let xml = all_slide_xml(&bytes);
    assert!(xml.contains("Kept"));
    assert!(xml.contains("real"));
    for dropped in ["\t\n", "   "] {
        assert!(
            !xml.contains(dropped),
            "whitespace-only content {dropped:?} leaked into a slide part"
        );
    }
}

#[test]
fn speaker_notes_reach_a_notes_part() {
    let bytes = generate(&spec()).expect("generation should succeed");
    let names = entry_names(&bytes);
    let notes: Vec<_> = names
        .iter()
        .filter(|n| n.contains("notesSlide"))
        .cloned()
        .collect();
    assert!(!notes.is_empty(), "no notes part emitted (got {names:?})");
    let body = entry_body(&bytes, &notes[0]);
    assert!(
        body.contains("Keep it short."),
        "notes text missing from {}",
        notes[0]
    );
}

#[test]
fn generate_validates_before_synthesising() {
    let mut s = spec();
    s.title = String::new();
    assert!(matches!(generate(&s), Err(Error::InvalidInput { .. })));
}

#[test]
fn images_are_embedded_with_their_captions() {
    let mut s = spec();
    s.slides[0].images = vec![image(320, 200, Some("A chart"))];
    let bytes = generate(&s).expect("generation should succeed");
    let names = entry_names(&bytes);
    assert!(
        names.iter().any(|n| n.starts_with("ppt/media/")),
        "no media part emitted (got {names:?})"
    );
    assert!(
        all_slide_xml(&bytes).contains("A chart"),
        "image caption missing from the slide"
    );
}

#[test]
fn a_single_image_is_centred_in_the_band() {
    use super::{
        IMAGE_BAND_BOTTOM_MARGIN_EMU, IMAGE_BAND_TOP_EMU, SIDE_MARGIN_EMU, SLIDE_HEIGHT_EMU,
        SLIDE_WIDTH_EMU, place_single_column,
    };

    let placed = place_single_column(&[image(400, 300, None)]);
    assert_eq!(placed.len(), 1);
    let img = &placed[0].image;

    let content_left = SIDE_MARGIN_EMU;
    let content_width = SLIDE_WIDTH_EMU - 2 * SIDE_MARGIN_EMU;
    let band_height = SLIDE_HEIGHT_EMU - IMAGE_BAND_TOP_EMU - IMAGE_BAND_BOTTOM_MARGIN_EMU;

    // Inside the content column and inside the band, in both axes.
    assert!(img.x >= content_left, "x={} left of the margin", img.x);
    assert!(
        img.x + img.width <= content_left + content_width,
        "image overflows the content column"
    );
    assert!(img.y >= IMAGE_BAND_TOP_EMU, "y={} above the band", img.y);
    assert!(
        img.y + img.height <= IMAGE_BAND_TOP_EMU + band_height,
        "image overflows the band"
    );

    // Centred: the gaps on either side match within a rounding unit.
    let left_gap = img.x - content_left;
    let right_gap = (content_left + content_width) - (img.x + img.width);
    assert!(
        left_gap.abs_diff(right_gap) <= 1,
        "not horizontally centred: {left_gap} vs {right_gap}"
    );
}

#[test]
fn stacked_images_do_not_overlap_and_stay_in_order() {
    use super::{IMAGE_BAND_TOP_EMU, place_single_column};

    let placed = place_single_column(&[
        image(400, 300, None),
        image(400, 300, None),
        image(400, 300, None),
    ]);
    assert_eq!(placed.len(), 3);
    let mut previous_bottom = IMAGE_BAND_TOP_EMU;
    for (i, p) in placed.iter().enumerate() {
        assert!(
            p.image.y >= previous_bottom,
            "image {i} at y={} overlaps the one above (bottom={previous_bottom})",
            p.image.y
        );
        previous_bottom = p.image.y + p.image.height;
    }
}

#[test]
fn no_images_places_nothing() {
    use super::place_single_column;
    assert!(place_single_column(&[]).is_empty());
}

#[test]
fn fit_within_preserves_the_aspect_ratio() {
    // A 2:1 source into a square box fills the width and halves the height.
    assert_eq!(fit_within(2_000, 1_000, 1_000, 1_000), (1_000, 500));
    // A 1:2 source into a square box fills the height.
    assert_eq!(fit_within(1_000, 2_000, 1_000, 1_000), (500, 1_000));
    // An exact fit is unchanged.
    assert_eq!(fit_within(800, 600, 800, 600), (800, 600));
}

#[test]
fn fit_within_upscales_a_small_source_to_the_box() {
    // The scale factor is the min of both ratios, so a source smaller than the
    // box grows to touch it on one axis without distorting.
    assert_eq!(fit_within(100, 50, 1_000, 1_000), (1_000, 500));
}

#[test]
fn fit_within_clamps_and_never_returns_zero() {
    // A degenerate source has no ratio to preserve, so it falls back to the box.
    assert_eq!(fit_within(0, 100, 640, 480), (640, 480));
    assert_eq!(fit_within(100, 0, 640, 480), (640, 480));
    // An extremely wide source still yields at least one EMU on the short axis
    // rather than a zero-height image the reader would reject.
    let (w, h) = fit_within(1_000_000, 1, 100, 100);
    assert!(w >= 1 && h >= 1, "got {w}x{h}");
    assert!(w <= 100 && h <= 100, "got {w}x{h}, outside the box");
}
