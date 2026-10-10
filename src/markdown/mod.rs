//! PDF and Office conversion to the Markdown used by document-memory ingestion.
//!
//! These readers retain TinyMemory's OfficeConverter conversion semantics:
//! normalized paragraphs, numeric slide order, worksheet row labels, and PDF
//! form-feed page breaks. Parsing stays inside the document module; callers
//! supply authorized bytes and retain their ingestion metadata and policy.
//! The implementation is adapted from TinyMemory's GPL-3.0 Office converter.

mod normalize;
mod ooxml;
mod pdf;
mod xlsx;

use crate::intake::DocumentFormat;
use crate::{Error, Result};

/// Largest compressed input accepted for Markdown conversion.
pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
/// Largest declared expansion of an Office archive.
pub const MAX_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
/// Largest dense used range accepted before spreadsheet grid allocation.
pub const MAX_SPREADSHEET_DENSE_CELLS: usize = 1_000_000;
/// Separates PDF pages, including pages with no text layer.
pub const PAGE_BREAK: char = '\u{c}';

/// Convert authorized PDF, DOCX, PPTX or XLSX bytes to normalized Markdown.
///
/// The result is complete rather than silently truncated. Hosts can store it
/// with their existing document provenance. PDF page boundaries are preserved;
/// an entirely scanned/empty document is refused instead of stored as empty.
///
/// # Errors
/// Returns [`Error::InvalidInput`] for empty or oversized input, and
/// [`Error::ExtractionFailed`] for malformed, overexpanding, or empty documents.
pub fn convert(bytes: &[u8], format: DocumentFormat) -> Result<String> {
    if bytes.is_empty() || bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::invalid_input(
            "document",
            "expected 1 to 33554432 bytes",
        ));
    }
    let markdown = match format {
        DocumentFormat::Pdf => pdf::extract(bytes)?
            .iter()
            .map(|page| normalize::normalize(page))
            .collect::<Vec<_>>()
            .join(&PAGE_BREAK.to_string()),
        DocumentFormat::Docx => normalize::normalize(&ooxml::docx(bytes)?),
        DocumentFormat::Pptx => normalize::normalize(&ooxml::pptx(bytes)?),
        DocumentFormat::Xlsx => normalize::normalize(&xlsx::extract(bytes)?),
    };
    if markdown.trim().is_empty() {
        return Err(unreadable(&format!(
            "converting {format:?} produced no text"
        )));
    }
    Ok(markdown)
}

fn unreadable(reason: &str) -> Error {
    Error::extraction_failed(reason)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
