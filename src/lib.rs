//! Agent-friendly document synthesis in Rust.
//!
//! `tinydocs` turns a typed, validated document spec into real office-format
//! bytes. It is built for hosts that let a language model produce documents:
//! the spec types are the JSON tool schema, validation rejects a malformed
//! spec with a structured [`Error::InvalidInput`] naming the exact field so
//! the model can self-correct, and synthesis returns a plain byte buffer.
//!
//! # What this crate deliberately does not do
//!
//! No filesystem access, no subprocesses, no async runtime, no deadline
//! handling. [`docx::generate`] is synchronous and CPU-bound. A host that runs
//! on an async executor owns the blocking-pool hop and the timeout, because
//! only the host knows its own executor and deadline policy — and a crate that
//! guessed at either would be wrong for every other host.
//!
//! # Layout
//!
//! - [`error`](self::Error) — the crate-wide [`Error`] and [`Result`].
//! - [`docx`] — `.docx` (OOXML `WordprocessingML`) synthesis.
//!
//! # Example
//!
//! ```
//! use tinydocs::docx::{self, DocumentSection, DocumentSpec};
//!
//! let spec = DocumentSpec {
//!     title: "Weekly Report".to_string(),
//!     author: Some("Ferris".to_string()),
//!     sections: vec![DocumentSection {
//!         heading: Some("Highlights".to_string()),
//!         paragraphs: vec!["Throughput doubled.".to_string()],
//!         bullets: vec!["Shipped the parser".to_string()],
//!     }],
//! };
//!
//! let bytes = docx::generate(&spec)?;
//! std::fs::write("report.docx", bytes)?;
//! # std::fs::remove_file("report.docx")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Feature flags
//!
//! - `docx` (default) — `.docx` synthesis via `docx-rs`. Turning it off drops
//!   the whole OOXML writer stack.

mod error;

#[cfg(feature = "docx")]
pub mod docx;

pub use error::{Error, Result};
