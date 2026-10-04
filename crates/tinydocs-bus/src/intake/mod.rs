//! Bounded document intake and selected PDF rasterization contracts.
use serde::{Deserialize, Serialize};

/// Supported document formats; callers identify the format before sending bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    /// PDF text layers, preserving page boundaries.
    Pdf,
    /// Word document body.
    Docx,
    /// `PowerPoint` slide text.
    Pptx,
    /// Excel worksheet cells.
    Xlsx,
}
/// Extraction budgets, subject to library hard ceilings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractDocumentSpec {
    /// Input document format.
    pub format: DocumentFormat,
    /// Total UTF-8 text bytes, at most 200,000.
    pub max_text_bytes: u32,
    /// Maximum returned sections, at most 256.
    pub max_sections: u32,
}
impl ExtractDocumentSpec {
    /// Default bounded extraction for a format.
    #[must_use]
    pub const fn new(format: DocumentFormat) -> Self {
        Self {
            format,
            max_text_bytes: 100_000,
            max_sections: 128,
        }
    }
}
/// One document body, slide, worksheet or PDF page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSectionText {
    /// Stable document part path or `page:N` label.
    pub source: String,
    /// One-based page/slide/sheet index.
    pub index: u32,
    /// Visible extracted text; empty text is preserved.
    pub text: String,
    /// A PDF page with no text layer; a host may offer vision/OCR.
    pub scanned_candidate: bool,
}
/// Extraction result, including explicit text/section truncation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedDocument {
    /// Input format.
    pub format: DocumentFormat,
    /// Total number of document parts/pages before section truncation.
    pub section_count: u32,
    /// Extracted sections in document order.
    pub sections: Vec<DocumentSectionText>,
    /// True when text or sections were omitted by a budget.
    pub truncated: bool,
}
/// PDF page rendering budgets; pages are explicitly selected and one-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPdfSpec {
    /// Selected page numbers, at most eight, without duplicates.
    pub pages: Vec<u32>,
    /// Longest raster edge, at most 2048 pixels.
    pub max_dimension: u32,
    /// Aggregate pixel budget, at most 16 million.
    pub max_total_pixels: u64,
    /// Aggregate encoded PNG budget, at most 32 MiB.
    pub max_output_bytes: u64,
}
/// Handle for an output retained until released or expired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    /// Opaque output capability.
    pub output_id: String,
    /// Byte length of the output.
    pub total_bytes: u64,
    /// Lowercase hex SHA-256 of the output.
    pub sha256: String,
}
/// One PNG raster held through the existing output lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderedPdfPage {
    /// One-based source page.
    pub page: u32,
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
    /// PNG bytes read with `ReadOutput` and freed with `ReleaseOutput`.
    pub output: OutputRef,
}
/// Selected PDF rasters and source page count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderedPdf {
    /// Total source pages.
    pub page_count: u32,
    /// Selected pages in request order.
    pub pages: Vec<RenderedPdfPage>,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
