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

test_dir="target/tinydocs-module-e2e"
mkdir -p "$test_dir"
test_module="$test_dir/$(basename "$artifact")"
cp "$artifact" "$test_module"
if command -v sha256sum >/dev/null 2>&1; then
  module_hash="$(sha256sum "$test_module" | awk '{print $1}')"
else
  module_hash="$(shasum -a 256 "$test_module" | awk '{print $1}')"
fi
printf '"%s" = "%s"\n' "$(basename "$test_module")" "$module_hash" \
  > "$test_dir/modules.toml"

TINYDOCS_TEST_MODULE="$PWD/$test_module" \
  cargo test --locked --release --package tinydocs-module --test module_e2e --features module-test-support -- --ignored
