//! Regression tests for image interpretation owned by the `TinyDocs` implementation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::inspect;
use tinydocs_bus::ImageFormat;

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[0x08, 0x06, 0, 0, 0, 0, 0, 0, 0]);
    out
}

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8];
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 0, 0, 0, 0xFF, 0xD9]);
    out
}

#[test]
fn inspects_png_and_jpeg_facts() {
    let png = inspect(&png(1920, 1080)).expect("valid PNG header");
    assert_eq!(png.format, ImageFormat::Png);
    assert_eq!((png.width_px, png.height_px), (1920, 1080));

    let jpeg = inspect(&jpeg(640, 480)).expect("valid JPEG header");
    assert_eq!(jpeg.format, ImageFormat::Jpeg);
    assert_eq!((jpeg.width_px, jpeg.height_px), (640, 480));
}

#[test]
fn rejects_empty_unknown_and_truncated_image_bytes() {
    assert!(inspect(&[]).is_err());
    assert!(inspect(b"GIF89a....").is_err());
    assert!(inspect(&[0x89, 0x50, 0x4E, 0x47]).is_err());
    assert!(inspect(&[0xFF, 0xD8]).is_err());
}

#[test]
fn rejects_zero_dimensions_and_missing_png_ihdr() {
    assert!(inspect(&png(0, 8)).is_err());
    assert!(inspect(&jpeg(8, 0)).is_err());
    let mut bytes = png(4, 4);
    bytes[12..16].copy_from_slice(b"XXXX");
    assert!(inspect(&bytes).is_err());
}

#[test]
fn jpeg_skips_standalone_markers_and_non_frame_segments() {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xD0, 0xFF, 0xFF, 0xFF, 0xC4, 0, 4, 0, 0];
    bytes.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
    bytes.extend_from_slice(&11u16.to_be_bytes());
    bytes.extend_from_slice(&22u16.to_be_bytes());
    bytes.extend_from_slice(&[3, 0, 0, 0]);
    let facts = inspect(&bytes).expect("SOF after standalone and DHT markers");
    assert_eq!((facts.width_px, facts.height_px), (22, 11));
}

#[test]
fn jpeg_treats_tem_as_a_standalone_marker_before_the_frame() {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0x01, 0xFF, 0xC0, 0, 11, 8];
    bytes.extend_from_slice(&33u16.to_be_bytes());
    bytes.extend_from_slice(&44u16.to_be_bytes());
    bytes.extend_from_slice(&[3, 0, 0, 0]);
    let facts = inspect(&bytes).expect("TEM does not carry a length field");
    assert_eq!((facts.width_px, facts.height_px), (44, 33));
}

#[test]
fn rejects_jpeg_without_a_frame_or_with_invalid_segment_length() {
    assert!(inspect(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xD9]).is_err());
    assert!(inspect(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 1, 0, 0]).is_err());
    assert!(inspect(&[0xFF, 0xD8, 0xFF, 0xE0]).is_err());
}

#[test]
fn rejects_inputs_above_the_existing_embedded_image_limit() {
    let mut bytes = png(8, 8);
    bytes.resize(tinydocs_bus::spec::presentation::MAX_IMAGE_BYTES + 1, 0);
    assert!(inspect(&bytes).is_err());
}
