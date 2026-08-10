//! The `.docx` document spec: the typed description a caller hands to
//! `docx::generate`, plus the size limits every spec is validated against.
//!
//! The spec is the crate's wire contract. It derives `Serialize` /
//! `Deserialize` with `deny_unknown_fields` because the usual caller is an
//! LLM tool boundary: the same struct that drives synthesis is the one whose
//! JSON schema the model is shown, and a typo'd field name should be a loud
//! rejection rather than a silently ignored key.
//!
//! Limits are public consts rather than private constants so a host can quote
//! the exact number in its own tool description and stay in lockstep with what
//! validation actually enforces.
//!
//! Nothing in this module depends on the `docx` feature or on `docx-rs`: it is
//! `serde` plus the crate error type. A host that only needs to *describe* and
//! *validate* a document — because synthesis happens elsewhere, in another
//! process or behind a message bus — can therefore depend on this crate with
//! `default-features = false` and still share one definition of the contract.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Maximum number of sections a single document may contain.
///
/// Bounds generation time and output size; a caller with more material is
/// expected to split it across multiple documents.
pub const MAX_SECTIONS: usize = 128;

/// Maximum length, in Unicode scalar values, of a short text field — the
/// document title, the author byline, or a section heading.
pub const MAX_TEXT_CHARS: usize = 2_000;

/// Maximum length, in Unicode scalar values, of a single body paragraph or
/// bullet item.
///
/// More generous than [`MAX_TEXT_CHARS`]: prose paragraphs legitimately run
/// far longer than a heading.
pub const MAX_PARAGRAPH_CHARS: usize = 20_000;

/// Maximum number of body paragraphs in a single section.
pub const MAX_PARAGRAPHS_PER_SECTION: usize = 200;

/// Maximum number of bullet-list items in a single section.
pub const MAX_BULLETS_PER_SECTION: usize = 200;

/// Aggregate cap on all renderable text across the whole document — the
/// title, the author byline, and every section's heading, paragraphs, and
/// bullets — in Unicode scalar values.
///
/// The per-field and per-section limits above bound each individual piece, but
/// not their product — `MAX_SECTIONS × MAX_PARAGRAPHS_PER_SECTION ×
/// MAX_PARAGRAPH_CHARS` alone is over 500M characters, so a spec satisfying
/// every other limit could still build a multi-hundred-megabyte document in
/// memory. This total keeps the worst case bounded to a few megabytes of text
/// while staying generous for any real document.
pub const MAX_TOTAL_CHARS: usize = 2_000_000;

/// One section of the document, rendered in spec order.
///
/// A section is an optional heading followed by any number of body paragraphs
/// and/or a bullet list. At least one of the three must carry renderable text —
/// a wholly blank section is rejected by [`DocumentSpec::validate`] rather than
/// silently rendering nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSection {
    /// Section heading, rendered as a bold heading paragraph. Optional: a
    /// section may be pure body text under the document title.
    #[serde(default)]
    pub heading: Option<String>,
    /// Body paragraphs, each rendered as its own paragraph, in order.
    /// Blank and whitespace-only entries are dropped during synthesis.
    #[serde(default)]
    pub paragraphs: Vec<String>,
    /// Bullet-list items, rendered as a single-level bulleted list after the
    /// section's body paragraphs. Blank and whitespace-only entries are
    /// dropped during synthesis.
    #[serde(default)]
    pub bullets: Vec<String>,
}

impl DocumentSection {
    /// Returns `true` when the section carries no renderable content at all —
    /// the heading is absent or blank, and every paragraph and bullet is blank.
    ///
    /// Synthesis trims and drops blank entries, so a section holding only
    /// `["   "]` would render as nothing despite carrying entries. Validation
    /// uses this to reject that case up front.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        let has_heading = self
            .heading
            .as_deref()
            .is_some_and(|h| !h.trim().is_empty());
        let has_paragraph = self.paragraphs.iter().any(|p| !p.trim().is_empty());
        let has_bullet = self.bullets.iter().any(|b| !b.trim().is_empty());
        !(has_heading || has_paragraph || has_bullet)
    }
}

/// A complete `.docx` document spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSpec {
    /// Document title, rendered as the leading title paragraph. Required and
    /// non-blank.
    pub title: String,
    /// Optional author byline, rendered as an italic line beneath the title.
    #[serde(default)]
    pub author: Option<String>,
    /// Sections, in display order. Must contain at least one entry.
    #[serde(default)]
    pub sections: Vec<DocumentSection>,
}

impl DocumentSpec {
    /// Total renderable text across the whole spec, in Unicode scalar values.
    ///
    /// Sums with saturating arithmetic so an adversarial spec cannot overflow
    /// the counter into a small value that passes the aggregate check.
    #[must_use]
    pub fn total_chars(&self) -> usize {
        let mut total = self.title.chars().count();
        if let Some(author) = self.author.as_deref() {
            total = total.saturating_add(author.chars().count());
        }
        for section in &self.sections {
            if let Some(heading) = section.heading.as_deref() {
                total = total.saturating_add(heading.chars().count());
            }
            for paragraph in &section.paragraphs {
                total = total.saturating_add(paragraph.chars().count());
            }
            for bullet in &section.bullets {
                total = total.saturating_add(bullet.chars().count());
            }
        }
        total
    }

    /// Check the spec against every documented size limit.
    ///
    /// Callers do not have to invoke this: `docx::generate` validates before it
    /// synthesises anything. It is public so a host can reject a malformed
    /// spec at its own boundary — an LLM tool call, say — and hand back the
    /// structured [`Error::InvalidInput`] before paying for a blocking hop, a
    /// process boundary, or a bus round trip.
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
