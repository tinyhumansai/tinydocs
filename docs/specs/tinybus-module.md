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
- Publish native module artifacts with each GitHub release.
- Test loading and calling the compiled artifact through a real broker.

## Non-goals

- Loading untrusted third-party modules safely.
- Stable ABI compatibility across TinyBus ABI revisions.
- Streaming or file-descriptor transfer in this first interface.
- Running TinyDocs as a separate socket process.

## Behavior

The private `tinydocs-module` workspace crate depends on the public library's
`docx` feature and builds as a `cdylib`. This separation keeps unpublished,
vendored TinyBus packages out of the crates.io package manifest. The module
claims `ai.tinyhumans.tinydocs.Docx`, serves the object path
`/ai/tinyhumans/tinydocs/Docx`, and exports one method:

```text
GenerateDocx(DocumentSpec) -> Vec<u8>
```

The argument is the same Serde document contract used by the Rust API. A
successful response contains a complete DOCX zip container. Invalid input and
writer failures use the distinct wire names
`ai.tinyhumans.tinydocs.Error.InvalidInput` and
`ai.tinyhumans.tinydocs.Error.GenerationFailed`.

Generation is CPU-bound and runs on the module runtime's blocking pool. The
module itself retains no document state between calls.

## Invariants and constraints

- The vendored TinyBus gitlink is the ABI source of truth.
- Manifest methods and generated dispatch members must remain identical.
- No Rust value crosses the dynamic-library ABI boundary.
- The native artifact must match the host target and TinyBus compatibility
  gate.
- Message payloads remain subject to TinyBus's 16 MiB frame cap. A future
  format that can exceed it must use path or file-descriptor transfer.
- Dynamic modules are trusted code with the host process's privileges.

## Acceptance criteria

- `cargo build --release --package tinydocs-module` emits the platform dynamic
  library.
- TinyBus `ModuleHost` admits that artifact and reaches `ready` state.
- A proxy call to `GenerateDocx` returns bytes beginning with the DOCX `PK`
  signature.
- CI executes that loader test on Linux.
- A release uploads one native module asset from each supported runner OS.

## Open questions

None blocking this version. Bulk transfer becomes a separate protocol change
if a future format approaches the frame cap.
