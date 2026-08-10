//! Unit tests for image identification and header measurement.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{ImageFormat, jpeg_dimensions, png_dimensions};

/// A 1×1 PNG assembled byte-for-byte: signature, `IHDR`, `IDAT`, `IEND`.
///
/// Built literally rather than decoded from base64 so the fixture needs no
/// dependency and the offsets under test are visible in the source.
pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&13u32.to_be_bytes()); // IHDR length
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[0x08, 0x06, 0x00, 0x00, 0x00]); // depth, colour, etc.
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // CRC placeholder
    out.extend_from_slice(&0u32.to_be_bytes()); // empty IDAT
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
    out
}

/// A minimal JPEG: SOI, an APP0 stub, then an SOF0 declaring `height × width`.
pub(crate) fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, // APP0, len=4, 2 payload bytes
        0xFF, 0xC0, 0x00, 0x0B, // SOF0, len=11
        0x08, // precision
    ];
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[0x03, 0x00, 0x00, 0x00]); // components (filler)
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}

#[test]
fn sniffs_png_and_jpeg() {
    assert_eq!(ImageFormat::sniff(&png(1, 1)), Some(ImageFormat::Png));
    assert_eq!(ImageFormat::sniff(&jpeg(7, 5)), Some(ImageFormat::Jpeg));
}

#[test]
fn rejects_non_images_and_unembeddable_formats() {
    assert_eq!(ImageFormat::sniff(b"not an image"), None);
    // GIF and WebP are recognisable, but the writer cannot embed either.
    assert_eq!(ImageFormat::sniff(b"GIF89a....."), None);
    assert_eq!(ImageFormat::sniff(b"RIFF\0\0\0\0WEBP"), None);
    assert_eq!(ImageFormat::sniff(&[]), None);
}

#[test]
fn reads_png_dimensions() {
    assert_eq!(ImageFormat::Png.dimensions(&png(1, 1)), Some((1, 1)), "1x1");
    assert_eq!(
        ImageFormat::Png.dimensions(&png(1920, 1080)),
        Some((1920, 1080))
    );
}

#[test]
fn reads_jpeg_dimensions() {
    assert_eq!(ImageFormat::Jpeg.dimensions(&jpeg(7, 5)), Some((7, 5)));
}

#[test]
fn truncated_headers_yield_none() {
    assert_eq!(png_dimensions(&[0x89, 0x50, 0x4E, 0x47]), None);
    assert_eq!(jpeg_dimensions(&[0xFF, 0xD8]), None);
}

#[test]
fn a_png_without_an_ihdr_chunk_yields_none() {
    let mut bytes = png(4, 4);
    bytes[12..16].copy_from_slice(b"XXXX");
    assert_eq!(png_dimensions(&bytes), None);
}

#[test]
fn a_zero_dimension_yields_none() {
    // Degenerate images cannot be placed aspect-correctly; they are rejected
    // rather than divided by.
    assert_eq!(png_dimensions(&png(0, 8)), None);
    assert_eq!(png_dimensions(&png(8, 0)), None);
    assert_eq!(jpeg_dimensions(&jpeg(0, 8)), None);
    assert_eq!(jpeg_dimensions(&jpeg(8, 0)), None);
}

#[test]
fn a_jpeg_with_no_start_of_frame_yields_none() {
    // SOI, then an APP0 segment and EOI — a valid marker stream carrying no
    // frame header at all.
    let bytes = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xD9, 0x00, 0x00,
    ];
    assert_eq!(jpeg_dimensions(&bytes), None);
}

#[test]
fn a_jpeg_with_a_degenerate_segment_length_yields_none() {
    // A declared segment length below the two length bytes themselves would
    // make the walk loop forever if it were trusted.
    let bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01, 0x00, 0x00, 0x00];
    assert_eq!(jpeg_dimensions(&bytes), None);
}

#[test]
fn a_jpeg_skips_standalone_and_non_frame_markers_before_the_frame() {
    // Restart markers and a DHT (0xC4, in the 0xCn range but not a frame
    // header) must both be stepped over rather than mistaken for an SOF.
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xD0, 0xFF, 0xFF];
    bytes.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00]); // DHT
    bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    bytes.extend_from_slice(&11u16.to_be_bytes()); // height
    bytes.extend_from_slice(&22u16.to_be_bytes()); // width
    bytes.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0xFF, 0xD9]);
    assert_eq!(jpeg_dimensions(&bytes), Some((22, 11)));
}

#[test]
fn format_renders_its_ooxml_name() {
    assert_eq!(ImageFormat::Png.as_str(), "PNG");
    assert_eq!(ImageFormat::Jpeg.as_str(), "JPEG");
    assert_eq!(ImageFormat::Jpeg.to_string(), "JPEG");
}

#[test]
fn format_round_trips_through_json_as_its_ooxml_name() {
    let json = serde_json::to_string(&ImageFormat::Png).expect("serialises");
    assert_eq!(json, r#""PNG""#);
    let back: ImageFormat = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, ImageFormat::Png);
}
