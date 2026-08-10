# TinyDocs

Agent-friendly document synthesis and text extraction in Rust.

`tinydocs` turns a typed, validated document spec into real office-format
bytes. It is built for hosts that let a language model produce documents: the
spec types double as the JSON tool schema, validation rejects a malformed spec
with a structured error naming the exact offending field so the model can
self-correct, and synthesis hands back a plain byte buffer.

```rust
use tinydocs::docx::{self, DocumentSection, DocumentSpec};

let spec = DocumentSpec {
    title: "Weekly Report".to_string(),
    author: Some("Ferris".to_string()),
    sections: vec![DocumentSection {
        heading: Some("Highlights".to_string()),
        paragraphs: vec!["Throughput doubled.".to_string()],
        bullets: vec!["Shipped the parser".to_string()],
    }],
};

let bytes = docx::generate(&spec)?;
std::fs::write("report.docx", bytes)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## What it does not do

No filesystem access, no subprocesses, no async runtime, no deadline handling.
`docx::generate` is synchronous and CPU-bound.

That is a deliberate seam, not an omission. A host running on an async executor
owns the blocking-pool hop and the timeout, because only the host knows its own
executor and deadline policy — a crate that guessed at either would be wrong
for every host that guessed differently. The typical async caller looks like:

```rust,ignore
let spec = spec.clone();
let bytes = tokio::time::timeout(
    deadline,
    tokio::task::spawn_blocking(move || tinydocs::docx::generate(&spec)),
)
.await???;
```

## Validation

Every limit is a public constant, so a host can quote the exact number in its
own tool description and stay in lockstep with what validation enforces.

| Limit | Value | Bounds |
| --- | --- | --- |
| `MAX_SECTIONS` | 128 | sections per document |
| `MAX_TEXT_CHARS` | 2,000 | title, author, section heading |
| `MAX_PARAGRAPH_CHARS` | 20,000 | one paragraph or bullet |
| `MAX_PARAGRAPHS_PER_SECTION` | 200 | paragraphs per section |
| `MAX_BULLETS_PER_SECTION` | 200 | bullets per section |
| `MAX_TOTAL_CHARS` | 2,000,000 | all text in the document |

The aggregate cap is the load-bearing one. The per-field limits bound each
individual piece but not their product — `MAX_SECTIONS ×
MAX_PARAGRAPHS_PER_SECTION × MAX_PARAGRAPH_CHARS` alone is over 500M
characters, so a spec satisfying every other limit could still build a
multi-hundred-megabyte document in memory.

`DocumentSpec::validate` is public and runs before any synthesis, so a host can
reject a bad tool call at its own boundary without paying for a blocking hop.

## TinyBus module

The private `tinydocs-module` workspace crate builds TinyDocs as a trusted
in-process TinyBus module while keeping the published library bus-agnostic:

```sh
cargo build --release --package tinydocs-module
```

The native artifact is `target/release/libtinydocs_module.so` on Linux,
`libtinydocs_module.dylib` on macOS, or `tinydocs_module.dll` on Windows. Load
it with a TinyBus host built with its `modules` feature. It claims
`ai.tinyhumans.tinydocs.Docx` at `/ai/tinyhumans/tinydocs/Docx` and exposes:

```text
GenerateDocx(DocumentSpec) -> Vec<u8>
```

The release workflow attaches installable Linux and macOS bundles containing
the matching TinyBus host, the TinyDocs module, a SHA-256 `modules.toml`
allowlist, and protocol/module documentation. It also attaches the published
crate and pinned TinyBus source. TinyBus modules are target-specific and
trusted: download the bundle matching the host, and install it only from a
trusted release.

Run the real loader test locally after building the release artifact:

```sh
TINYDOCS_TEST_MODULE="$PWD/target/release/libtinydocs_module.so" \
  cargo test --package tinydocs-module --test module_e2e -- --ignored
```

## Feature flags

| Feature | Default | Gates |
| --- | --- | --- |
| `docx` | on | `.docx` synthesis via `docx-rs` |

## Layout

```text
src/
├── lib.rs              # crate docs + the entire public re-export surface
├── error/
│   ├── mod.rs          # crate-wide `Error` and `Result<T>`
│   └── test.rs
├── docx/
    ├── mod.rs          # `generate` + spec validation
    ├── types.rs        # `DocumentSpec`, `DocumentSection`, limits
    └── test.rs
tests/
└── public_api.rs       # integration tests against the public API only
crates/
└── tinydocs-module/    # private TinyBus cdylib adapter + loader E2E test
examples/
└── basic.rs            # compiled and linted in CI
```

## Development

```sh
git submodule update --init --recursive

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run --example basic
.github/scripts/check-file-coverage.sh 90 coverage.json
```

Run the gated build too — it is the only thing that catches a feature that
compiles only when it is turned on:

```sh
cargo clippy --all-targets --no-default-features -- -D warnings
```

## Documentation

- [`AGENTS.md`](AGENTS.md) — repository guidelines for humans and agents
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to propose a change
- [`SECURITY.md`](SECURITY.md) — how to report a vulnerability

## License

GPL-3.0-only. See [LICENSE](LICENSE).
