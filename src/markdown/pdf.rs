//! A PDF's text layer, page by page.

use super::unreadable;
use crate::Result;

/// Extracts a PDF's text layer, one string per page, any of which may be
/// empty.
///
/// A scanned PDF has none: the file was read, it simply carries pictures of
/// words. That comes back as empty pages, and the converter reports it as a
/// document with no text rather than as a parse failure.
pub(super) fn extract(bytes: &[u8]) -> Result<Vec<String>> {
    // `pdf-extract` panics on some malformed documents rather than erroring.
    // Caught so one bad file is one refused document, not a crashed task.
    match std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes)) {
        Ok(Ok(pages)) => Ok(pages),
        Ok(Err(error)) => Err(unreadable(&format!("the PDF could not be read: {error}"))),
        Err(_) => Err(unreadable(
            "the PDF is malformed enough that the parser gave up on it",
        )),
    }
}
