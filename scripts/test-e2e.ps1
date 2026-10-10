$ErrorActionPreference = 'Stop'

cargo build --locked --release --package tinydocs-module
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$artifact = Join-Path $PWD 'target/release/tinydocs_module.dll'
if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
    throw "built module artifact was not found: $artifact"
}

$testDirectory = Join-Path $PWD 'target/tinydocs-module-e2e'
if (Test-Path -LiteralPath $testDirectory) {
    Remove-Item -LiteralPath $testDirectory -Recurse -Force
}
New-Item -ItemType Directory -Force $testDirectory | Out-Null
# GitHub's Windows runner inherits broad write ACEs from D:\a. TinyBus checks
# the staged module directory's DACL before loading it, so make this test-only
# directory private to the runner, Administrators, and SYSTEM before staging.
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$runnerSid = "*$($identity.User.Value):(OI)(CI)F"
$adminSid = '*S-1-5-32-544:(OI)(CI)F'
$systemSid = '*S-1-5-18:(OI)(CI)F'
icacls $testDirectory /inheritance:r /grant:r $runnerSid $adminSid $systemSid | Out-Null
if ($LASTEXITCODE -ne 0) { throw "could not restrict module test directory permissions: $testDirectory" }
icacls $testDirectory /setowner $identity.Name | Out-Null
if ($LASTEXITCODE -ne 0) { throw "could not set module test directory owner: $testDirectory" }
$testModule = Join-Path $testDirectory 'tinydocs_module.dll'
Copy-Item -LiteralPath $artifact -Destination $testModule -Force
$moduleHash = (Get-FileHash -LiteralPath $testModule -Algorithm SHA256).Hash.ToLowerInvariant()
$moduleName = Split-Path -Leaf $testModule
'"{0}" = "{1}"' -f $moduleName, $moduleHash |
    Set-Content -LiteralPath (Join-Path $testDirectory 'modules.toml') -Encoding utf8NoBOM

$env:TINYDOCS_TEST_MODULE = $testModule
cargo test --locked --release --package tinydocs-module --test module_e2e -- --ignored
exit $LASTEXITCODE
