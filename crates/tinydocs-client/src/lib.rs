//! Typed host client for the `tinydocs` `TinyBus` module.
//!
//! Module loading, readiness, host policy, and transport ownership stay with
//! the embedding host. The client accepts that host's `ModuleClient`, then
//! owns `TinyDocs` contract calls and held-output transfer.
//!
//! ```no_run
//! use tinydocs_bus::DocumentSpec;
//! use tinydocs_client::{DocumentCallError, TinyDocsClient};
//!
//! async fn create_docx(
//!     client: &TinyDocsClient,
//!     spec: &DocumentSpec,
//! ) -> Result<Vec<u8>, DocumentCallError> {
//!     client.generate_docx(spec).await
//! }
//! ```

mod client;
mod error;

pub use client::TinyDocsClient;
pub use error::DocumentCallError;
