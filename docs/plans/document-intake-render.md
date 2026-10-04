# Document intake and selected PDF rendering implementation

Status: Implemented.
Specification: [Document intake and selected PDF rendering](../specs/document-intake-render.md).

This records the implementation sequence for the accepted document-intake work.
Hosts own upload storage, OCR/inference policy, cancellation, and stronger parser
isolation. TinyDocs owns supplied-byte parsing and its module contract.

1. Define validated extraction/render specs, provenance results, and shared
   `OutputRef` in `crates/tinydocs-bus/src/intake/`; declare additive member names
   in `src/names.rs`. Add serialization and rejection tests before wiring I/O.
2. Add optional `intake` and `pdf-render` features in `Cargo.toml` and deliberate
   exports in `src/lib.rs`. Preserve existing writer/extraction features.
3. Implement bounded text sinks, selected OOXML member reads, shared-string
   resolution, and relationship ordering in `src/intake/mod.rs` and
   `office_order.rs`. Add DOCX/PPTX/XLSX fixtures and malformed/budget tests in
   their sibling test files. Test adversarial empty shared strings and output
   capacity retention before adding allocation checks.
4. Preflight ZIP metadata in `src/intake/zip_admission.rs` before eager reader
   allocation. Verify oversized counts, names, ZIP64 metadata, embedded footer
   confusion, and unusual valid preambles with synthetic archives.
5. Extract PDF sections with bounded text sinks and scanned candidates. Build
   deterministic text/raster fixtures in `src/pdf/fixtures.rs`; test mixed pages
   and truncation without external files or network services.
6. Raster selected pages in `src/pdf_render/mod.rs`; validate page, edge, pixel,
   and PNG output budgets. Test actual raster pixels rather than only headers.
7. Add streamed service handlers and output-store rollback in
   `crates/tinydocs-module/src/service/` and `outputs/`. Extend
   `tests/module_e2e.rs` to exercise the compiled module through TinyBus and
   read/release generated PNGs.
8. Document APIs and parser limitations in module READMEs, the root README,
   and the linked specification. Keep dependency advisories enabled; use patched
   `quick-xml` and adapt attribute decoding through the reader's decoder.

## Verification

Run from the TinyDocs repository root:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.88.0 check --workspace --all-features
cargo deny --all-features check all
```

The module E2E workflow builds and loads the cdylib before executing the bus
fixture suite. Each completed step has corresponding sibling regression tests;
publishing module artifacts and updating downstream release pins remain outside
this plan.
