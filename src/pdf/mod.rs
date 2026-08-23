//! Text extraction from `.pdf` documents, backed by
//! [`pdf-extract`](https://crates.io/crates/pdf-extract).
//!
//! This is the one module in the crate that reads rather than writes, and the
//! asymmetry is worth stating plainly: everything else here turns a spec a caller
//! authored into bytes, whereas [`extract_text`] takes a document somebody else
//! produced and recovers what it says. There is no spec and nothing to validate
//! beyond "is this a PDF at all" — the input is arbitrary and often damaged.
//!
//! Like the synthesis modules, this is **synchronous and CPU-bound** and holds no
//! opinion about executors or deadlines. That matters more here than elsewhere:
//! extraction time scales with the document, not with a spec this crate has
//! already bounded, so a host handling untrusted PDFs wants both a blocking-pool
//! hop *and* a timeout. Only the host knows what either should be.
//!
//! # What it does not recover
//!
//! Extraction reads the text layer. A scanned page holds an image of text and no
//! text layer, so it yields nothing — that is not a failure to retry but a
//! document that needs OCR, which is out of scope here. Encrypted documents and
//! damaged cross-reference tables surface as [`Error::ExtractionFailed`].

use crate::{Error, Result};

/// Maximum size, in bytes, of a document [`extract_text`] will accept.
///
/// Extraction allocates well beyond the input size while parsing, and the caller
/// is usually handing over something it did not produce. A host that wants a
/// tighter bound should apply it before calling; this one exists so an
/// unbounded input cannot become an unbounded allocation by default.
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

/// Extract the text layer of the PDF in `bytes`.
///
/// Returns the document's text with the library's own layout decisions intact —
/// no normalisation, trimming, or truncation is applied, because how to bound
/// extracted text is a host policy that depends on what the text is for.
///
/// Synchronous and CPU-bound, and unlike synthesis its cost is set by the input
/// rather than by a validated spec. Run it on a blocking pool under a timeout.
///
/// # Errors
///
/// - [`Error::InvalidInput`] if `bytes` is empty, exceeds
///   [`MAX_DOCUMENT_BYTES`], or does not begin with the `%PDF-` signature.
/// - [`Error::ExtractionFailed`] if the document cannot be parsed — damaged,
///   encrypted, or otherwise unreadable.
///
/// A document that parses cleanly but carries no text layer, such as a scan, is
/// **not** an error: it yields an empty string.
///
/// # Examples
///
/// ```
/// # fn main() -> Result<(), tinydocs::Error> {
/// let not_a_pdf = b"GIF89a";
/// assert!(tinydocs::pdf::extract_text(not_a_pdf).is_err());
/// # Ok(())
/// # }
/// ```
pub fn extract_text(bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() {
        return Err(Error::invalid_input("bytes", "must not be empty"));
    }
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::invalid_input(
            "bytes",
            format!("must be ≤ {MAX_DOCUMENT_BYTES} bytes"),
        ));
    }
    // Checked here rather than left to the parser so that "you handed me a JPEG"
    // is an `InvalidInput` naming the field, not an `ExtractionFailed` carrying
    // a parser's phrasing. The signature may be preceded by junk in the wild,
    // but a leading `%PDF-` is what every conforming producer emits and what the
    // parser needs to find the header.
    if !bytes.starts_with(b"%PDF-") {
        return Err(Error::invalid_input(
            "bytes",
            "must be a PDF document (no %PDF- signature)",
        ));
    }

    pdf_extract::extract_text_from_mem(bytes)
        .map_err(|err| Error::extraction_failed(&err.to_string()))
}

#[cfg(test)]
mod test;
