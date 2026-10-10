$ErrorActionPreference = 'Stop'

cargo build --locked --release --package tinydocs-module
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$artifact = Join-Path $PWD 'target/release/tinydocs_module.dll'
if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
    throw "built module artifact was not found: $artifact"
}

$env:TINYDOCS_TEST_MODULE = $artifact
cargo test --locked --release --package tinydocs-module --test module_e2e -- --ignored
exit $LASTEXITCODE
