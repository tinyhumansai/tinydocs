//! Typed calls and held-output transfer for the `TinyDocs` module.

use std::fmt::{Debug, Formatter};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};
use tinybus::module::{
    ModuleCallError, ModuleCallFailure, ModuleClient, PinnedModuleClient, PinnedModuleObjectClient,
};
use tinydocs_bus::names::methods;
use tinydocs_bus::spec::presentation::MAX_IMAGE_BYTES;
use tinydocs_bus::{
    DocumentFormat, DocumentSpec, ExtractDocumentSpec, ExtractedDocument, ImageFacts, OutputRef,
    RenderPdfSpec, RenderedPdf, WirePresentationSpec,
};

use crate::DocumentCallError;

const MODULE_ID: &str = "tinydocs";
const READ_CHUNK_BYTES: u64 = 1024 * 1024;
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const INVALID_INPUT_WIRE_NAME: &str = "ai.tinyhumans.tinydocs.Error.InvalidInput";

/// Typed client for the `TinyDocs` module.
///
/// The embedding host supplies its process-owned [`ModuleClient`]. Each
/// operation that returns held outputs binds the serving peer for that
/// operation, keeping its producer, output reads, releases, and cleanup on the
/// same peer. Stateless one-call operations continue through `ModuleClient`.
#[derive(Clone)]
pub struct TinyDocsClient {
    module: ModuleClient,
}

impl Debug for TinyDocsClient {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TinyDocsClient")
            .finish_non_exhaustive()
    }
}

impl TinyDocsClient {
    /// Create a `TinyDocs` client over the host's module client.
    #[must_use]
    pub fn new(module: ModuleClient) -> Self {
        Self { module }
    }

    /// Generate and collect a `.docx` document.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError`] when `TinyDocs` is unavailable, rejects the
    /// specification, or fails to generate or transfer the output.
    pub async fn generate_docx(&self, spec: &DocumentSpec) -> Result<Vec<u8>, DocumentCallError> {
        let module = self.bind().await?;
        let output = Self::output_client(&module)?;
        let handle: OutputRef = module
            .call_detailed(methods::GENERATE_DOCX, (spec,))
            .await
            .map_err(classify_failure)?;
        collect(&output, handle).await
    }

    /// Generate and collect a `.pptx` document.
    ///
    /// `images` contains slide-image bytes concatenated in slide order. Each
    /// slide's byte length in `deck` defines its range in that buffer. An empty
    /// image buffer uses the text-only call without opening a stream.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError`] when `TinyDocs` is unavailable, rejects the
    /// deck, or fails to generate or transfer the output.
    pub async fn generate_pptx(
        &self,
        deck: &WirePresentationSpec,
        images: &[u8],
    ) -> Result<Vec<u8>, DocumentCallError> {
        let module = self.bind().await?;
        let output = Self::output_client(&module)?;
        let handle: OutputRef = if images.is_empty() {
            module
                .call_detailed(
                    methods::GENERATE_PPTX,
                    (deck, Option::<tinybus::stream::StreamRef>::None),
                )
                .await
                .map_err(classify_failure)?
        } else {
            module
                .call_with_stream_detailed(
                    methods::GENERATE_PPTX,
                    |stream| serde_json::json!([deck, stream]),
                    images,
                )
                .await
                .map_err(classify_failure)?
        };
        collect(&output, handle).await
    }

    /// Extract the text layer from a streamed PDF.
    ///
    /// Scanned documents with no text layer produce an empty string.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError`] when extraction, output transfer, or UTF-8
    /// decoding fails.
    pub async fn extract_text(&self, document: &[u8]) -> Result<String, DocumentCallError> {
        let module = self.bind().await?;
        let output = Self::output_client(&module)?;
        let handle: OutputRef = module
            .call_with_stream_detailed(
                methods::EXTRACT_TEXT,
                |stream| serde_json::json!([stream]),
                document,
            )
            .await
            .map_err(classify_failure)?;
        let bytes = collect(&output, handle).await?;
        String::from_utf8(bytes).map_err(|_| {
            DocumentCallError::Failed("extracted text was not valid UTF-8".to_string())
        })
    }

    /// Extract bounded text and provenance from a streamed document.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError`] when extraction or the typed reply fails.
    pub async fn extract_document(
        &self,
        document: &[u8],
        spec: &ExtractDocumentSpec,
    ) -> Result<ExtractedDocument, DocumentCallError> {
        self.module
            .call_with_stream_detailed(
                MODULE_ID,
                methods::EXTRACT_DOCUMENT,
                |stream| serde_json::json!([spec, stream]),
                document,
            )
            .await
            .map_err(classify_failure)
    }

    /// Convert a streamed PDF or Office document into normalized Markdown.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError`] when conversion, output transfer, or UTF-8
    /// decoding fails.
    pub async fn convert_markdown(
        &self,
        format: DocumentFormat,
        document: &[u8],
    ) -> Result<String, DocumentCallError> {
        let module = self.bind().await?;
        let output = Self::output_client(&module)?;
        let handle: OutputRef = module
            .call_with_stream_detailed(
                methods::CONVERT_MARKDOWN,
                |stream| serde_json::json!([format, stream]),
                document,
            )
            .await
            .map_err(classify_failure)?;
        let bytes = collect(&output, handle).await?;
        String::from_utf8(bytes).map_err(|_| {
            DocumentCallError::Failed("converted Markdown was not valid UTF-8".to_string())
        })
    }

    /// Inspect a bounded PNG or JPEG image and return its verified facts.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError::InvalidInput`] for empty or oversized image
    /// data, and another [`DocumentCallError`] when `TinyDocs` cannot inspect it.
    pub async fn inspect_image(&self, image: &[u8]) -> Result<ImageFacts, DocumentCallError> {
        if image.is_empty() || image.len() > MAX_IMAGE_BYTES {
            return Err(DocumentCallError::InvalidInput(format!(
                "image must contain 1 to {MAX_IMAGE_BYTES} bytes"
            )));
        }
        self.module
            .call_with_stream_detailed(
                MODULE_ID,
                methods::INSPECT_IMAGE,
                |stream| serde_json::json!([stream]),
                image,
            )
            .await
            .map_err(classify_failure)
    }

    /// Render selected pages of a PDF and collect every returned PNG.
    ///
    /// The result preserves the page order declared by `TinyDocs`.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCallError`] when rendering or any page transfer fails.
    pub async fn render_pdf(
        &self,
        document: &[u8],
        spec: &RenderPdfSpec,
    ) -> Result<Vec<(u32, Vec<u8>)>, DocumentCallError> {
        let module = self.bind().await?;
        let output = Self::output_client(&module)?;
        let rendered: RenderedPdf = module
            .call_with_stream_detailed(
                methods::RENDER_PDF,
                |stream| serde_json::json!([spec, stream]),
                document,
            )
            .await
            .map_err(classify_failure)?;

        let mut pending = OutputReleaseGuard {
            output: output.clone(),
            ids: rendered
                .pages
                .iter()
                .map(|page| page.output.output_id.clone())
                .collect(),
        };
        let mut pages = Vec::new();
        let mut first_error = None;
        for page in rendered.pages {
            let output_id = page.output.output_id.clone();
            // Transfer cleanup responsibility to `collect` before its first
            // await. If this future is cancelled, that guard releases this
            // output and `pending` releases all pages not yet visited.
            pending.ids.retain(|pending_id| pending_id != &output_id);
            match collect(&output, page.output).await {
                Ok(bytes) if first_error.is_none() => pages.push((page.page, bytes)),
                Ok(_) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(pages)
        }
    }

    async fn bind(&self) -> Result<PinnedModuleClient, DocumentCallError> {
        self.module.bind(MODULE_ID).await.map_err(classify_failure)
    }

    fn output_client(
        module: &PinnedModuleClient,
    ) -> Result<PinnedModuleObjectClient, DocumentCallError> {
        module
            .object_client(tinydocs_bus::names::OBJECT_PATH)
            .map_err(classify_sdk_error)
    }
}

/// Releases retained module outputs when the collection future is cancelled.
struct OutputReleaseGuard {
    output: PinnedModuleObjectClient,
    ids: Vec<String>,
}

impl Drop for OutputReleaseGuard {
    fn drop(&mut self) {
        if self.ids.is_empty() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let output = self.output.clone();
        let ids = std::mem::take(&mut self.ids);
        runtime.spawn(async move {
            for output_id in ids {
                match output
                    .call_detailed::<()>(methods::RELEASE_OUTPUT, (output_id,))
                    .await
                {
                    Ok(()) => {}
                    Err(_) => {
                        tracing::debug!(module = MODULE_ID, "releasing a TinyDocs output failed");
                    }
                }
            }
        });
    }
}

async fn collect(
    output: &PinnedModuleObjectClient,
    handle: OutputRef,
) -> Result<Vec<u8>, DocumentCallError> {
    let mut release = OutputReleaseGuard {
        output: output.clone(),
        ids: vec![handle.output_id.clone()],
    };
    let result = read_all(output, &handle).await;
    if output
        .call_detailed::<()>(methods::RELEASE_OUTPUT, (handle.output_id.clone(),))
        .await
        .is_err()
    {
        tracing::debug!(module = MODULE_ID, "releasing a TinyDocs output failed");
    }
    release.ids.clear();
    result
}

async fn read_all(
    output: &PinnedModuleObjectClient,
    handle: &OutputRef,
) -> Result<Vec<u8>, DocumentCallError> {
    if handle.total_bytes > MAX_DOCUMENT_BYTES {
        return Err(DocumentCallError::Failed(format!(
            "the module declared a {}-byte document, over the {MAX_DOCUMENT_BYTES}-byte limit",
            handle.total_bytes
        )));
    }

    let capacity = usize::try_from(handle.total_bytes).unwrap_or(0);
    let max_capacity = usize::try_from(MAX_DOCUMENT_BYTES).unwrap_or(usize::MAX);
    let mut bytes = Vec::with_capacity(capacity.min(max_capacity));
    while (bytes.len() as u64) < handle.total_bytes {
        let encoded: String = output
            .call_detailed(
                methods::READ_OUTPUT,
                (
                    handle.output_id.clone(),
                    bytes.len() as u64,
                    READ_CHUNK_BYTES,
                ),
            )
            .await
            .map_err(classify_failure)?;
        let chunk = BASE64.decode(encoded).map_err(|_| {
            DocumentCallError::Failed("a TinyDocs output chunk was not base64".to_string())
        })?;
        if chunk.is_empty() {
            return Err(DocumentCallError::Failed(
                "the TinyDocs output ended before its declared length".to_string(),
            ));
        }
        if bytes.len().saturating_add(chunk.len()) as u64 > handle.total_bytes {
            return Err(DocumentCallError::Failed(
                "the TinyDocs output exceeded its declared length".to_string(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    if sha256_hex(&bytes) != handle.sha256 {
        return Err(DocumentCallError::Failed(
            "the assembled TinyDocs output did not match its declared digest".to_string(),
        ));
    }
    Ok(bytes)
}

fn classify_failure(failure: ModuleCallFailure) -> DocumentCallError {
    if failure.wire_name == INVALID_INPUT_WIRE_NAME {
        DocumentCallError::InvalidInput(failure.message)
    } else if failure.kind == ModuleCallError::Unavailable
        || failure.kind == ModuleCallError::IncompatibleContract
        || failure.wire_name.contains("ModuleUnavailable")
    {
        DocumentCallError::Unavailable(failure.to_string())
    } else {
        DocumentCallError::Failed(failure.to_string())
    }
}

fn classify_sdk_error(error: ModuleCallError) -> DocumentCallError {
    if matches!(
        error,
        ModuleCallError::Unavailable | ModuleCallError::IncompatibleContract
    ) {
        DocumentCallError::Unavailable(error.to_string())
    } else {
        DocumentCallError::Failed(error.to_string())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
