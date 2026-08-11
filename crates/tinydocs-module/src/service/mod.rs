//! `TinyBus` service boundary for the document surface.
//!
//! One object, `/ai/tinyhumans/tinydocs/Documents`, exporting five methods:
//!
//! ```text
//! GenerateDocx(DocumentSpec)                        -> OutputRef
//! GeneratePptx(WirePresentationSpec, Option<Stream>) -> OutputRef
//! ExtractText(StreamRef)                            -> OutputRef
//! ReadOutput(output_id, offset, len)                -> base64
//! ReleaseOutput(output_id)                          -> ()
//! ```
//!
//! # Payloads in and payloads out are not symmetric
//!
//! Inbound bytes ride a `TinyBus` stream: the caller opens one alongside the
//! method call, writes while the call is outstanding, and the module reads it.
//! Flow control, the size cap, the idle timeout and the "only the peer that
//! opened it may write" rule are all the bus's, which is why nothing in this
//! crate re-implements them.
//!
//! Replies cannot do that. `Interface::call` gets a member name and a JSON body —
//! no caller identity, no connection — so a served object cannot open a stream
//! back to whoever called it. A produced document is therefore held in
//! [`crate::outputs`] and pulled with `ReadOutput`, because returning it inline
//! would put it through a 16 MiB JSON frame where a `Vec<u8>` costs about 3.5
//! bytes per byte. A reply-stream seam in `TinyBus` would remove that half.
//!
//! # Slide images arrive as one stream
//!
//! A deck can carry several images, and a call has one stream. Rather than stage
//! each image separately, the wire spec gives every image a `byte_len` and the
//! images are concatenated into a single stream in slide order; the module splits
//! them back apart. The lengths are part of the spec, so a truncated or
//! over-long stream is a named rejection rather than a deck with a corrupt
//! picture in it.
//!
//! # This replaces the `Docx` interface rather than extending it
//!
//! The previous interface returned `GenerateDocx(DocumentSpec) -> Vec<u8>`
//! inline. `TinyBus` is explicit that an existing interface must not change in
//! place, and returning a handle where callers expect bytes is exactly that.
//!
//! The old interface is retired rather than served beside the new one because
//! `module_export!` attaches its `methods` list to the *first* entry in
//! `provides` and leaves any others empty, so a second fully-declared interface
//! is not expressible today.

use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use tinybus::stream::StreamRef;
use tinybus::{Connection, Error as BusError, Result as BusResult};
use tinydocs::spec::presentation::MAX_IMAGE_BYTES;
use tinydocs::spec::{DocumentSpec, PresentationSpec, SlideImage, SlideSpec};
use tinydocs::{Error, pdf, pptx};

use crate::outputs::{OutputError, OutputRef, OutputStore};

pub use tinydocs::spec::{WirePresentationSpec, WireSlideImage, WireSlideSpec};

/// Well-known name and interface exported by the `TinyDocs` module.
pub const BUS_NAME: &str = "ai.tinyhumans.tinydocs.Documents";

/// Object path exported by the `TinyDocs` module.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinydocs/Documents";

const INVALID_INPUT_ERROR: &str = "ai.tinyhumans.tinydocs.Error.InvalidInput";
const GENERATION_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.GenerationFailed";
const EXTRACTION_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.ExtractionFailed";
const MODULE_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.ModuleFailed";
const TRANSFER_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.TransferFailed";
const OUTPUT_REFUSED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.OutputRefused";
const UNKNOWN_OUTPUT_ERROR: &str = "ai.tinyhumans.tinydocs.Error.UnknownOutput";

/// The served object.
///
/// Holds the connection, because reading an inbound stream needs one, and the
/// produced-document store. No document state survives a call.
struct Documents {
    connection: Connection,
    outputs: Arc<OutputStore>,
}

// The interface macro rejects a non-async method outright, so the two output
// methods below are async because the dispatch contract says so, not because
// they await anything. `unused_async` can never be actionable in this block.
#[allow(
    clippy::unused_async,
    reason = "tinybus::interface requires every method to be `async fn`"
)]
#[tinybus::interface(name = "ai.tinyhumans.tinydocs.Documents")]
impl Documents {
    /// Generate a `.docx` and hold it for reading.
    async fn generate_docx(&self, spec: DocumentSpec) -> BusResult<OutputRef> {
        // Validated on this thread, before a blocking slot is taken: rejecting a
        // malformed spec should not queue behind real work.
        spec.validate().map_err(|error| map_error(&error))?;
        let bytes = blocking(move || tinydocs::docx::generate(&spec)).await?;
        self.hold(bytes)
    }

    /// Generate a `.pptx`, reading its images from one concatenated stream.
    async fn generate_pptx(
        &self,
        spec: WirePresentationSpec,
        images: Option<StreamRef>,
    ) -> BusResult<OutputRef> {
        let resolved = self.resolve_presentation(spec, images).await?;
        resolved.validate().map_err(|error| map_error(&error))?;
        let bytes = blocking(move || pptx::generate(&resolved)).await?;
        self.hold(bytes)
    }

    /// Extract the text layer of a streamed `.pdf` and hold the result.
    async fn extract_text(&self, document: StreamRef) -> BusResult<OutputRef> {
        let bytes = self.read_stream(&document).await?;
        let text = blocking(move || pdf::extract_text(&bytes)).await?;
        self.hold(text.into_bytes())
    }

    /// Read up to `len` bytes of a held document at `offset`, base64-encoded.
    async fn read_output(&self, output_id: String, offset: u64, len: u64) -> BusResult<String> {
        let bytes = self
            .outputs
            .read_chunk(&output_id, offset, len, Instant::now())
            .map_err(|error| map_output_error(&error))?;
        Ok(BASE64.encode(bytes))
    }

    /// Drop a held document and free its budget.
    async fn release_output(&self, output_id: String) -> BusResult<()> {
        self.outputs
            .release(&output_id, Instant::now())
            .map_err(|error| map_output_error(&error))
    }
}

impl Documents {
    /// Hold a produced document and return its handle.
    fn hold(&self, bytes: Vec<u8>) -> BusResult<OutputRef> {
        self.outputs
            .insert(bytes, Instant::now())
            .map_err(|error| map_output_error(&error))
    }

    /// Read a whole inbound stream into memory.
    ///
    /// The bus enforces the size cap, the flow-control window and the idle
    /// timeout; a failure here is a transfer that did not complete.
    async fn read_stream(&self, stream: &StreamRef) -> BusResult<Vec<u8>> {
        self.connection
            .read_stream(stream)
            .await
            .map_err(|error| BusError::MethodFailed {
                name: TRANSFER_FAILED_ERROR.to_string(),
                // The bus's own message, which never carries payload bytes.
                message: error.to_string(),
            })
    }

    /// Turn a wire deck plus one concatenated image stream into a real spec.
    ///
    /// The spec's `byte_len` values are the authority on where each image ends.
    /// A stream that does not add up to their sum is refused rather than sliced
    /// into whatever happens to be there: the alternative is a deck containing a
    /// picture assembled from two different images.
    async fn resolve_presentation(
        &self,
        spec: WirePresentationSpec,
        images: Option<StreamRef>,
    ) -> BusResult<PresentationSpec> {
        // Every length is caller-controlled, so the arithmetic is checked and
        // each one is bounded before it is summed. `u64::MAX + 1` wraps to zero
        // in a release build, which would let a zero-byte stream satisfy the
        // aggregate check and then panic on the first slice.
        let mut expected: u64 = 0;
        for image in spec.slides.iter().flat_map(|slide| slide.images.iter()) {
            if image.byte_len > MAX_IMAGE_BYTES as u64 {
                return Err(BusError::MethodFailed {
                    name: INVALID_INPUT_ERROR.to_string(),
                    message: format!(
                        "an image declares {} bytes, over the {MAX_IMAGE_BYTES}-byte limit",
                        image.byte_len
                    ),
                });
            }
            expected =
                expected
                    .checked_add(image.byte_len)
                    .ok_or_else(|| BusError::MethodFailed {
                        name: INVALID_INPUT_ERROR.to_string(),
                        message: "declared image lengths overflow".to_string(),
                    })?;
        }

        let payload = match (&images, expected) {
            (Some(stream), _) => self.read_stream(stream).await?,
            // No stream is only coherent with no images.
            (None, 0) => Vec::new(),
            (None, _) => {
                return Err(BusError::MethodFailed {
                    name: INVALID_INPUT_ERROR.to_string(),
                    message: "the deck declares images but no image stream was opened".to_string(),
                });
            }
        };
        if payload.len() as u64 != expected {
            return Err(BusError::MethodFailed {
                name: INVALID_INPUT_ERROR.to_string(),
                message: format!(
                    "image stream carried {} bytes but the deck declares {expected}",
                    payload.len()
                ),
            });
        }

        let mut cursor = 0usize;
        let mut slides = Vec::with_capacity(spec.slides.len());
        for slide in spec.slides {
            let mut resolved = Vec::with_capacity(slide.images.len());
            for image in slide.images {
                // Bounded above, so this cannot truncate; `checked_add` and a
                // fallible slice keep the walk honest anyway rather than
                // trusting the loop that produced `expected`.
                let len = usize::try_from(image.byte_len).map_err(|_| BusError::MethodFailed {
                    name: INVALID_INPUT_ERROR.to_string(),
                    message: "image length is out of range".to_string(),
                })?;
                let end = cursor
                    .checked_add(len)
                    .ok_or_else(|| BusError::MethodFailed {
                        name: INVALID_INPUT_ERROR.to_string(),
                        message: "image offsets overflow".to_string(),
                    })?;
                let bytes = payload
                    .get(cursor..end)
                    .ok_or_else(|| BusError::MethodFailed {
                        name: INVALID_INPUT_ERROR.to_string(),
                        message: "declared image lengths do not fit the image stream".to_string(),
                    })?;
                resolved.push(
                    SlideImage::from_bytes(bytes.to_vec(), image.caption)
                        .map_err(|error| map_error(&error))?,
                );
                cursor = end;
            }
            slides.push(SlideSpec {
                title: slide.title,
                body: slide.body,
                bullets: slide.bullets,
                speaker_notes: slide.speaker_notes,
                images: resolved,
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

/// Map an output-store failure onto its wire name.
///
/// Grouped by what the caller should do next: `UnknownOutput` means the document
/// is gone and the call has to be made again, `OutputRefused` means the store is
/// full and the same request may succeed later, `TransferFailed` means the read
/// itself was malformed.
fn map_output_error(error: &OutputError) -> BusError {
    let name = match *error {
        OutputError::UnknownOutput => UNKNOWN_OUTPUT_ERROR,
        OutputError::StoreFull | OutputError::TooManyOutputs | OutputError::OutputTooLarge => {
            OUTPUT_REFUSED_ERROR
        }
        OutputError::ChunkTooLarge | OutputError::ReadPastEnd => TRANSFER_FAILED_ERROR,
    };
    BusError::MethodFailed {
        name: name.to_string(),
        message: error.to_string(),
    }
}

async fn setup(connection: Connection) -> BusResult<()> {
    let documents = Documents {
        connection: connection.clone(),
        outputs: Arc::new(OutputStore::new()),
    };
    connection
        .serve_at(OBJECT_PATH.try_into()?, documents)
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
            "GenerateDocx",
            "GeneratePptx",
            "ExtractText",
            "ReadOutput",
            "ReleaseOutput",
        ],
        signals = [],
        requires = [],
        optional = [],
        lazy = false,
    }
}

#[cfg(test)]
mod test;
