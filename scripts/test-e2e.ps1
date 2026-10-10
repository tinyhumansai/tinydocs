$ErrorActionPreference = 'Stop'

cargo build --locked --release --package tinydocs-module
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$artifact = Join-Path $PWD 'target/release/tinydocs_module.dll'
if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
    throw "built module artifact was not found: $artifact"
}

$testDirectory = Join-Path $PWD 'target/tinydocs-module-e2e'
New-Item -ItemType Directory -Force $testDirectory | Out-Null
$testModule = Join-Path $testDirectory 'tinydocs_module.dll'
Copy-Item -LiteralPath $artifact -Destination $testModule -Force
$moduleHash = (Get-FileHash -LiteralPath $testModule -Algorithm SHA256).Hash.ToLowerInvariant()
$moduleName = Split-Path -Leaf $testModule
'"{0}" = "{1}"' -f $moduleName, $moduleHash |
    Set-Content -LiteralPath (Join-Path $testDirectory 'modules.toml') -Encoding utf8NoBOM

$env:TINYDOCS_TEST_MODULE = $testModule
cargo test --locked --release --package tinydocs-module --test module_e2e -- --ignored
exit $LASTEXITCODE
