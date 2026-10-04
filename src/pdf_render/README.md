# PDF rasterization

The optional `pdf-render` feature uses Hayro 0.5 to render selected source pages
to PNG. `render(bytes, &RenderPdfSpec)` returns `PdfImages`, preserving request
order, one-based page numbers and dimensions. It never eagerly rasterizes all
pages. The TinyBus `RenderPdf` method retains each PNG through the existing
`OutputRef` / `ReadOutput` / `ReleaseOutput` lifecycle. If an output-store
insertion fails partway through a batch, all new handles in that batch are
released; previously held caller outputs remain intact.

Hard ceilings are 64 MiB input, 4,096 source pages, eight unique selected pages,
2,048 pixels on the longest edge, 16 million aggregate pixels, and 32 MiB
aggregate PNG bytes. Caller budgets can only narrow these limits. All selected
page dimensions and the aggregate pixel budget are checked before rendering.
Encrypted/malformed PDFs, missing pages and exceeded budgets return an error;
no partial successful result is returned. PNG byte limits apply to encoded
outputs; one bounded raster must be encoded before its size is known.

Hayro runs synchronously in the native module's process. Its parser, image
codecs and display-list allocations do not expose a global memory budget.
Raster/input/output bounds therefore do not guarantee bounded internal parser
or renderer resources. A blocking-pool timeout is not cancellation. The host
owns execution deadlines and any stronger process isolation. No network OCR
or remote inference happens here.

Hayro 0.5 and the resolved extraction/render dependency graph compile and test
on the declared Rust 1.88 MSRV.
