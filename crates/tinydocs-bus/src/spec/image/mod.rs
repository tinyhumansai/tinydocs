//! Raster-image format vocabulary used by presentation specs.

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
}

/// PNG/JPEG facts interpreted by the `TinyDocs` implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageFacts {
    /// Format that the implementation identified from the encoded header.
    pub format: ImageFormat,
    /// Native image width in pixels.
    pub width_px: u32,
    /// Native image height in pixels.
    pub height_px: u32,
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
