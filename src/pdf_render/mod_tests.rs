//! PDF rendering boundary tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
#[test]
fn rejects_unbounded_or_duplicate_page_selection() {
    let spec = RenderPdfSpec {
        pages: vec![1, 1],
        max_dimension: 1024,
        max_total_pixels: 4_000_000,
        max_output_bytes: 8_000_000,
    };
    assert!(render(b"%PDF-invalid", &spec).is_err());
}

fn spec() -> RenderPdfSpec {
    RenderPdfSpec {
        pages: vec![2, 1],
        max_dimension: 120,
        max_total_pixels: 1_000_000,
        max_output_bytes: 1_000_000,
    }
}
#[test]
fn renders_only_requested_pages_in_order_with_png_dimensions() {
    let images = render(
        &crate::pdf::fixtures::document(&[("text", false), ("", true), ("mixed", true)], false),
        &spec(),
    )
    .unwrap();
    assert_eq!(images.page_count, 3);
    assert_eq!(images.pages.len(), 2);
    assert_eq!(images.pages[0].page, 2);
    assert_eq!(images.pages[1].page, 1);
    let decoder = png::Decoder::new(std::io::Cursor::new(&images.pages[0].bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert!(
        pixels[..frame.buffer_size()]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128),
        "scanned image must produce non-white raster pixels"
    );
    for page in images.pages {
        assert!(page.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!((page.width, page.height), (80, 120));
    }
}
#[test]
fn refuses_page_pixel_output_dimension_and_encryption_failures() {
    let bytes = crate::pdf::fixtures::document(&[("text", false), ("", true)], false);
    let mut request = spec();
    request.pages = vec![3];
    assert!(render(&bytes, &request).is_err());
    request = spec();
    request.pages = vec![0];
    assert!(render(&bytes, &request).is_err());
    request = spec();
    request.max_total_pixels = 1;
    assert!(render(&bytes, &request).is_err());
    request = spec();
    request.max_output_bytes = 1;
    assert!(render(&bytes, &request).is_err());
    request = spec();
    request.max_dimension = 2049;
    assert!(render(&bytes, &request).is_err());
    assert!(
        render(
            &crate::pdf::fixtures::document(&[("secret", false)], true),
            &spec()
        )
        .is_err()
    );
    assert!(render(b"%PDF-broken", &spec()).is_err());
}
