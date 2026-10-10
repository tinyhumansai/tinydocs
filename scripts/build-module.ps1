param(
    [Parameter(Mandatory = $true)][string]$Crate,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$HostKey,
    [string]$ExtraFilesJson = '[]',
    [string]$OutputDir = 'release-assets'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root
if ($Crate -ne 'tinydocs-module') { throw "Unsupported module crate: $Crate" }
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw "Invalid module version: $Version" }
if ($HostKey -notmatch '^[A-Za-z0-9_-]+$') { throw "Invalid release host key: $HostKey" }
$pathSeparators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
if ([IO.Path]::IsPathRooted($OutputDir) -or $OutputDir.Split($pathSeparators, [StringSplitOptions]::None) -contains '..') { throw 'Output directory must be a relative path inside the repository' }
$outputPath = [IO.Path]::GetFullPath((Join-Path $root $OutputDir))
$rootPrefix = $root.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $outputPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Output directory must stay inside the repository' }
$cursor = $root
foreach ($component in $OutputDir.Split($pathSeparators, [StringSplitOptions]::RemoveEmptyEntries)) {
    $cursor = Join-Path $cursor $component
    if (Test-Path -LiteralPath $cursor -PathType Container -and ((Get-Item -LiteralPath $cursor).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Output path contains a reparse point' }
}
New-Item -ItemType Directory -Force $outputPath | Out-Null
$outputPath = (Resolve-Path -LiteralPath $outputPath).Path
if (-not $outputPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Output directory must stay inside the repository' }
$extraFiles = @($ExtraFilesJson | ConvertFrom-Json)
foreach ($file in $extraFiles) {
    if ($file -isnot [string] -or $file -notmatch '^docs/[A-Za-z0-9._/-]+$' -or ($file.Split('/') -contains '..') -or ($file.Split('/') -contains '.')) {
        throw "Extra files must be repository-local docs paths without traversal: $file"
    }
    $source = Join-Path $root $file
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Extra file is missing: $file" }
    $cursor = $root
    foreach ($component in $file.Split('/')) {
        $cursor = Join-Path $cursor $component
        if ((Get-Item -LiteralPath $cursor).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Extra file path contains a reparse point: $file"
        }
    }
}
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
New-Item -ItemType Directory -Force $targetRoot | Out-Null
$targetRoot = (Resolve-Path -LiteralPath $targetRoot).Path
if (-not $targetRoot.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'CARGO_TARGET_DIR must stay inside the repository' }

cargo build --locked --release --package $Crate --target $Target
if ($LASTEXITCODE -ne 0) { throw 'Failed to build tinydocs-module' }
$stage = Join-Path $targetRoot 'module-package'
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
$extension = switch -Regex ($Target) {
    '^(x86_64|aarch64)-pc-windows-' { 'dll'; break }
    '^(x86_64|aarch64)-apple-darwin$' { 'dylib'; break }
    '^(x86_64|aarch64)-unknown-linux-gnu$' { 'so'; break }
    default { throw "Unsupported module target: $Target" }
}
$libraryName = if ($extension -eq 'dylib') { 'libtinydocs_module.dylib' } elseif ($extension -eq 'so') { 'libtinydocs_module.so' } else { 'tinydocs_module.dll' }
$library = Join-Path $targetRoot "$Target/release/$libraryName"
Copy-Item -LiteralPath $library -Destination $stage
$stagedLibrary = Join-Path $stage $libraryName
$hash = (Get-FileHash -LiteralPath $stagedLibrary -Algorithm SHA256).Hash.ToLowerInvariant()
'"{0}" = "{1}"' -f $libraryName, $hash | Set-Content -Path (Join-Path $stage 'modules.toml') -Encoding utf8NoBOM
Copy-Item -LiteralPath 'LICENSE', 'README.md' -Destination $stage
foreach ($file in $extraFiles) {
    $destination = Join-Path $stage $file
    New-Item -ItemType Directory -Force (Split-Path $destination) | Out-Null
    Copy-Item -LiteralPath $file -Destination $destination
}
$archive = Join-Path $outputPath "$Crate-$Version-$HostKey.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -Force
Write-Output $archive
