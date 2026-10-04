//! Selected PDF page rasterization for host-owned vision and OCR.
//!
//! Rendering is synchronous, has no filesystem/network access, and returns PNG
//! bytes. Hosts own isolation, cancellation and deadlines for parser/renderer
//! work; pixel and output budgets bound returned raster allocations.
use crate::{Error, Result};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use std::sync::Arc;
pub use tinydocs_bus::RenderPdfSpec;

/// One selected PNG raster, prior to placement in a module output store.
#[derive(Debug)]
pub struct PdfPageImage {
    /// One-based source page number.
    pub page: u32,
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
    /// Encoded PNG bytes.
    pub bytes: Vec<u8>,
}
/// Selected PNG rasters and total source page count.
#[derive(Debug)]
pub struct PdfImages {
    /// Total pages in the source document.
    pub page_count: u32,
    /// Selected pages in request order.
    pub pages: Vec<PdfPageImage>,
}
/// Rasterize only explicitly selected pages, preserving request order.
///
/// # Errors
/// Rejects malformed/encrypted PDFs, invalid pages, duplicate selections,
/// dimensions outside 1–2048, batches over eight pages and exceeded pixel/PNG
/// byte budgets. No output is returned if any selected page fails.
pub fn render(bytes: &[u8], spec: &RenderPdfSpec) -> Result<PdfImages> {
    validate(spec)?;
    if bytes.len() > crate::pdf::MAX_DOCUMENT_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err(Error::invalid_input(
            "bytes",
            "PDF signature or size is invalid",
        ));
    }
    let document = pdf_extract::Document::load_mem(bytes).map_err(failed)?;
    if document.is_encrypted() || document.trailer.has(b"Encrypt") {
        return Err(Error::extraction_failed("encrypted PDF is not supported"));
    }
    let pdf = Pdf::new(Arc::new(bytes.to_vec())).map_err(|error| failed(format!("{error:?}")))?;
    let page_count = u32::try_from(pdf.pages().len()).map_err(failed)?;
    if page_count > 4096 {
        return Err(Error::extraction_failed("PDF page limit exceeded"));
    }
    let mut plans = Vec::new();
    let mut pixels = 0u64;
    for number in &spec.pages {
        let page = pdf.pages().get((*number - 1) as usize).ok_or_else(|| {
            Error::invalid_input("pages", "page number exceeds document page count")
        })?;
        let (source_width, source_height) = page.render_dimensions();
        if !source_width.is_finite()
            || !source_height.is_finite()
            || source_width <= 0.0
            || source_height <= 0.0
            || !(0.001..=1_000_000.0).contains(&source_width)
            || !(0.001..=1_000_000.0).contains(&source_height)
        {
            return Err(Error::extraction_failed("invalid PDF page dimensions"));
        }
        let (width, height, scale) = dimensions(source_width, source_height, spec.max_dimension);
        pixels = pixels
            .checked_add(u64::from(width) * u64::from(height))
            .ok_or_else(|| Error::invalid_input("max_total_pixels", "pixel count overflow"))?;
        if pixels > spec.max_total_pixels {
            return Err(Error::invalid_input(
                "max_total_pixels",
                "selected pages exceed pixel budget",
            ));
        }
        plans.push((*number, width, height, scale));
    }
    let mut output = PdfImages {
        page_count,
        pages: Vec::new(),
    };
    let mut total_bytes = 0u64;
    for (number, width, height, scale) in plans {
        let page = &pdf.pages()[(number - 1) as usize];
        let settings = hayro::RenderSettings {
            width: Some(width),
            height: Some(height),
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        };
        let pixmap = hayro::render(page, &InterpreterSettings::default(), &settings);
        let bytes = pixmap.into_png().map_err(failed)?;
        total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| Error::extraction_failed("PNG byte count overflow"))?;
        if total_bytes > spec.max_output_bytes {
            return Err(Error::invalid_input(
                "max_output_bytes",
                "rendered PNGs exceed byte budget",
            ));
        }
        output.pages.push(PdfPageImage {
            page: number,
            width: u32::from(width),
            height: u32::from(height),
            bytes,
        });
    }
    Ok(output)
}
fn validate(spec: &RenderPdfSpec) -> Result<()> {
    if spec.pages.is_empty()
        || spec.pages.len() > 8
        || spec.max_dimension == 0
        || spec.max_dimension > 2048
        || spec.max_total_pixels == 0
        || spec.max_total_pixels > 16_000_000
        || spec.max_output_bytes == 0
        || spec.max_output_bytes > 32 * 1024 * 1024
    {
        return Err(Error::invalid_input(
            "spec",
            "render budgets outside hard bounds",
        ));
    }
    for (index, page) in spec.pages.iter().enumerate() {
        if *page == 0 || spec.pages[..index].contains(page) {
            return Err(Error::invalid_input(
                "pages",
                "pages must be positive and unique",
            ));
        }
    }
    Ok(())
}
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "validated finite positive dimensions clamp to 1..=2048 before conversion"
)]
fn dimensions(width: f32, height: f32, max: u32) -> (u16, u16, f32) {
    let scale = max as f32 / width.max(height);
    (
        (width * scale).ceil().clamp(1.0, max as f32) as u16,
        (height * scale).ceil().clamp(1.0, max as f32) as u16,
        scale,
    )
}
fn failed(error: impl std::fmt::Display) -> Error {
    Error::extraction_failed(&error.to_string())
}
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
