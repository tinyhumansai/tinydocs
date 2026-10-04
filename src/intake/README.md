# Document intake

`extract(bytes, &ExtractDocumentSpec)` accepts PDF, DOCX, PPTX and XLSX bytes.
Each section retains its source page or OOXML part path. Empty PDF text layers
are retained and marked `scanned_candidate`; that flag is a suggestion for host
vision, not a determination that the page contains an image or needs OCR.

The library accepts at most 64 MiB of input, 2,048 ZIP members, 32 MiB of declared
and read ZIP expansion, 8 MiB per member, 256-byte member names, and 128 nested
XML elements. DTDs, unresolved entities, encrypted ZIP/PDF documents, duplicate
selected parts and malformed structures fail explicitly. ZIP contents are never
written to disk. Only document body/slide/worksheet parts are returned; archive
listing is outside this module.

The result has at most 256 sections and 200,000 UTF-8 text bytes, with explicit
`truncated` and total `section_count`. Source labels are separately bounded.
Caller budgets may narrow these ceilings. Unicode is never cut in the middle
of a code point. Office sections are ordered by numbered part path. XLSX resolves
shared and inline strings and uses cached cell values rather than evaluating
formulas; paths identify sheets without inventing workbook display names.

PDF text is extracted page by page with a bounded output writer. At most 4,096
source pages are accepted. The PDF parser may allocate for objects, fonts and
expanded streams internally: the input/page/output limits are **not a total
parser memory or CPU bound**. Calls are synchronous. A native TinyBus module
runs in the host process; a Tokio timeout does not stop a blocking parser.
Hosts own deadline/cancellation policy and any stronger isolation they need.

No network OCR, office application, shell process or filesystem read is used.
