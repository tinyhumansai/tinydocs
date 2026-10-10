//! PNG/JPEG image interpretation owned by the TinyDocs implementation.

use tinydocs_bus::{Error, ImageFacts, ImageFormat, Result, SlideImage};

use crate::spec::{PresentationSpec, presentation::MAX_IMAGE_BYTES};

/// Identify a bounded PNG/JPEG and return its serialized format and dimensions.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for empty, oversized, unsupported, truncated,
/// malformed, or zero-dimension image bytes.
pub fn inspect(bytes: &[u8]) -> Result<ImageFacts> {
    if bytes.is_empty() {
        return Err(Error::invalid_input("bytes", "must not be empty"));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Error::invalid_input(
            "bytes",
            format!("must be ≤ {MAX_IMAGE_BYTES} bytes"),
        ));
    }
    let format =
        sniff(bytes).ok_or_else(|| Error::invalid_input("bytes", "must be a PNG or JPEG image"))?;
    let (width_px, height_px) = dimensions(format, bytes).ok_or_else(|| {
        Error::invalid_input(
            "bytes",
            format!("{format} header is truncated or malformed"),
        )
    })?;
    Ok(ImageFacts {
        format,
        width_px,
        height_px,
    })
}

/// Build the shared slide-image value from implementation-verified bytes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] when the image exceeds the shared limit or
/// its encoded header is unsupported or malformed.
pub fn slide_image_from_bytes(bytes: Vec<u8>, caption: Option<String>) -> Result<SlideImage> {
    let facts = inspect(&bytes)?;
    Ok(SlideImage {
        bytes,
        format: facts.format,
        width_px: facts.width_px,
        height_px: facts.height_px,
        caption,
    })
}

/// Verify image metadata in a typed deck against its encoded bytes.
///
/// This is called by the writer after the vocabulary-level size/field checks.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] naming an image field whose declared facts
/// disagree with its encoded header.
pub fn validate_presentation_images(spec: &PresentationSpec) -> Result<()> {
    for (slide_index, slide) in spec.slides.iter().enumerate() {
        for (image_index, image) in slide.images.iter().enumerate() {
            let field = format!("slides[{slide_index}].images[{image_index}]");
            let facts = inspect(&image.bytes).map_err(|error| match error {
                Error::InvalidInput { reason, .. } => {
                    Error::invalid_input(format!("{field}.bytes"), reason)
                }
                other => other,
            })?;
            if facts.format != image.format {
                return Err(Error::invalid_input(
                    format!("{field}.format"),
                    format!(
                        "declared {} but the bytes are {}",
                        image.format, facts.format
                    ),
                ));
            }
            if (facts.width_px, facts.height_px) != (image.width_px, image.height_px) {
                return Err(Error::invalid_input(
                    format!("{field}.width_px"),
                    format!(
                        "declared {}x{} but the bytes are {}x{}",
                        image.width_px, image.height_px, facts.width_px, facts.height_px
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

fn dimensions(format: ImageFormat, bytes: &[u8]) -> Option<(u32, u32)> {
    match format {
        ImageFormat::Png => png_dimensions(bytes),
        ImageFormat::Jpeg => jpeg_dimensions(bytes),
    }
}

/// PNG: 8-byte signature, then an `IHDR` chunk with big-endian dimensions.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (width != 0 && height != 0).then_some((width, height))
}

/// JPEG: walk marker segments until a start-of-frame segment supplies dimensions.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut index = 2;
    while index + 3 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        index += 2;
        if marker == 0xFF
            || marker == 0x01
            || marker == 0xD8
            || marker == 0xD9
            || (0xD0..=0xD7).contains(&marker)
        {
            continue;
        }
        if index + 1 >= bytes.len() {
            return None;
        }
        let segment_len = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        if segment_len < 2 {
            return None;
        }
        let is_sof = matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        );
        if is_sof {
            if index + 6 >= bytes.len() {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]));
            let width = u32::from(u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]));
            return (width != 0 && height != 0).then_some((width, height));
        }
        index += segment_len;
    }
    None
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
