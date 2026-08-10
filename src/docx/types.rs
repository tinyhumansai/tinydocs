//! The `.docx` document spec: the typed description a caller hands to
//! [`generate`](super::generate), plus the size limits every spec is
//! validated against.
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

use serde::{Deserialize, Serialize};

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

/// Aggregate cap on the total body text across the whole document, in Unicode
/// scalar values.
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
}
