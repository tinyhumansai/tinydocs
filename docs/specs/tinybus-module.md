# TinyBus module

Status: Implemented

Owner: TinyDocs maintainers

## Problem

TinyDocs must be installable as a compiled TinyBus module so a host can use
document generation without linking the document stack into its own binary.
The released artifact must exercise the same ABI and loading path used in
production.

## Goals

- Preserve the existing pure Rust library API.
- Build a target-specific dynamic library implementing TinyBus module ABI v1.
- Expose typed DOCX generation through a stable bus identity.
- Publish installable native bundles with each GitHub release.
- Test loading and calling the compiled artifact through a real broker.

## Non-goals

- Loading untrusted third-party modules safely.
- Stable ABI compatibility across TinyBus ABI revisions.
- Streaming or file-descriptor transfer in this first interface.
- Running TinyDocs as a separate socket process.

## Behavior

The private `tinydocs-module` workspace crate depends on the public library's
`docx`, `pptx` and `pdf` features and builds as a `cdylib`. This separation keeps
unpublished, vendored TinyBus packages out of the crates.io package manifest. The
module claims `ai.tinyhumans.tinydocs.Documents`, serves the object path
`/ai/tinyhumans/tinydocs/Documents`, and exports seven methods:

```text
BeginBlob(total_bytes, sha256)       -> blob_id
PutChunk(blob_id, offset, base64)    -> bytes received so far
GetChunk(blob_id, offset, len)       -> base64
ReleaseBlob(blob_id)                 -> ()
GenerateDocx(DocumentSpec)           -> BlobRef
GeneratePptx(deck with image blobs)  -> BlobRef
ExtractText(blob_id)                 -> BlobRef
```

The format arguments are the same Serde contracts used by the Rust API, except
that a slide image names a staged blob rather than carrying bytes inline.

No method returns bytes inline. A frame is a 16 MiB JSON document and a `Vec<u8>`
serialises as an array of integers — about 3.5 bytes of frame per byte — so the
real inline ceiling is a few megabytes, below both a deck's legal image payload
and any `.pdf` worth extracting. Every unbounded value is therefore staged and
moved in base64 chunks.

Invalid input, writer failures and extraction failures use the distinct wire
names `ai.tinyhumans.tinydocs.Error.InvalidInput`,
`ai.tinyhumans.tinydocs.Error.GenerationFailed` and
`ai.tinyhumans.tinydocs.Error.ExtractionFailed`. Transfer failures are grouped by
what the caller should do next: `Error.UnknownBlob` (restart the transfer),
`Error.TransferRefused` (a budget is full; the same request may succeed later)
and `Error.TransferFailed` (the caller sent something wrong; re-send).

Synthesis and extraction are CPU-bound and run on the module runtime's blocking
pool. The module retains no document state between calls — only staged blobs,
each bounded and expiring.

This interface replaces `ai.tinyhumans.tinydocs.Docx`, which returned bytes
inline. TinyBus forbids changing an interface in place, so the new contract took
a new name. It is not served alongside the old one: `module_export!` attaches its
method list to the first entry in `provides` and leaves the rest empty, so a
second fully-declared interface is not expressible without a TinyBus change.

## Invariants and constraints

- The vendored TinyBus gitlink is the ABI source of truth.
- Manifest methods and generated dispatch members must remain identical.
- No Rust value crosses the dynamic-library ABI boundary.
- The native artifact must match the host target and TinyBus compatibility
  gate.
- Message payloads remain subject to TinyBus's 16 MiB frame cap, which is why
  bytes move in bounded chunks rather than inline. Path or file-descriptor
  transfer would remove the copies and remains the better long-term answer.
- The staging area is bounded per chunk, per blob, in total and by blob count,
  and expires untouched blobs. A module is never unloaded, so an unbounded
  staging area is a leak with no end.
- A blob is verified against its declared SHA-256 before it becomes readable, so
  a truncated or reordered transfer cannot be consumed as though it were whole.
- Dynamic modules are trusted code with the host process's privileges.

## Acceptance criteria

- `cargo build --release --package tinydocs-module` emits the platform dynamic
  library.
- TinyBus `ModuleHost` admits that artifact and reaches `ready` state.
- `GenerateDocx` and `GeneratePptx` stage output beginning with the OOXML `PK`
  signature, and `ExtractText` recovers the text layer of a staged PDF.
- An image transferred across more than one chunk arrives intact and is embedded,
  which is the case a single-chunk transfer would not prove.
- CI executes that loader test on Linux.
- A release uploads Linux and macOS bundles containing the matching TinyBus
  host, TinyDocs module, SHA-256 allowlist, and operational documentation.
- A release uploads `checksum.toml` with the SHA-256 digest of every archive so
  TinyBus can verify a precompiled module before extracting or loading it.
- The release also uploads the crates.io package and pinned TinyBus source.

## Open questions

None blocking this version.

Two things belong upstream in TinyBus rather than here. The staging area is
format-agnostic and every module that moves bytes will want it, so it is a
candidate for the module SDK. And `module_export!` attaching its method list only
to the first provided interface is what forces one interface to carry both the
transfer and the format methods; per-interface method lists would allow the
cleaner split.
