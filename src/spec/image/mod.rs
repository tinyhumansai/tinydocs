//! Raster-image identification for specs that embed images.
//!
//! Two formats are supported, PNG and JPEG, and the restriction is deliberate
//! rather than incidental: the OOXML presentation writer this crate drives
//! declares no `webp` default in the generated `[Content_Types].xml`, and its
//! automatic format detection misclassifies `webp` as PNG — producing a part
//! `PowerPoint` refuses to render. Accepting only what can actually be embedded
//! turns that into a clean rejection at the boundary.
//!
//! Identification is done by reading the container header directly, in about a
//! hundred lines and with no dependencies, rather than by pulling in a decoding
//! stack. Nothing here decodes pixels: it answers "which format is this" and
//! "what are its native dimensions", which is all a layout engine needs to
//! place an image with the right aspect ratio.
//!
//! Like the rest of [`crate::spec`], this module is compiled in every build. A
//! host resolving image bytes has to identify and measure them to *build* a
//! spec, and that must not require the writer.

use serde::{Deserialize, Serialize};

/// A raster image format that can be embedded in a generated document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ImageFormat {
    /// Portable Network Graphics.
    Png,
    /// JPEG / JFIF.
    Jpeg,
}

impl ImageFormat {
    /// The format's canonical OOXML name — `"PNG"` or `"JPEG"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
        }
    }

    /// Identify `bytes` by its container header.
    ///
    /// Returns `None` for a truncated header or any format other than the two
    /// embeddable ones — including GIF, WebP and BMP, which are recognisable
    /// but not embeddable.
    #[must_use]
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else {
            None
        }
    }

    /// Native `(width, height)` of `bytes` in pixels, read from the header.
    ///
    /// Returns `None` when the header is truncated or malformed, or when either
    /// dimension is zero — a degenerate image cannot be placed aspect-correctly
    /// and is rejected rather than divided by.
    #[must_use]
    pub fn dimensions(self, bytes: &[u8]) -> Option<(u32, u32)> {
        match self {
            Self::Png => png_dimensions(bytes),
            Self::Jpeg => jpeg_dimensions(bytes),
        }
    }
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// PNG: 8-byte signature, then an `IHDR` chunk whose width / height are
/// big-endian `u32`s at byte offsets 16 and 20.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// JPEG: walk the marker segments until a Start-Of-Frame is hit; its payload
/// carries height then width as big-endian `u16`s.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // skip the leading FF D8 SOI
    while i + 3 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        i += 2;
        // Standalone markers (no length field): padding fill bytes and
        // RSTn / SOI / EOI. Skip without consuming a segment length.
        if marker == 0xFF || marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if i + 1 >= bytes.len() {
            return None;
        }
        let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if seg_len < 2 {
            return None;
        }
        // SOF markers carrying frame dimensions. Excludes 0xC4 (DHT),
        // 0xC8 (JPG) and 0xCC (DAC), which share the 0xCn range but are not
        // frame headers.
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
            // segment: [len_hi len_lo precision h_hi h_lo w_hi w_lo ...]
            if i + 6 >= bytes.len() {
                return None;
            }
            let h = u32::from(u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]));
            let w = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
            if w == 0 || h == 0 {
                return None;
            }
            return Some((w, h));
        }
        i += seg_len;
    }
    None
}

// Visible crate-wide under `cfg(test)`: the `png` / `jpeg` header builders here
// are the fixtures every image-carrying spec and every synthesis test needs, and
// one honest builder beats a base64 blob copied into three files.
#[cfg(test)]
pub(crate) mod test;
