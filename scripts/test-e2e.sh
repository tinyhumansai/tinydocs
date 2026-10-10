#!/usr/bin/env bash
set -euo pipefail

case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*)
    pwsh -NoProfile -File scripts/test-e2e.ps1
    exit
    ;;
  macOS|Darwin) artifact="target/release/libtinydocs_module.dylib" ;;
  Linux) artifact="target/release/libtinydocs_module.so" ;;
  *) echo "unsupported test host: ${RUNNER_OS:-$(uname -s)}" >&2; exit 1 ;;
esac

cargo build --locked --release --package tinydocs-module

if [[ ! -f "$artifact" ]]; then
  echo "built module artifact was not found: $artifact" >&2
  exit 1
fi

TINYDOCS_TEST_MODULE="$PWD/$artifact" \
  cargo test --locked --release --package tinydocs-module --test module_e2e -- --ignored
