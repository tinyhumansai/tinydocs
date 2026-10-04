# Document intake and selected PDF rendering

Status: Implemented. Owner: TinyDocs.

## Problem

Agent hosts need document text with page, slide, and worksheet provenance and
selected PDF page images when text extraction finds scanned pages. Intake must
operate on supplied bytes without extracting files or contacting a network.

## Behavior

`intake::extract(bytes, &ExtractDocumentSpec)` accepts PDF, DOCX, PPTX, and XLSX.
The result reports total section count, ordered source labels, bounded text,
truncation, and PDF scanned-page candidates. Blank PDF text is a candidate for
host-side analysis, not a determination that OCR will succeed. Office ordering
follows presentation/workbook relationships. XLSX supports shared strings,
inline strings, and cached values; it does not calculate formulas.

`pdf_render::render(bytes, &RenderPdfSpec)` renders explicitly selected 1-based
PDF pages to PNG with width and height metadata. It does not perform OCR or
choose pages. Original bytes remain the host's responsibility.

The TinyBus `ExtractDocument` member accepts the extraction spec and input stream
and returns bounded inline section text. `RenderPdf` returns page metadata and
held `OutputRef` values, read and released through existing output-store members.
Invalid arguments, malformed or encrypted documents, unsupported structures,
and exceeded hard budgets return typed domain errors. Earlier module members
remain compatible; the additive contract remains version 2.

## Constraints

- Inputs are nonempty and at most 64 MiB.
- Extraction allows at most 200,000 text bytes and 256 returned sections.
- OOXML allows 2,048 members, 256-byte member names, 8 MiB per XML part,
  32 MiB total expansion, and XML nesting depth 128. ZIP metadata admission
  runs before constructing the eager ZIP reader. DTDs are rejected.
- Shared strings allow at most 65,536 records and 4 MiB decoded text. Record
  count and text checks precede retained allocation. Returned short sections
  release unused reserved text capacity.
- PDFs allow at most 4,096 pages; rendering selects at most 8 pages, bounds the
  maximum edge to 2,048 pixels, total pixels to 16 million, and encoded output
  to 32 MiB.
- The native module runs in-process. These input and output budgets do not
  bound all PDF parser internals. A host requiring parser isolation must use
  a separate process; a task timeout alone cannot terminate blocking parsing.
- Library features `intake` and `pdf-render` are opt-in. The module enables
  them. No archive extraction, networking, OCR, or provider inference occurs.

## Acceptance criteria

Fixture tests verify ordered Office sections, shared/inline/cached worksheet
values, mixed text/scanned PDFs, raster pixels and PNG dimensions, truncation,
malformed XML, hostile ZIP metadata, and all documented budget rejections.
Bus tests verify schema validation, streamed input, held output reads/releases,
and rollback when a rendering output cannot be retained. MSRV is Rust 1.88.

## Open questions

None blocking the library contract. Publishing the updated module is a separate
release operation; hosts must not call new members on an older pinned artifact.
