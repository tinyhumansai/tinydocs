#!/usr/bin/env bash
set -euo pipefail

cargo build --locked --release --package tinydocs-module

case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) artifact="target/release/tinydocs_module.dll" ;;
  macOS|Darwin) artifact="target/release/libtinydocs_module.dylib" ;;
  Linux) artifact="target/release/libtinydocs_module.so" ;;
  *) echo "unsupported test host: ${RUNNER_OS:-$(uname -s)}" >&2; exit 1 ;;
esac

if [[ ! -f "$artifact" ]]; then
  echo "built module artifact was not found: $artifact" >&2
  exit 1
fi

TINYDOCS_TEST_MODULE="$PWD/$artifact" \
  cargo test --locked --release --package tinydocs-module --test module_e2e -- --ignored
