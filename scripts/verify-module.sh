#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [[ "${1:-}" == --archive ]]; then shift; fi
archive="${1:?usage: verify-module.sh --archive <module-archive>}"
target_root="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target_root" == /* ]] || target_root="$root/$target_root"
mkdir -p "$target_root"
target_root="$(cd "$target_root" && pwd -P)"
[[ "$target_root" == "$root"/* ]] || { echo "CARGO_TARGET_DIR must stay inside the repository" >&2; exit 2; }
work="$target_root/tinydocs-module-verify-$$"
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
case "$archive" in
  *.tar.gz)
    members="$(tar -tzf "$archive")"
    while IFS= read -r member; do
      case "$member" in
        /*|../*|*/../*|*/..) echo "archive member escapes the extraction directory: $member" >&2; exit 2 ;;
      esac
    done <<<"$members"
    if tar -tvzf "$archive" | awk 'substr($0, 1, 1) !~ /^[-d]$/ { exit 1 }'; then
      tar -xzf "$archive" -C "$work"
    else
      echo "archive contains a link or special file" >&2
      exit 2
    fi
    ;;
  *) echo "unsupported module archive: $archive" >&2; exit 2 ;;
esac
case "$(uname -s)" in
  Darwin) library="$work/libtinydocs_module.dylib" ;;
  Linux) library="$work/libtinydocs_module.so" ;;
  *) echo "verify-module.sh requires a Unix runner" >&2; exit 1 ;;
esac
test -f "$work/modules.toml"
TINYDOCS_TEST_MODULE="$library" cargo test --locked --release \
  --package tinydocs-module --test module_e2e -- --ignored
