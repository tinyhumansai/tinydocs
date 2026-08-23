//! Typed payloads carried by the `TinyDocs` `GenerateDocx` bus member.
//!
//! The limits are part of the contract so hosts can describe and preflight the
//! same payload bounds enforced by `tinydocs` before it starts generation.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Maximum number of sections a single document may contain.
pub const MAX_SECTIONS: usize = 128;

/// Maximum length, in Unicode scalar values, of a title, author, or heading.
pub const MAX_TEXT_CHARS: usize = 2_000;

/// Maximum length, in Unicode scalar values, of one paragraph or bullet.
pub const MAX_PARAGRAPH_CHARS: usize = 20_000;

/// Maximum number of body paragraphs in a single section.
pub const MAX_PARAGRAPHS_PER_SECTION: usize = 200;

/// Maximum number of bullet-list items in a single section.
pub const MAX_BULLETS_PER_SECTION: usize = 200;

/// Aggregate cap on all renderable text in one document, in Unicode scalars.
pub const MAX_TOTAL_CHARS: usize = 2_000_000;

/// One document section, rendered in spec order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSection {
    /// Optional section heading.
    #[serde(default)]
    pub heading: Option<String>,
    /// Body paragraphs in display order.
    #[serde(default)]
    pub paragraphs: Vec<String>,
    /// Bullet items in display order.
    #[serde(default)]
    pub bullets: Vec<String>,
}

impl DocumentSection {
    /// Returns whether this section has no renderable heading, paragraph, or bullet.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        let has_heading = self
            .heading
            .as_deref()
            .is_some_and(|heading| !heading.trim().is_empty());
        let has_paragraph = self
            .paragraphs
            .iter()
            .any(|paragraph| !paragraph.trim().is_empty());
        let has_bullet = self.bullets.iter().any(|bullet| !bullet.trim().is_empty());
        !(has_heading || has_paragraph || has_bullet)
    }
}

/// A complete `.docx` document specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSpec {
    /// Required document title.
    pub title: String,
    /// Optional author byline.
    #[serde(default)]
    pub author: Option<String>,
    /// Document sections in display order.
    #[serde(default)]
    pub sections: Vec<DocumentSection>,
}

impl DocumentSpec {
    /// Checks this specification against every documented size limit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] naming the first field that violates a
    /// limit. Fields are checked in spec order for a stable result.
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
        for (section_index, section) in self.sections.iter().enumerate() {
            if section.is_blank() {
                return Err(Error::invalid_input(
                    format!("sections[{section_index}]"),
                    "must have at least one of heading / paragraphs / bullets",
                ));
            }
            if let Some(heading) = section.heading.as_deref() {
                if heading.chars().count() > MAX_TEXT_CHARS {
                    return Err(Error::invalid_input(
                        format!("sections[{section_index}].heading"),
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
                    format!("sections[{section_index}].paragraphs"),
                    format!("must contain ≤ {MAX_PARAGRAPHS_PER_SECTION} paragraphs"),
                ));
            }
            for (paragraph_index, paragraph) in section.paragraphs.iter().enumerate() {
                if paragraph.chars().count() > MAX_PARAGRAPH_CHARS {
                    return Err(Error::invalid_input(
                        format!("sections[{section_index}].paragraphs[{paragraph_index}]"),
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
                    format!("sections[{section_index}].bullets"),
                    format!("must contain ≤ {MAX_BULLETS_PER_SECTION} bullets"),
                ));
            }
            for (bullet_index, bullet) in section.bullets.iter().enumerate() {
                if bullet.chars().count() > MAX_PARAGRAPH_CHARS {
                    return Err(Error::invalid_input(
                        format!("sections[{section_index}].bullets[{bullet_index}]"),
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

    /// Returns all renderable text length, in Unicode scalar values.
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

#[cfg(test)]
mod test {
    //! Tests for `TinyDocs` document payload compatibility.

    #![allow(clippy::expect_used)]

    use super::{DocumentSection, DocumentSpec};

    #[test]
    fn specification_round_trips_through_json() {
        let spec = DocumentSpec {
            title: "Contract".to_string(),
            author: Some("TinyDocs".to_string()),
            sections: vec![DocumentSection {
                heading: Some("Payload".to_string()),
                paragraphs: vec!["Shared with hosts.".to_string()],
                bullets: vec![],
            }],
        };
        let json = serde_json::to_string(&spec).expect("spec serializes");
        let parsed: DocumentSpec = serde_json::from_str(&json).expect("spec deserializes");
        assert_eq!(parsed, spec);
    }

    #[test]
    fn specification_rejects_unknown_json_fields() {
        let json = r#"{"title":"T","sections":[],"titel":"typo"}"#;
        assert!(serde_json::from_str::<DocumentSpec>(json).is_err());
    }

    #[test]
    fn specification_defaults_optional_fields() {
        let spec: DocumentSpec =
            serde_json::from_str(r#"{"title":"T"}"#).expect("spec deserializes");
        assert_eq!(spec.author, None);
        assert!(spec.sections.is_empty());
    }
}
