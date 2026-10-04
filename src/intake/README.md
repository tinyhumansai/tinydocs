# Document intake

`extract(bytes, &ExtractDocumentSpec)` accepts PDF, DOCX, PPTX and XLSX bytes.
Each section retains its source page or OOXML part path. Empty PDF text layers
are retained and marked `scanned_candidate`; that flag is a suggestion for host
vision, not a determination that the page contains an image or needs OCR.

The library accepts at most 64 MiB of input, 2,048 ZIP members, 32 MiB of declared
and read ZIP expansion, 8 MiB per member, 256-byte member names, and 128 nested
XML elements. A bounded metadata admission pass checks member counts, decoded
names and at most 1 MiB each of central-directory and ZIP64 footer metadata
before constructing the eager ZIP index. Raw duplicate names are rejected in
a set capped at 2,048 borrowed names, before the ZIP library can deduplicate
them. Alternative footer signatures in
metadata are rejected conservatively; signatures in member payloads are hidden
only while indexing, so the parser cannot fall back to an unadmitted archive.
DTDs, unresolved entities, encrypted ZIP/PDF documents, duplicate member names
and malformed structures fail explicitly. ZIP contents are never
written to disk. Only document body/slide/worksheet parts are returned; archive
listing is outside this module.

The result has at most 256 sections and 200,000 UTF-8 text bytes, with explicit
`truncated` and total `section_count`. Source labels are separately bounded.
Caller budgets may narrow these ceilings. Unicode is never cut in the middle
of a code point. Office text is bounded while XML is parsed, including every
resolved shared-string append; repeating a large XLSX string cannot amplify the
output allocation beyond the remaining caller budget. Parsing continues after
truncation to validate the selected XML.

PPTX presentation and XLSX workbook manifests determine section order through
their relationships. Unreferenced slide/sheet parts are excluded. Missing,
external, duplicate or unsupported references fail explicitly, as do missing
manifests; there is no filename-order fallback. Relationship paths are resolved
inside the package and retain the actual source part path. XLSX resolves shared
and inline strings and uses cached cell values rather than evaluating formulas;
paths identify sheets without inventing workbook display names.

PDF text is extracted page by page with a bounded output writer. At most 4,096
source pages are accepted. The PDF parser may allocate for objects, fonts and
expanded streams internally: the input/page/output limits are **not a total
parser memory or CPU bound**. Calls are synchronous. A native TinyBus module
runs in the host process; a Tokio timeout does not stop a blocking parser.
Hosts own deadline/cancellation policy and any stronger isolation they need.

No network OCR, office application, shell process or filesystem read is used.
