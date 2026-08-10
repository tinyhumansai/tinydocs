//! Chunked byte transfer for payloads that do not fit in a bus frame.
//!
//! # Why this exists
//!
//! A `TinyBus` frame is a JSON document capped at 16 MiB, and `TinyBus`'s own
//! guidance is that large payloads travel as paths rather than inline. Neither
//! half of the document surface fits inside that: a deck may legally carry
//! 8 images of 5 MiB each, and a `.pdf` handed in for extraction is bounded only
//! by what the host accepted. Serialising bytes as a JSON array of integers
//! makes it worse — roughly 3.5 bytes of frame per byte of payload — so the
//! real inline ceiling is a few megabytes, not sixteen.
//!
//! So bytes move in chunks, base64-encoded (1.34× rather than 3.5×), through a
//! staging area addressed by opaque blob ids. A caller stages a `.pdf`, calls
//! `ExtractText`, and reads the result back out the same way; the frame size
//! stops being part of the contract.
//!
//! # Every limit here is load-bearing
//!
//! A module is trusted in-process code that `TinyBus` never unloads, so an
//! abandoned upload is not garbage collected by a process exit that never comes.
//! The store therefore bounds four separate things — one chunk, one blob, the
//! whole staging area, and the number of live blobs — and expires blobs that
//! stop being touched. Without the last one, a caller that dies mid-upload leaks
//! its partial blob for the life of the host.
//!
//! Expiry is lazy: every operation sweeps first, so there is no background task
//! and no timer to reason about. The clock is a parameter rather than a call to
//! [`Instant::now`], which is what makes the expiry rules testable at all.
//!
//! # Append-only by construction
//!
//! `put_chunk` requires `offset` to equal exactly how many bytes have arrived so
//! far. That is stricter than necessary, and deliberately so: sparse writes would
//! need range bookkeeping, a definition of what overlapping writes mean, and a
//! way to know when a blob is actually complete. Requiring append makes
//! "complete" mean "length reached", and makes a lost or duplicated chunk a named
//! error at the moment it happens rather than a corrupt blob discovered later.
//!
//! Completion verifies the caller's SHA-256 before the blob becomes readable, so
//! a truncated or reordered transfer cannot be consumed as though it were whole.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum size of a single chunk.
///
/// Sized so that one chunk plus its base64 expansion and the surrounding JSON
/// stays well inside a 16 MiB frame, with room left for a method envelope.
pub const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Maximum size of one staged blob.
pub const MAX_BLOB_BYTES: usize = 64 * 1024 * 1024;

/// Maximum total size of all staged blobs at once.
///
/// Bounds the module's resident memory independently of how many callers are
/// mid-transfer.
pub const MAX_TOTAL_STAGED_BYTES: usize = 128 * 1024 * 1024;

/// Maximum number of blobs alive at once.
///
/// A separate bound from the byte budget: many tiny abandoned blobs are as much
/// of a leak as one large one, and each carries bookkeeping of its own.
pub const MAX_LIVE_BLOBS: usize = 64;

/// How long a blob may go untouched before it is expired.
///
/// Long enough that a slow but live transfer is never reaped, short enough that
/// an abandoned one does not outlive the request that started it by much.
pub const IDLE_TTL: Duration = Duration::from_secs(300);

/// A handle to a complete staged blob, plus what a caller needs to read it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobRef {
    /// Opaque identifier for the blob.
    pub blob_id: String,
    /// Total size in bytes, so a caller knows how many chunks to ask for.
    pub total_bytes: u64,
    /// Lowercase hex SHA-256 of the bytes, so a caller can verify what it read.
    pub sha256: String,
}

/// Why a blob operation was refused.
///
/// Every variant is a distinct condition with a distinct wire name, because a
/// caller's correct response differs: a budget refusal is worth retrying later,
/// a hash mismatch means re-sending, and an unknown id means the blob is gone
/// and the whole transfer has to start again.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlobError {
    /// The declared SHA-256 was not 64 lowercase hexadecimal characters.
    #[error("sha256 must be 64 lowercase hexadecimal characters")]
    MalformedDigest,

    /// The declared total size exceeds [`MAX_BLOB_BYTES`].
    #[error("blob size exceeds the {MAX_BLOB_BYTES}-byte per-blob limit")]
    BlobTooLarge,

    /// A single chunk exceeded [`MAX_CHUNK_BYTES`].
    #[error("chunk exceeds the {MAX_CHUNK_BYTES}-byte per-chunk limit")]
    ChunkTooLarge,

    /// Accepting the blob would exceed [`MAX_TOTAL_STAGED_BYTES`].
    #[error("staging area is full")]
    StagingFull,

    /// [`MAX_LIVE_BLOBS`] blobs are already staged.
    #[error("too many blobs staged at once")]
    TooManyBlobs,

    /// No blob with that id — never staged, released, or expired.
    #[error("unknown blob id")]
    UnknownBlob,

    /// `offset` did not equal the number of bytes received so far.
    #[error("chunk offset {actual} does not continue the blob at {expected}")]
    OutOfOrderChunk {
        /// The offset the next chunk must carry.
        expected: u64,
        /// The offset the caller sent.
        actual: u64,
    },

    /// The chunk would write past the declared total size.
    #[error("chunk would exceed the declared blob size")]
    OverlongBlob,

    /// The assembled bytes did not hash to the declared digest.
    #[error("assembled blob does not match the declared sha256")]
    DigestMismatch,

    /// The blob is still being uploaded and cannot be read yet.
    #[error("blob is incomplete")]
    IncompleteBlob,

    /// A read started past the end of the blob.
    #[error("read offset is past the end of the blob")]
    ReadPastEnd,
}

/// One blob in the staging area.
struct Blob {
    /// Declared total size; the blob is complete when `data` reaches it.
    expected_bytes: usize,
    /// Declared digest, verified once the blob is complete.
    expected_sha256: String,
    data: Vec<u8>,
    complete: bool,
    last_touched: Instant,
}

impl Blob {
    /// Bytes charged against the staging budget.
    ///
    /// The declared total, not the bytes received so far: the budget is reserved
    /// at `begin` so a transfer that is admitted can always finish, rather than
    /// failing halfway when somebody else fills the area.
    fn reserved(&self) -> usize {
        self.expected_bytes.max(self.data.len())
    }
}

/// The staging area shared by every method on the service.
#[derive(Default)]
pub struct BlobStore {
    inner: Mutex<Inner>,
}

/// Reports how much is staged, never what is staged.
///
/// Written by hand rather than derived because a derived implementation would
/// put every staged byte into whatever formatted it — a log line, a panic
/// message, an error. Staged bytes are caller data.
impl std::fmt::Debug for BlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("BlobStore")
            .field("live_blobs", &inner.blobs.len())
            .field("staged_bytes", &inner.staged_bytes())
            .finish()
    }
}

#[derive(Default)]
struct Inner {
    blobs: HashMap<String, Blob>,
    next_id: u64,
}

impl BlobStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve space for a blob of `total_bytes` that will hash to `sha256`.
    ///
    /// # Errors
    ///
    /// [`BlobError::MalformedDigest`], [`BlobError::BlobTooLarge`],
    /// [`BlobError::TooManyBlobs`], or [`BlobError::StagingFull`].
    pub fn begin(&self, total_bytes: u64, sha256: &str, now: Instant) -> Result<String, BlobError> {
        if !is_lowercase_sha256(sha256) {
            return Err(BlobError::MalformedDigest);
        }
        let expected_bytes = usize::try_from(total_bytes).map_err(|_| BlobError::BlobTooLarge)?;
        if expected_bytes > MAX_BLOB_BYTES {
            return Err(BlobError::BlobTooLarge);
        }

        let mut inner = self.lock();
        inner.sweep_expired(now);
        if inner.blobs.len() >= MAX_LIVE_BLOBS {
            return Err(BlobError::TooManyBlobs);
        }
        if inner.staged_bytes().saturating_add(expected_bytes) > MAX_TOTAL_STAGED_BYTES {
            return Err(BlobError::StagingFull);
        }

        let id = inner.allocate_id();
        inner.blobs.insert(
            id.clone(),
            Blob {
                expected_bytes,
                expected_sha256: sha256.to_string(),
                // Not pre-allocated: a caller can declare 64 MiB and never send
                // it, and reserving the allocation up front would make that a
                // way to spend the host's memory for free.
                data: Vec::new(),
                complete: expected_bytes == 0,
                last_touched: now,
            },
        );
        // A zero-length blob is complete on arrival, so its digest is checked
        // here rather than on a chunk that will never come.
        if expected_bytes == 0 {
            let verified = verify(&[], sha256);
            if !verified {
                inner.blobs.remove(&id);
                return Err(BlobError::DigestMismatch);
            }
        }
        Ok(id)
    }

    /// Append `data` at `offset`, returning the number of bytes received so far.
    ///
    /// When the blob reaches its declared size, its digest is verified and it
    /// becomes readable. A mismatch drops the blob.
    ///
    /// # Errors
    ///
    /// [`BlobError::ChunkTooLarge`], [`BlobError::UnknownBlob`],
    /// [`BlobError::OutOfOrderChunk`], [`BlobError::OverlongBlob`], or
    /// [`BlobError::DigestMismatch`].
    pub fn put_chunk(
        &self,
        blob_id: &str,
        offset: u64,
        data: &[u8],
        now: Instant,
    ) -> Result<u64, BlobError> {
        if data.len() > MAX_CHUNK_BYTES {
            return Err(BlobError::ChunkTooLarge);
        }

        let mut inner = self.lock();
        inner.sweep_expired(now);
        let blob = inner.blobs.get_mut(blob_id).ok_or(BlobError::UnknownBlob)?;

        let received = blob.data.len() as u64;
        if blob.complete || offset != received {
            return Err(BlobError::OutOfOrderChunk {
                expected: received,
                actual: offset,
            });
        }
        if blob.data.len().saturating_add(data.len()) > blob.expected_bytes {
            return Err(BlobError::OverlongBlob);
        }

        blob.data.extend_from_slice(data);
        blob.last_touched = now;
        if blob.data.len() == blob.expected_bytes {
            if verify(&blob.data, &blob.expected_sha256) {
                blob.complete = true;
            } else {
                inner.blobs.remove(blob_id);
                return Err(BlobError::DigestMismatch);
            }
        }
        // Re-read rather than reuse the borrow above: the mismatch branch may
        // have removed the entry.
        Ok(inner
            .blobs
            .get(blob_id)
            .map_or(0, |blob| blob.data.len() as u64))
    }

    /// Read up to `len` bytes of a complete blob starting at `offset`.
    ///
    /// A read that runs past the end is clamped rather than refused, so a caller
    /// can ask for a whole chunk on the final read without special-casing the
    /// remainder.
    ///
    /// # Errors
    ///
    /// [`BlobError::UnknownBlob`], [`BlobError::IncompleteBlob`],
    /// [`BlobError::ChunkTooLarge`], or [`BlobError::ReadPastEnd`].
    pub fn get_chunk(
        &self,
        blob_id: &str,
        offset: u64,
        len: u64,
        now: Instant,
    ) -> Result<Vec<u8>, BlobError> {
        let len = usize::try_from(len).map_err(|_| BlobError::ChunkTooLarge)?;
        if len > MAX_CHUNK_BYTES {
            return Err(BlobError::ChunkTooLarge);
        }

        let mut inner = self.lock();
        inner.sweep_expired(now);
        let blob = inner.blobs.get_mut(blob_id).ok_or(BlobError::UnknownBlob)?;
        if !blob.complete {
            return Err(BlobError::IncompleteBlob);
        }
        let start = usize::try_from(offset).map_err(|_| BlobError::ReadPastEnd)?;
        if start > blob.data.len() {
            return Err(BlobError::ReadPastEnd);
        }
        blob.last_touched = now;
        let end = start.saturating_add(len).min(blob.data.len());
        Ok(blob.data[start..end].to_vec())
    }

    /// Stage `bytes` as an already-complete blob and return its handle.
    ///
    /// This is the outbound direction: a generated document or an extracted text
    /// body that the module produced and the caller now has to read back.
    ///
    /// # Errors
    ///
    /// [`BlobError::BlobTooLarge`], [`BlobError::TooManyBlobs`], or
    /// [`BlobError::StagingFull`].
    pub fn insert_complete(&self, bytes: Vec<u8>, now: Instant) -> Result<BlobRef, BlobError> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(BlobError::BlobTooLarge);
        }

        let mut inner = self.lock();
        inner.sweep_expired(now);
        if inner.blobs.len() >= MAX_LIVE_BLOBS {
            return Err(BlobError::TooManyBlobs);
        }
        if inner.staged_bytes().saturating_add(bytes.len()) > MAX_TOTAL_STAGED_BYTES {
            return Err(BlobError::StagingFull);
        }

        let sha256 = hex_digest(&bytes);
        let total_bytes = bytes.len() as u64;
        let id = inner.allocate_id();
        inner.blobs.insert(
            id.clone(),
            Blob {
                expected_bytes: bytes.len(),
                expected_sha256: sha256.clone(),
                data: bytes,
                complete: true,
                last_touched: now,
            },
        );
        Ok(BlobRef {
            blob_id: id,
            total_bytes,
            sha256,
        })
    }

    /// Remove a complete blob and return its bytes.
    ///
    /// Used when the module consumes a staged input — the bytes of a `.pdf`, or
    /// an image for a deck. Taking rather than copying frees the staging budget
    /// at the moment the blob stops being needed.
    ///
    /// # Errors
    ///
    /// [`BlobError::UnknownBlob`] or [`BlobError::IncompleteBlob`].
    pub fn take_complete(&self, blob_id: &str, now: Instant) -> Result<Vec<u8>, BlobError> {
        let mut inner = self.lock();
        inner.sweep_expired(now);
        let blob = inner.blobs.get(blob_id).ok_or(BlobError::UnknownBlob)?;
        if !blob.complete {
            return Err(BlobError::IncompleteBlob);
        }
        Ok(inner
            .blobs
            .remove(blob_id)
            .map(|blob| blob.data)
            .unwrap_or_default())
    }

    /// Drop a blob and free its budget.
    ///
    /// # Errors
    ///
    /// [`BlobError::UnknownBlob`] if there is nothing to release. Releasing is
    /// reported rather than silently accepted so a caller learns that its blob
    /// had already expired.
    pub fn release(&self, blob_id: &str, now: Instant) -> Result<(), BlobError> {
        let mut inner = self.lock();
        inner.sweep_expired(now);
        inner
            .blobs
            .remove(blob_id)
            .map(|_| ())
            .ok_or(BlobError::UnknownBlob)
    }

    /// Number of blobs currently staged, for tests and diagnostics.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.lock().blobs.len()
    }

    /// Take the lock, recovering from a poisoned mutex.
    ///
    /// A panic while holding this lock can only have happened between two
    /// `HashMap` operations, so the map is structurally intact and the worst
    /// case is one blob left in a partial state — which its digest check will
    /// reject. Refusing every subsequent request would turn one caller's panic
    /// into a dead module, and `TinyBus` never unloads a module to recover.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Inner {
    /// Total bytes reserved by every staged blob.
    fn staged_bytes(&self) -> usize {
        self.blobs
            .values()
            .map(Blob::reserved)
            .fold(0usize, usize::saturating_add)
    }

    /// Drop every blob untouched for longer than [`IDLE_TTL`].
    fn sweep_expired(&mut self, now: Instant) {
        self.blobs
            .retain(|_, blob| now.saturating_duration_since(blob.last_touched) <= IDLE_TTL);
    }

    /// Allocate an unused blob id.
    ///
    /// A counter, not a random value: ids are opaque handles inside one process,
    /// never authorisation tokens, and a counter makes a leaked id visible in a
    /// log rather than looking like a secret.
    fn allocate_id(&mut self) -> String {
        self.next_id = self.next_id.wrapping_add(1);
        format!("blob-{}", self.next_id)
    }
}

/// Whether `value` is exactly 64 lowercase hexadecimal characters.
fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Lowercase hex SHA-256 of `bytes`, in the exact shape `BeginBlob` expects.
///
/// Public because declaring a digest is part of using the transfer surface: a
/// caller has to produce this value, and one implementation both sides agree on
/// beats two that can disagree about case or padding.
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

/// Whether `bytes` hashes to `expected`, compared without early exit.
fn verify(bytes: &[u8], expected: &str) -> bool {
    let actual = hex_digest(bytes);
    // Constant-time over the digest strings. The digest is not a secret, so this
    // is defence in depth rather than a requirement — but a hash comparison is
    // exactly the shape that later becomes security-relevant, and the cost of
    // getting it right once is nothing.
    if actual.len() != expected.len() {
        return false;
    }
    actual
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod test;
