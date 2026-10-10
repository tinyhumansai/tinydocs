# Markdown conversion

`tinydocs::markdown::convert(bytes, DocumentFormat)` extracts PDF, DOCX, PPTX,
and XLSX to the complete normalized Markdown used by memory ingestion. It
retains TinyMemory OfficeConverter semantics, including form-feed PDF page
boundaries, numeric slide order, and `sheet | cell | cell` spreadsheet rows.
The implementation is adapted from TinyMemory under GPL-3.0-only.

Input is limited to 32 MiB. Office archives are admitted only when their
aggregate declared expansion is at most 64 MiB; spreadsheet ranges are checked
with a sparse scan before allocating a dense grid of at most one million
cells. An entirely empty or scanned document returns `ExtractionFailed`.

The TinyBus `ConvertMarkdown(format, StreamRef)` member executes this converter
inside the compiled module and returns an `OutputRef`. Download with
`ReadOutput(output_id, offset, length)` and release with
`ReleaseOutput(output_id)`. The existing output store limits and expiry apply.
Callers retain authorization, provenance, ingestion metadata, and cancellation.
The pure bus contract contains no parser dependency.
