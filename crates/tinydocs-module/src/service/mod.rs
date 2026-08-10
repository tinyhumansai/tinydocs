//! `TinyBus` service boundary for the document surface.
//!
//! One object, `/ai/tinyhumans/tinydocs/Documents`, exporting the three format
//! operations plus the four chunked-transfer operations they depend on:
//!
//! ```text
//! BeginBlob(total_bytes, sha256)      -> blob_id
//! PutChunk(blob_id, offset, base64)   -> bytes received so far
//! GetChunk(blob_id, offset, len)      -> base64
//! ReleaseBlob(blob_id)                -> ()
//! GenerateDocx(DocumentSpec)          -> BlobRef
//! GeneratePptx(WirePresentationSpec)  -> BlobRef
//! ExtractText(blob_id)                -> BlobRef
//! ```
//!
//! # Why everything returns a `BlobRef`
//!
//! See [`crate::blobs`]. A `TinyBus` frame is a 16 MiB JSON document and
//! `Vec<u8>` serialises as an array of integers, so the real inline ceiling is a
//! few megabytes — below a deck's legal image payload and below any `.pdf` worth
//! extracting. Rather than have some methods return bytes inline and others not,
//! every unbounded result is staged and read back in chunks. The caller's code
//! path is then the same regardless of size.
//!
//! # This replaces the `Docx` interface rather than extending it
//!
//! The previous interface, `ai.tinyhumans.tinydocs.Docx`, returned
//! `GenerateDocx(DocumentSpec) -> Vec<u8>` inline. `TinyBus`'s module guidance is
//! explicit that an existing interface must not change in place — a breaking
//! contract gets a new interface name — and returning a `BlobRef` where callers
//! expect bytes is exactly that. Hence a new name.
//!
//! The old interface is retired rather than served alongside, because
//! `module_export!` attaches its `methods` list to the *first* entry in
//! `provides` and leaves any others with an empty method list. A second
//! fully-declared interface is therefore not expressible today, and a manifest
//! that under-declares its members would break the invariant that manifest
//! methods and dispatch members stay identical. Serving both needs a `TinyBus`
//! change first; retiring one at a pre-1.0 minor bump does not.
//!
//! # Runtime
//!
//! Synthesis and extraction are CPU-bound and run on the module runtime's
//! blocking pool. The blob operations are memory copies under a short lock and
//! run inline. The module holds no document state between calls — only staged
//! blobs, every one of them bounded and expiring.

mod wire;

use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use tinybus::{Connection, Error as BusError, Result as BusResult};
use tinydocs::spec::{DocumentSpec, PresentationSpec, SlideImage, SlideSpec};
use tinydocs::{Error, pdf, pptx};

use crate::blobs::{BlobError, BlobRef, BlobStore};

pub use wire::{WirePresentationSpec, WireSlideImage, WireSlideSpec};

/// Well-known name and interface exported by the `TinyDocs` module.
pub const BUS_NAME: &str = "ai.tinyhumans.tinydocs.Documents";

/// Object path exported by the `TinyDocs` module.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinydocs/Documents";

const INVALID_INPUT_ERROR: &str = "ai.tinyhumans.tinydocs.Error.InvalidInput";
const GENERATION_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.GenerationFailed";
const EXTRACTION_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.ExtractionFailed";
const MODULE_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.ModuleFailed";
const TRANSFER_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.TransferFailed";
const TRANSFER_REFUSED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.TransferRefused";
const UNKNOWN_BLOB_ERROR: &str = "ai.tinyhumans.tinydocs.Error.UnknownBlob";

/// The served object. Owns the staging area; holds no document state.
struct Documents {
    blobs: Arc<BlobStore>,
}

// The interface macro rejects a non-async method outright, so the four transfer
// methods below are async because the dispatch contract says so, not because they
// await anything. `unused_async` can therefore never be actionable in this block.
#[allow(
    clippy::unused_async,
    reason = "tinybus::interface requires every method to be `async fn`"
)]
#[tinybus::interface(name = "ai.tinyhumans.tinydocs.Documents")]
impl Documents {
    /// Reserve space for a blob of `total_bytes` that will hash to `sha256`.
    async fn begin_blob(&self, total_bytes: u64, sha256: String) -> BusResult<String> {
        self.blobs
            .begin(total_bytes, &sha256, Instant::now())
            .map_err(|error| map_blob_error(&error))
    }

    /// Append a base64 chunk at `offset`, returning bytes received so far.
    async fn put_chunk(&self, blob_id: String, offset: u64, data: String) -> BusResult<u64> {
        let decoded = decode_base64(&data)?;
        self.blobs
            .put_chunk(&blob_id, offset, &decoded, Instant::now())
            .map_err(|error| map_blob_error(&error))
    }

    /// Read up to `len` bytes of a complete blob at `offset`, base64-encoded.
    async fn get_chunk(&self, blob_id: String, offset: u64, len: u64) -> BusResult<String> {
        let bytes = self
            .blobs
            .get_chunk(&blob_id, offset, len, Instant::now())
            .map_err(|error| map_blob_error(&error))?;
        Ok(BASE64.encode(bytes))
    }

    /// Drop a blob and free its budget.
    async fn release_blob(&self, blob_id: String) -> BusResult<()> {
        self.blobs
            .release(&blob_id, Instant::now())
            .map_err(|error| map_blob_error(&error))
    }

    /// Generate a `.docx` and stage it for reading.
    async fn generate_docx(&self, spec: DocumentSpec) -> BusResult<BlobRef> {
        // Validated on this thread, before a blocking slot is taken: rejecting a
        // malformed spec should not have to queue behind real work.
        spec.validate().map_err(|error| map_error(&error))?;
        let bytes = blocking(move || tinydocs::docx::generate(&spec)).await?;
        self.stage(bytes)
    }

    /// Generate a `.pptx` from a spec whose images name staged blobs.
    async fn generate_pptx(&self, spec: WirePresentationSpec) -> BusResult<BlobRef> {
        let resolved = self.resolve_presentation(spec)?;
        resolved.validate().map_err(|error| map_error(&error))?;
        let bytes = blocking(move || pptx::generate(&resolved)).await?;
        self.stage(bytes)
    }

    /// Extract the text layer of a staged `.pdf` and stage the result.
    async fn extract_text(&self, blob_id: String) -> BusResult<BlobRef> {
        // Taken rather than copied: the document is often the largest thing in
        // the staging area, and holding it through extraction as well would
        // double its cost for no reason.
        let bytes = self
            .blobs
            .take_complete(&blob_id, Instant::now())
            .map_err(|error| map_blob_error(&error))?;
        let text = blocking(move || pdf::extract_text(&bytes)).await?;
        self.stage(text.into_bytes())
    }
}

impl Documents {
    /// Stage a produced payload and return its handle.
    fn stage(&self, bytes: Vec<u8>) -> BusResult<BlobRef> {
        self.blobs
            .insert_complete(bytes, Instant::now())
            .map_err(|error| map_blob_error(&error))
    }

    /// Turn a wire deck into a real [`PresentationSpec`] by consuming the blobs
    /// its images name.
    ///
    /// Images are taken from the staging area, so a deck's bytes stop being
    /// charged twice the moment they are resolved. A blob that is missing or
    /// incomplete fails the whole call rather than silently dropping a slide's
    /// image — the caller staged it, so its absence is a transfer bug worth
    /// reporting, not a degraded deck.
    fn resolve_presentation(&self, spec: WirePresentationSpec) -> BusResult<PresentationSpec> {
        let now = Instant::now();
        let mut slides = Vec::with_capacity(spec.slides.len());
        for slide in spec.slides {
            let mut images = Vec::with_capacity(slide.images.len());
            for image in slide.images {
                let bytes = self
                    .blobs
                    .take_complete(&image.blob_id, now)
                    .map_err(|error| map_blob_error(&error))?;
                images.push(
                    SlideImage::from_bytes(bytes, image.caption)
                        .map_err(|error| map_error(&error))?,
                );
            }
            slides.push(SlideSpec {
                title: slide.title,
                body: slide.body,
                bullets: slide.bullets,
                speaker_notes: slide.speaker_notes,
                images,
            });
        }
        Ok(PresentationSpec {
            title: spec.title,
            author: spec.author,
            theme: spec.theme,
            slides,
        })
    }
}

/// Run a CPU-bound library call on the blocking pool and map its failure.
async fn blocking<T, F>(work: F) -> BusResult<T>
where
    F: FnOnce() -> Result<T, Error> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| BusError::MethodFailed {
            name: MODULE_FAILED_ERROR.to_string(),
            message: "document worker failed".to_string(),
        })?
        .map_err(|error| map_error(&error))
}

/// Decode a base64 chunk, refusing malformed input by name.
fn decode_base64(data: &str) -> BusResult<Vec<u8>> {
    BASE64.decode(data).map_err(|_| BusError::MethodFailed {
        name: INVALID_INPUT_ERROR.to_string(),
        // The payload itself is never echoed: it is caller data, and an error
        // message is the wrong place for it.
        message: "chunk data is not valid base64".to_string(),
    })
}

/// Map a library error onto its wire name.
fn map_error(error: &Error) -> BusError {
    let name = match error {
        Error::InvalidInput { .. } => INVALID_INPUT_ERROR,
        Error::GenerationFailed { .. } => GENERATION_FAILED_ERROR,
        Error::ExtractionFailed { .. } => EXTRACTION_FAILED_ERROR,
        _ => MODULE_FAILED_ERROR,
    };
    BusError::MethodFailed {
        name: name.to_string(),
        message: error.to_string(),
    }
}

/// Map a staging failure onto its wire name.
///
/// Three names rather than one, because the caller's correct response differs.
/// `UnknownBlob` means the transfer is gone and has to restart; `TransferRefused`
/// means a budget is full and retrying later may work; `TransferFailed` means the
/// caller sent something wrong and should re-send.
fn map_blob_error(error: &BlobError) -> BusError {
    let name = match *error {
        BlobError::UnknownBlob => UNKNOWN_BLOB_ERROR,
        BlobError::StagingFull | BlobError::TooManyBlobs => TRANSFER_REFUSED_ERROR,
        BlobError::MalformedDigest
        | BlobError::BlobTooLarge
        | BlobError::ChunkTooLarge
        | BlobError::OutOfOrderChunk { .. }
        | BlobError::OverlongBlob
        | BlobError::DigestMismatch
        | BlobError::IncompleteBlob
        | BlobError::ReadPastEnd => TRANSFER_FAILED_ERROR,
    };
    BusError::MethodFailed {
        name: name.to_string(),
        message: error.to_string(),
    }
}

async fn setup(connection: Connection) -> BusResult<()> {
    connection
        .serve_at(
            OBJECT_PATH.try_into()?,
            Documents {
                blobs: Arc::new(BlobStore::new()),
            },
        )
        .await?;
    connection.request_name(BUS_NAME).await?;
    Ok(())
}

// Isolate the three generated public C symbols so the lint exception cannot
// hide undocumented Rust API. Their contract is TinyBus ABI v1, and none is a
// Rust-callable export from this private module.
#[allow(
    missing_docs,
    unreachable_pub,
    reason = "generated C ABI symbols are documented by the TinyBus module SDK"
)]
mod exports {
    tinybus_module::module_export! {
        setup = super::setup,
        worker_threads = 2,
        provides = ["ai.tinyhumans.tinydocs.Documents"],
        methods = [
            "BeginBlob",
            "PutChunk",
            "GetChunk",
            "ReleaseBlob",
            "GenerateDocx",
            "GeneratePptx",
            "ExtractText",
        ],
        signals = [],
        requires = [],
        optional = [],
        lazy = false,
    }
}

#[cfg(test)]
mod test;
