//! The wire contracts: typed document specs and their validation, with no
//! dependency on any format writer.
//!
//! Every format module in this crate (`docx`, `pptx`, …) synthesises bytes from
//! a spec defined here. The split matters for two reasons:
//!
//! 1. **A host can share the contract without paying for the codec.** This
//!    module is `serde` plus the crate [`Error`](crate::Error) — nothing else.
//!    It is compiled in *every* build, including `--no-default-features`, so a
//!    host whose synthesis happens elsewhere (in another process, or behind a
//!    message bus) still gets the one authoritative definition of the spec
//!    instead of re-declaring it and drifting.
//! 2. **Validation is cheap and belongs at the boundary.** The specs validate
//!    themselves without touching a writer, so a host can reject a malformed
//!    LLM tool call before paying for a blocking hop or a round trip.
//!
//! # Where things live
//!
//! - [`document`] — `.docx`: [`DocumentSpec`], [`DocumentSection`].
//! - [`presentation`] — `.pptx`: [`PresentationSpec`], [`SlideSpec`],
//!   [`SlideImage`].
//! - [`image`] — [`ImageFormat`], for specs that embed raster images.
//!
//! **Types are re-exported here; limits are not.** Each format's limits stay
//! inside its own module, because the same name means a different thing in each
//! — `document::MAX_TEXT_CHARS` bounds a heading, `presentation::MAX_TEXT_CHARS`
//! bounds a bullet — and flattening them would put two distinct constants under
//! one name. Reach for `spec::presentation::MAX_SLIDES` and read it as the
//! sentence it is.
//!
//! The format modules re-export both the types and the limits they consume, so
//! `tinydocs::docx::DocumentSpec` and [`tinydocs::spec::DocumentSpec`] name the
//! same type.
//!
//! [`tinydocs::spec::DocumentSpec`]: DocumentSpec

pub mod document;
pub mod image;
pub mod presentation;

pub use document::{DocumentSection, DocumentSpec};
pub use image::ImageFormat;
pub use presentation::{PresentationSpec, SlideImage, SlideSpec};
