//! `TinyDocs` bus identity and member names.

/// Well-known bus name exported by the `TinyDocs` module.
pub const BUS_NAME: &str = "ai.tinyhumans.tinydocs.Documents";

/// Object path served by the `TinyDocs` module.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinydocs/Documents";

/// One constant per method name on [`BUS_NAME`].
pub mod methods {
    /// `GenerateDocx` — generate a complete DOCX payload.
    pub const GENERATE_DOCX: &str = "GenerateDocx";
    /// `GeneratePptx` — generate a complete PPTX payload.
    pub const GENERATE_PPTX: &str = "GeneratePptx";
    /// `ExtractText` — extract text from a streamed PDF.
    pub const EXTRACT_TEXT: &str = "ExtractText";
    /// `ExtractDocument` — bounded document text and provenance from a stream.
    pub const EXTRACT_DOCUMENT: &str = "ExtractDocument";
    /// `ConvertMarkdown` — convert a streamed Office/PDF document to full Markdown.
    pub const CONVERT_MARKDOWN: &str = "ConvertMarkdown";
    /// `RenderPdf` — selected PDF pages as held PNG outputs.
    pub const RENDER_PDF: &str = "RenderPdf";
    /// `ReadOutput` — read a bounded base64-encoded output chunk.
    pub const READ_OUTPUT: &str = "ReadOutput";
    /// `ReleaseOutput` — release a held output.
    pub const RELEASE_OUTPUT: &str = "ReleaseOutput";
    /// `InspectImage` — interpret one bounded PNG/JPEG stream.
    pub const INSPECT_IMAGE: &str = "InspectImage";
}

/// All method names in the declaration order used by the module interface.
pub const METHODS: [&str; 9] = [
    methods::GENERATE_DOCX,
    methods::GENERATE_PPTX,
    methods::EXTRACT_TEXT,
    methods::EXTRACT_DOCUMENT,
    methods::CONVERT_MARKDOWN,
    methods::RENDER_PDF,
    methods::READ_OUTPUT,
    methods::RELEASE_OUTPUT,
    methods::INSPECT_IMAGE,
];
