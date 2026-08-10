# TinyBus module implementation plan

Specification: [`../specs/tinybus-module.md`](../specs/tinybus-module.md)

Goal: ship TinyDocs as a tested TinyBus dynamic module without changing the
default library dependency graph.

## Completed work

- [x] Advance `vendor/tinybus` to canonical `main` with module ABI v1.
- [x] Add a private adapter crate with TinyBus SDK, host, macro, and runtime
  dependencies so `tinydocs` remains publishable.
- [x] Emit both `rlib` and `cdylib` adapter crate types.
- [x] Add the typed DOCX service and stable bus identity.
- [x] Preserve domain-specific errors at the wire boundary.
- [x] Add unit coverage for identity, dispatch, and error mapping.
- [x] Add an ignored integration test that loads the built native artifact.
- [x] Run that real-loader test in CI.
- [x] Build and upload installable Linux and macOS TinyBus bundles, source
  packages, module allowlists, and documentation during release.
- [x] Publish a release-level `checksum.toml` so TinyBus can verify each
  precompiled module archive before extraction.
- [x] Document build, loading, trust, and payload constraints.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
.github/scripts/check-file-coverage.sh 90 coverage.json
cargo build --locked --release --package tinydocs-module
TINYDOCS_TEST_MODULE="$PWD/target/release/libtinydocs_module.so" \
  cargo test --locked --package tinydocs-module --test module_e2e -- --ignored
```
