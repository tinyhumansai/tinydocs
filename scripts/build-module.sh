#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
target_root="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target_root" == /* ]] || target_root="$root/$target_root"
mkdir -p "$target_root"
target_root="$(cd "$target_root" && pwd -P)"
[[ "$target_root" == "$root"/* ]] || { echo "CARGO_TARGET_DIR must stay inside the repository" >&2; exit 2; }
crate="" version="" rust_target="" host="" output="release-assets" extras='[]'
while (($#)); do
  case "$1" in
    --crate) crate="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --target) rust_target="$2"; shift 2 ;;
    --host) host="$2"; shift 2 ;;
    --extra-files-json) extras="$2"; shift 2 ;;
    --output-dir) output="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$crate" == tinydocs-module && "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && -n "$rust_target" && "$host" =~ ^[A-Za-z0-9_-]+$ ]] || {
  echo "usage: build-module.sh --crate tinydocs-module --version VERSION --target TARGET --host HOST [--extra-files-json JSON] [--output-dir DIR]" >&2
  exit 2
}
case "$rust_target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) library=libtinydocs_module.so ;;
  x86_64-apple-darwin|aarch64-apple-darwin) library=libtinydocs_module.dylib ;;
  x86_64-pc-windows-gnu|aarch64-pc-windows-gnullvm|x86_64-pc-windows-msvc|aarch64-pc-windows-msvc) library=tinydocs_module.dll ;;
  *) echo "unsupported module target: $rust_target" >&2; exit 2 ;;
esac
[[ "$output" != /* && "$output" != .. && "$output" != ../* && "$output" != */../* && "$output" != */.. ]] || {
  echo "output directory must stay inside the repository" >&2
  exit 2
}
path="$root"
IFS=/ read -r -a output_components <<<"$output"
for component in "${output_components[@]}"; do
  path="$path/$component"
  [[ ! -L "$path" ]] || { echo "output path contains a symbolic link: $output" >&2; exit 2; }
done
jq -e 'type == "array" and all(.[]; type == "string" and test("^docs/[A-Za-z0-9._/-]+$") and all(split("/"); . != "." and . != ".."))' <<<"$extras" >/dev/null || {
  echo "extra files must be repository-local docs paths without traversal" >&2
  exit 2
}
cargo build --locked --release --package "$crate" --target "$rust_target"
stage="$target_root/module-package"
rm -rf "$stage"
mkdir -p "$stage" "$output"
output="$(cd "$output" && pwd -P)"
[[ "$output" == "$root"/* ]] || { echo "output directory must stay inside the repository" >&2; exit 2; }
install -m 755 "$target_root/$rust_target/release/$library" "$stage/$library"
if command -v sha256sum >/dev/null 2>&1; then
  hash="$(sha256sum "$stage/$library" | awk '{print $1}')"
else
  hash="$(shasum -a 256 "$stage/$library" | awk '{print $1}')"
fi
printf '"%s" = "%s"\n' "$library" "$hash" > "$stage/modules.toml"
cp LICENSE README.md "$stage/"
while IFS= read -r file; do
  [[ -f "$root/$file" ]] || { echo "extra file is missing: $file" >&2; exit 2; }
  path="$root"
  IFS=/ read -r -a components <<<"$file"
  for component in "${components[@]}"; do
    path="$path/$component"
    [[ ! -L "$path" ]] || { echo "extra file path contains a symbolic link: $file" >&2; exit 2; }
  done
  mkdir -p "$stage/$(dirname "$file")"
  cp "$file" "$stage/$file"
done < <(jq -r '.[]' <<<"$extras")
archive="$output/${crate}-${version}-${host}.tar.gz"
tar -C "$stage" -czf "$archive" .
echo "$archive"
