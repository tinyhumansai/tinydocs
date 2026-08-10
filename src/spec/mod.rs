//! The wire contracts: typed document specs and their validation, with no
//! dependency on any format writer.
//!
//! Every format module in this crate (`docx`, …) synthesises bytes from a spec
//! defined here. The split matters for two reasons:
//!
//! 1. **A host can share the contract without paying for the codec.** This
//!    module is `serde` plus the crate [`Error`](crate::Error) — nothing else.
//!    It is compiled in *every* build, including
//!    `--no-default-features`, so a host whose synthesis happens elsewhere (in
//!    another process, or behind a message bus) still gets the one authoritative
//!    definition of the spec instead of re-declaring it and drifting.
//! 2. **Validation is cheap and belongs at the boundary.** The specs validate
//!    themselves without touching a writer, so a host can reject a malformed
//!    LLM tool call before paying for a blocking hop or a round trip.
//!
//! The format modules re-export the types they consume, so
//! `tinydocs::docx::DocumentSpec` and [`tinydocs::spec::DocumentSpec`] name the
//! same type.
//!
//! [`tinydocs::spec::DocumentSpec`]: DocumentSpec

mod document;

pub use document::{
    DocumentSection, DocumentSpec, MAX_BULLETS_PER_SECTION, MAX_PARAGRAPH_CHARS,
    MAX_PARAGRAPHS_PER_SECTION, MAX_SECTIONS, MAX_TEXT_CHARS, MAX_TOTAL_CHARS,
};

#[cfg(test)]
mod test;
