//! Loadable `TinyBus` module adapter for `TinyDocs`.
//!
//! This private workspace crate keeps the vendored `TinyBus` dependency out of
//! the independently published `tinydocs` crate. Its `cdylib` output is the
//! target-specific binary distributed in GitHub releases.

pub mod outputs;
mod service;

/// Entry points for hosts that link `TinyDocs` into their own executable.
#[cfg(feature = "static-link")]
pub mod linked {
    pub use crate::service::exports::{
        TINYBUS_MODULE_ABI_V1, tinybus_module_init_v1, tinybus_module_manifest_v1,
    };
}

pub use outputs::{OutputError, OutputRef, OutputStore, hex_digest};
pub use tinydocs_bus::{
    BUS_NAME, CONTRACT_VERSION, DocumentSection, DocumentSpec, Error, ImageFormat, METHODS,
    OBJECT_PATH, PresentationSpec, Result, SlideImage, SlideSpec, WirePresentationSpec,
    WireSlideImage, WireSlideSpec, is_compatible,
};
