//! Holding a produced document until the caller has read it.
//!
//! # Why this exists at all
//!
//! Inbound payloads do not need it: `TinyBus` streams carry a `.pdf` or a slide
//! image alongside the method call, flow-controlled and bounded by the receiver's
//! own [`StreamLimits`](tinybus::stream::StreamLimits), and the transfer is tied
//! to the call that started it.
//!
//! Replies have no such thing. `Interface::call` receives a member name and a
//! JSON body — no caller identity, no connection — so a served object cannot open
//! a stream back to whoever called it. A generated `.docx` is therefore held here
//! and pulled in chunks, because the alternative is returning it inline through a
//! 16 MiB JSON frame where a `Vec<u8>` costs ~3.5 bytes per byte.
//!
//! That asymmetry is worth fixing upstream rather than working around forever: a
//! reply-stream seam in `TinyBus` would delete this module.
//!
//! # The bounds are the whole design
//!
//! A module is trusted in-process code that `TinyBus` never unloads, so anything
//! retained here is retained until the process exits unless something reclaims
//! it. A caller that asks for a document and then dies must not cost the host
//! that document forever. Hence a per-output cap, a total cap, a count cap, and
//! expiry of outputs nobody has read.
//!
//! Expiry is lazy — every operation sweeps first — so there is no background task
//! and no timer to reason about, and the clock is a parameter rather than a call
//! to [`Instant::now`], which is what makes the rules testable.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Largest chunk a caller may read in one `ReadOutput`.
///
/// Sized so the chunk plus its base64 expansion and the surrounding JSON stays
/// well inside a 16 MiB frame.
pub const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Largest single produced document.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// Largest total of all unread documents.
pub const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

/// Most documents held unread at once.
///
/// A separate bound from the byte budget: many small abandoned outputs are as
/// much of a leak as one large one.
pub const MAX_LIVE_OUTPUTS: usize = 32;

/// How long an output may go unread before it is dropped.
pub const IDLE_TTL: Duration = Duration::from_secs(300);

/// A handle to a produced document, and what a caller needs to read it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    /// Opaque identifier, valid until read and released or expired.
    pub output_id: String,
    /// Total size in bytes, so a caller knows when it is done.
    pub total_bytes: u64,
    /// Lowercase hex SHA-256, so a caller can verify what it assembled.
    pub sha256: String,
}

/// Why an output operation was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    /// The produced document exceeds [`MAX_OUTPUT_BYTES`].
    #[error("document exceeds the {MAX_OUTPUT_BYTES}-byte per-output limit")]
    OutputTooLarge,

    /// Holding it would exceed [`MAX_TOTAL_BYTES`].
    #[error("too many bytes are waiting to be read")]
    StoreFull,

    /// [`MAX_LIVE_OUTPUTS`] documents are already waiting.
    #[error("too many documents are waiting to be read")]
    TooManyOutputs,

    /// No output with that id — read and released, or expired unread.
    #[error("unknown output id")]
    UnknownOutput,

    /// The requested chunk exceeds [`MAX_CHUNK_BYTES`].
    #[error("chunk exceeds the {MAX_CHUNK_BYTES}-byte per-chunk limit")]
    ChunkTooLarge,

    /// The read started past the end of the document.
    #[error("read offset is past the end of the document")]
    ReadPastEnd,
}

/// One produced document waiting to be read.
struct Output {
    bytes: Vec<u8>,
    last_read: Instant,
}

/// Documents produced but not yet read.
#[derive(Default)]
pub struct OutputStore {
    inner: Mutex<Inner>,
}

/// Reports how much is held, never what is held.
///
/// Written by hand rather than derived: a derived implementation would put a
/// whole document into whatever formatted it. Documents are caller data.
impl std::fmt::Debug for OutputStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("OutputStore")
            .field("live_outputs", &inner.outputs.len())
            .field("held_bytes", &inner.held_bytes())
            .finish()
    }
}

#[derive(Default)]
struct Inner {
    outputs: HashMap<String, Output>,
    next_id: u64,
}

impl OutputStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Hold `bytes` and return the handle a caller reads them back with.
    ///
    /// # Errors
    ///
    /// [`OutputError::OutputTooLarge`], [`OutputError::TooManyOutputs`], or
    /// [`OutputError::StoreFull`].
    pub fn insert(&self, bytes: Vec<u8>, now: Instant) -> Result<OutputRef, OutputError> {
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err(OutputError::OutputTooLarge);
        }

        let mut inner = self.lock();
        inner.sweep_expired(now);
        if inner.outputs.len() >= MAX_LIVE_OUTPUTS {
            return Err(OutputError::TooManyOutputs);
        }
        if inner.held_bytes().saturating_add(bytes.len()) > MAX_TOTAL_BYTES {
            return Err(OutputError::StoreFull);
        }

        let sha256 = hex_digest(&bytes);
        let total_bytes = bytes.len() as u64;
        let output_id = inner.allocate_id();
        inner.outputs.insert(
            output_id.clone(),
            Output {
                bytes,
                last_read: now,
            },
        );
        Ok(OutputRef {
            output_id,
            total_bytes,
            sha256,
        })
    }

    /// Read up to `len` bytes at `offset`.
    ///
    /// A read running past the end is clamped rather than refused, so a caller
    /// can ask for a full chunk on the final read without computing the
    /// remainder itself.
    ///
    /// # Errors
    ///
    /// [`OutputError::ChunkTooLarge`], [`OutputError::UnknownOutput`], or
    /// [`OutputError::ReadPastEnd`].
    pub fn read_chunk(
        &self,
        output_id: &str,
        offset: u64,
        len: u64,
        now: Instant,
    ) -> Result<Vec<u8>, OutputError> {
        let len = usize::try_from(len).map_err(|_| OutputError::ChunkTooLarge)?;
        if len > MAX_CHUNK_BYTES {
            return Err(OutputError::ChunkTooLarge);
        }

        let mut inner = self.lock();
        inner.sweep_expired(now);
        let output = inner
            .outputs
            .get_mut(output_id)
            .ok_or(OutputError::UnknownOutput)?;
        let start = usize::try_from(offset).map_err(|_| OutputError::ReadPastEnd)?;
        if start > output.bytes.len() {
            return Err(OutputError::ReadPastEnd);
        }
        // Reading is what keeps an output alive: a caller working through a
        // large document in chunks must not have it reaped mid-read.
        output.last_read = now;
        let end = start.saturating_add(len).min(output.bytes.len());
        Ok(output.bytes[start..end].to_vec())
    }

    /// Drop an output and free its budget.
    ///
    /// # Errors
    ///
    /// [`OutputError::UnknownOutput`] if there is nothing to release, so a
    /// caller learns its output had already expired rather than assuming it
    /// tidied up.
    pub fn release(&self, output_id: &str, now: Instant) -> Result<(), OutputError> {
        let mut inner = self.lock();
        inner.sweep_expired(now);
        inner
            .outputs
            .remove(output_id)
            .map(|_| ())
            .ok_or(OutputError::UnknownOutput)
    }

    /// Number of outputs currently held, for tests and diagnostics.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.lock().outputs.len()
    }

    /// Take the lock, recovering from a poisoned mutex.
    ///
    /// A panic under this lock can only have happened between two `HashMap`
    /// operations, so the map is intact and the worst case is one stale output
    /// that its TTL will reap. Refusing every later request would turn one
    /// caller's panic into a dead module, and `TinyBus` never unloads a module
    /// to recover.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Inner {
    /// Total bytes held by every output.
    fn held_bytes(&self) -> usize {
        self.outputs
            .values()
            .map(|output| output.bytes.len())
            .fold(0usize, usize::saturating_add)
    }

    /// Drop every output unread for longer than [`IDLE_TTL`].
    fn sweep_expired(&mut self, now: Instant) {
        self.outputs
            .retain(|_, output| now.saturating_duration_since(output.last_read) <= IDLE_TTL);
    }

    /// Allocate an unused output id.
    ///
    /// A counter, not a random value: ids are opaque handles inside one process,
    /// never authorisation tokens, and a counter makes a leaked id visible in a
    /// log rather than looking like a secret.
    fn allocate_id(&mut self) -> String {
        self.next_id = self.next_id.wrapping_add(1);
        format!("out-{}", self.next_id)
    }
}

/// Lowercase hex SHA-256 of `bytes`.
///
/// Public because a caller verifies what it assembled against
/// [`OutputRef::sha256`], and one implementation both sides agree on beats two
/// that can disagree about case.
#[must_use]
pub fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        // Writing into a String cannot fail; the result is discarded rather than
        // unwrapped so this stays panic-free.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod test;
