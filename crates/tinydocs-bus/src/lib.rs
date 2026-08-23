//! The transport-free `TinyBus` wire contract for `TinyDocs`.
//!
//! A `TinyBus` host loads the `tinydocs-module` dynamic library, but it cannot
//! import Rust types from that binary. This crate supplies the shared
//! vocabulary: bus identity, member names, versioning, and the serializable
//! document specification. It deliberately has no dependency on `tinybus`, an
//! async runtime, or document generation.
//!
//! `tinydocs` depends on and re-exports these types, so
//! `tinydocs::docx::DocumentSpec` and [`spec::DocumentSpec`] are identical.

pub mod error;
pub mod names;
pub mod spec;
pub mod version;

pub use error::{Error, Result};
pub use names::{BUS_NAME, METHODS, OBJECT_PATH};
pub use spec::{
    DocumentSection, DocumentSpec, ImageFormat, PresentationSpec, SlideImage, SlideSpec,
    WirePresentationSpec, WireSlideImage, WireSlideSpec,
};
pub use version::{CONTRACT_VERSION, is_compatible};
