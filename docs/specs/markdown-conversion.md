# Complete Markdown conversion

Contract version 3 adds `ConvertMarkdown(format, StreamRef) -> OutputRef`.
Existing member argument counts and representations remain unchanged. Hosts
must pin a released version-3 module and verify its artifact digest before
calling this member. Version-2 modules do not provide this operation.

`format` uses the existing `DocumentFormat` wire values: `pdf`, `docx`, `pptx`,
and `xlsx`. Authorized bytes arrive over the existing TinyBus input stream.
The module executes the Markdown converter and retains UTF-8 output in its
bounded output store. The host reads it with `ReadOutput` and releases it with
`ReleaseOutput`; normal expiry also releases abandoned output.

Unlike `ExtractDocument`, which intentionally produces bounded previews,
`ConvertMarkdown` returns complete normalized text for document persistence.
It preserves the OfficeConverter text shape used by TinyMemory: paragraphs,
XML entity text, numeric PowerPoint slide order, worksheet row labels, and
PDF form-feed page boundaries, including blank pages between text pages.
Hosts keep their existing stored documents and ingestion metadata.

Empty and oversized input return `InvalidInput`. Malformed, overexpanding,
and entirely empty or scanned documents return `ExtractionFailed`. The
32-MiB input ceiling, 64-MiB declared Office expansion ceiling, and one-million
cell spreadsheet extent ceiling apply before materializing parser data.
No local fallback is required or provided by the bus contract.

Regression fixtures exercise all four formats and error cases. The compiled
artifact test calls conversion over an actual broker, checks returned bytes,
and verifies output release and structured parse errors.
