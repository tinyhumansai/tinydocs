//! Loadable `TinyBus` module adapter for `TinyDocs`.
//!
//! This private workspace crate keeps the vendored `TinyBus` dependency out of
//! the independently published `tinydocs` crate. Its `cdylib` output is the
//! target-specific binary distributed in GitHub releases.

pub mod outputs;
mod service;

pub use outputs::{OutputError, OutputRef, OutputStore, hex_digest};
pub use service::{BUS_NAME, OBJECT_PATH, WirePresentationSpec, WireSlideImage, WireSlideSpec};
