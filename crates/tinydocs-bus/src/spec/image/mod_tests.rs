//! Tests for image vocabulary serialization.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{ImageFacts, ImageFormat};

#[test]
fn format_uses_its_canonical_ooxml_name() {
    assert_eq!(ImageFormat::Png.as_str(), "PNG");
    assert_eq!(ImageFormat::Jpeg.as_str(), "JPEG");
    assert_eq!(ImageFormat::Jpeg.to_string(), "JPEG");
}

#[test]
fn image_format_and_facts_keep_their_serialized_shape() {
    assert_eq!(
        serde_json::to_string(&ImageFormat::Png).unwrap(),
        r#""PNG""#
    );
    let facts = ImageFacts {
        format: ImageFormat::Jpeg,
        width_px: 640,
        height_px: 480,
    };
    let json = serde_json::to_string(&facts).unwrap();
    assert_eq!(json, r#"{"format":"JPEG","width_px":640,"height_px":480}"#);
    assert_eq!(serde_json::from_str::<ImageFacts>(&json).unwrap(), facts);
    assert!(
        serde_json::from_str::<ImageFacts>(
            r#"{"format":"PNG","width_px":1,"height_px":1,"extra":true}"#
        )
        .is_err()
    );
}
