//! `.docx` (OOXML `WordprocessingML`) synthesis, backed by
//! [`docx-rs`](https://crates.io/crates/docx-rs).
//!
//! [`generate`] turns a validated [`DocumentSpec`] into the bytes of a
//! `.docx` file. It is **synchronous, pure, and CPU-bound**: it touches no
//! filesystem, spawns no subprocess, and knows nothing about deadlines. A
//! host that needs a timeout or a blocking-pool hop owns that policy and wraps
//! this call — which is the whole reason it stays synchronous here, since only
//! the host knows whether it is running on an async executor at all.
//!
//! # Spec → OOXML mapping
//!
//! The document is emitted as a linear paragraph stream:
//!
//! ```text
//! title             → bold, large "Title"-styled paragraph
//! author (opt)      → italic paragraph beneath the title
//! per section:
//!   heading (opt)   → bold "Heading1"-styled paragraph
//!   paragraphs[]    → one normal paragraph each
//!   bullets[]       → single-level `•` list (shared numbering id)
//! ```
//!
//! Headings carry both a style id (`Title` / `Heading1`, which Word maps to
//! its built-in outline styles) and an explicit bold + size run, so the visual
//! hierarchy survives even in a reader that ignores the style table.
//! Whitespace-only paragraphs and bullets are trimmed away rather than
//! emitting an empty run.

mod types;

pub use types::{
    DocumentSection, DocumentSpec, MAX_BULLETS_PER_SECTION, MAX_PARAGRAPH_CHARS,
    MAX_PARAGRAPHS_PER_SECTION, MAX_SECTIONS, MAX_TEXT_CHARS, MAX_TOTAL_CHARS,
};

use docx_rs::{
    AbstractNumbering, Docx, IndentLevel, Level, LevelJc, LevelText, NumberFormat, Numbering,
    NumberingId, Paragraph, Run, Start,
};

use crate::{Error, Result};

/// Shared numbering id for the single-level bullet list.
///
/// One abstract numbering plus one concrete numbering is registered on the
/// document, and every bullet paragraph references it at indent level 0.
const BULLET_NUMBERING_ID: usize = 1;

/// Run font size for the document title, in OOXML half-points (28 pt).
const TITLE_SIZE_HALF_PT: usize = 56;
/// Run font size for a section heading, in half-points (16 pt).
const HEADING_SIZE_HALF_PT: usize = 32;
/// Run font size for the author byline, in half-points (12 pt).
const AUTHOR_SIZE_HALF_PT: usize = 24;

impl DocumentSpec {
    /// Check the spec against every documented size limit.
    ///
    /// Callers do not have to invoke this: [`generate`] validates before it
    /// synthesises anything. It is public so a host can reject a malformed
    /// spec at its own boundary — an LLM tool call, say — and hand back the
    /// structured [`Error::InvalidInput`] before paying for a blocking hop.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] naming the first field that violates a
    /// limit. Fields are checked in spec order (title, author, sections, then
    /// each section's contents) so the reported field is stable for a given
    /// spec.
    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(Error::invalid_input("title", "must not be empty"));
        }
        if self.title.chars().count() > MAX_TEXT_CHARS {
            return Err(Error::invalid_input(
                "title",
                format!("must be ≤ {MAX_TEXT_CHARS} chars"),
            ));
        }
        // Running total across every renderable field — title, author, and all
        // section contents — checked as each field is processed. A spec can pass
        // every per-field limit yet blow the aggregate budget, and checking
        // incrementally rejects it as soon as the budget is crossed without a
        // second pass over the whole spec.
        let over_budget = || {
            Error::invalid_input(
                "sections",
                format!("total document text must be ≤ {MAX_TOTAL_CHARS} chars"),
            )
        };
        let mut total = self.title.chars().count();
        if let Some(author) = self.author.as_deref() {
            if author.chars().count() > MAX_TEXT_CHARS {
                return Err(Error::invalid_input(
                    "author",
                    format!("must be ≤ {MAX_TEXT_CHARS} chars"),
                ));
            }
            total = total.saturating_add(author.chars().count());
        }
        if self.sections.is_empty() {
            return Err(Error::invalid_input(
                "sections",
                "must contain at least one section",
            ));
        }
        if self.sections.len() > MAX_SECTIONS {
            return Err(Error::invalid_input(
                "sections",
                format!("must contain ≤ {MAX_SECTIONS} sections"),
            ));
        }

        for (i, section) in self.sections.iter().enumerate() {
            if section.is_blank() {
                return Err(Error::invalid_input(
                    format!("sections[{i}]"),
                    "must have at least one of heading / paragraphs / bullets",
                ));
            }
            if let Some(heading) = section.heading.as_deref() {
                if heading.chars().count() > MAX_TEXT_CHARS {
                    return Err(Error::invalid_input(
                        format!("sections[{i}].heading"),
                        format!("must be ≤ {MAX_TEXT_CHARS} chars"),
                    ));
                }
                total = total.saturating_add(heading.chars().count());
                if total > MAX_TOTAL_CHARS {
                    return Err(over_budget());
                }
            }
            if section.paragraphs.len() > MAX_PARAGRAPHS_PER_SECTION {
                return Err(Error::invalid_input(
                    format!("sections[{i}].paragraphs"),
                    format!("must contain ≤ {MAX_PARAGRAPHS_PER_SECTION} paragraphs"),
                ));
            }
            for (p, paragraph) in section.paragraphs.iter().enumerate() {
                if paragraph.chars().count() > MAX_PARAGRAPH_CHARS {
                    return Err(Error::invalid_input(
                        format!("sections[{i}].paragraphs[{p}]"),
                        format!("must be ≤ {MAX_PARAGRAPH_CHARS} chars"),
                    ));
                }
                total = total.saturating_add(paragraph.chars().count());
                if total > MAX_TOTAL_CHARS {
                    return Err(over_budget());
                }
            }
            if section.bullets.len() > MAX_BULLETS_PER_SECTION {
                return Err(Error::invalid_input(
                    format!("sections[{i}].bullets"),
                    format!("must contain ≤ {MAX_BULLETS_PER_SECTION} bullets"),
                ));
            }
            for (b, bullet) in section.bullets.iter().enumerate() {
                if bullet.chars().count() > MAX_PARAGRAPH_CHARS {
                    return Err(Error::invalid_input(
                        format!("sections[{i}].bullets[{b}]"),
                        format!("must be ≤ {MAX_PARAGRAPH_CHARS} chars"),
                    ));
                }
                total = total.saturating_add(bullet.chars().count());
                if total > MAX_TOTAL_CHARS {
                    return Err(over_budget());
                }
            }
        }
        Ok(())
    }
}

/// Validate `spec` and synthesise it into `.docx` bytes.
///
/// The returned buffer is a complete OOXML zip container: any Word-compatible
/// reader can open it, and a host can write it straight to disk or stream it.
///
/// Synchronous and CPU-bound. Synthesis of a document at the section cap
/// completes well under 100 ms, but a host on an async executor should still
/// run this on a blocking pool rather than inline.
///
/// # Errors
///
/// - [`Error::InvalidInput`] if `spec` violates any documented limit — no
///   synthesis is attempted.
/// - [`Error::GenerationFailed`] if `docx-rs` fails to pack the document.
///
/// # Examples
///
/// ```
/// use tinydocs::docx::{generate, DocumentSection, DocumentSpec};
///
/// let spec = DocumentSpec {
///     title: "Project Charter".to_string(),
///     author: Some("Alice".to_string()),
///     sections: vec![DocumentSection {
///         heading: Some("Goals".to_string()),
///         paragraphs: vec!["Ship the first release.".to_string()],
///         bullets: vec!["Fast".to_string(), "Correct".to_string()],
///     }],
/// };
///
/// let bytes = generate(&spec)?;
/// assert_eq!(&bytes[0..2], b"PK", "a .docx is a zip container");
/// # Ok::<(), tinydocs::Error>(())
/// ```
pub fn generate(spec: &DocumentSpec) -> Result<Vec<u8>> {
    spec.validate()?;

    // `XMLDocx::pack` takes a `Write + Seek` writer by value; a
    // `&mut Cursor<Vec<u8>>` satisfies both, so we pack into an in-memory
    // buffer and recover the bytes via `into_inner`.
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    build(spec)
        .build()
        .pack(&mut cursor)
        .map_err(|err| Error::generation_failed(&err.to_string()))?;
    Ok(cursor.into_inner())
}

/// Pure transformation from the spec to a `docx-rs` [`Docx`].
///
/// Split out from [`generate`] for unit-testability: the paragraph ordering
/// and blank-filtering rules are load-bearing for the rendered document shape.
fn build(spec: &DocumentSpec) -> Docx {
    // Register the shared single-level bullet list once. `NumberFormat`
    // "bullet" plus a `•` level text renders an unordered list; every bullet
    // paragraph binds to this numbering id at indent level 0.
    let mut docx = Docx::new()
        .add_abstract_numbering(
            AbstractNumbering::new(BULLET_NUMBERING_ID).add_level(Level::new(
                0,
                Start::new(1),
                NumberFormat::new("bullet"),
                LevelText::new("•"),
                LevelJc::new("left"),
            )),
        )
        .add_numbering(Numbering::new(BULLET_NUMBERING_ID, BULLET_NUMBERING_ID));

    // Title — bold and large, styled as the built-in "Title" outline style.
    docx = docx.add_paragraph(
        Paragraph::new().style("Title").add_run(
            Run::new()
                .add_text(spec.title.trim())
                .bold()
                .size(TITLE_SIZE_HALF_PT),
        ),
    );

    // Author byline — italic, when present and non-blank.
    if let Some(author) = spec.author.as_deref().filter(|a| !a.trim().is_empty()) {
        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(author.trim())
                    .italic()
                    .size(AUTHOR_SIZE_HALF_PT),
            ),
        );
    }

    for section in &spec.sections {
        if let Some(heading) = section.heading.as_deref().filter(|h| !h.trim().is_empty()) {
            docx = docx.add_paragraph(
                Paragraph::new().style("Heading1").add_run(
                    Run::new()
                        .add_text(heading.trim())
                        .bold()
                        .size(HEADING_SIZE_HALF_PT),
                ),
            );
        }
        for paragraph in &section.paragraphs {
            let text = paragraph.trim();
            if !text.is_empty() {
                docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(text)));
            }
        }
        for bullet in &section.bullets {
            let text = bullet.trim();
            if !text.is_empty() {
                docx = docx.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(text))
                        .numbering(NumberingId::new(BULLET_NUMBERING_ID), IndentLevel::new(0)),
                );
            }
        }
    }

    docx
}

#[cfg(test)]
mod test;
