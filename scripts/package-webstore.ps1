# Builds the Chrome Web Store upload from the built extension (house pipeline: chrome-webstore-publish).
#
# What ships is packages/extension/dist, produced by the build. This script never builds: it
# validates, stages exactly what the validation approved, runs the layer 4 doxguard scan against
# those staged copies, and only then writes the zip and its SHA256.
#
#   pnpm -F vtype-core build
#   pnpm -F vtype-extension build
#   pwsh -NoProfile -File scripts/package-webstore.ps1
#
# Output (both gitignored):
#   dist/release-assets/vtype-vX.Y.Z-webstore.zip
#   dist/release-assets/SHA256SUMS-vX.Y.Z.txt

param(
  [switch]$SkipValidate
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$slug = 'vtype'
$extDist = Join-Path $root 'packages/extension/dist'
$validateScript = Join-Path $PSScriptRoot 'validate-extension.ps1'

if (-not (Test-Path -LiteralPath $extDist -PathType Container)) {
  throw ("The built extension is missing: $extDist" + [Environment]::NewLine +
    'Run "pnpm -F vtype-core build" then "pnpm -F vtype-extension build" first.')
}

if ($SkipValidate) {
  Write-Warning 'validate-extension.ps1 was skipped (-SkipValidate). Do not upload a package built this way.'
} else {
  & pwsh -NoProfile -File $validateScript
  if ($LASTEXITCODE -ne 0) { throw "validate-extension.ps1 failed (exit code $LASTEXITCODE); refusing to package." }
}

$manifest = Get-Content -Raw -LiteralPath (Join-Path $extDist 'manifest.json') | ConvertFrom-Json
$version = $manifest.version

$releaseDir = Join-Path $root 'dist/release-assets'
$stagingDir = Join-Path $root "dist/staging/$slug"
$zipPath = Join-Path $releaseDir "$slug-v$version-webstore.zip"

if (Test-Path -LiteralPath $stagingDir) { Remove-Item -LiteralPath $stagingDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stagingDir, $releaseDir | Out-Null

# The whole built folder, and nothing else: validate-extension.ps1 has already asserted that it
# holds exactly the expected files, so a hand-kept list here would only be a second place to drift.
Copy-Item -Path (Join-Path $extDist '*') -Destination $stagingDir -Recurse -Force
$staged = Get-ChildItem -LiteralPath $stagingDir -Recurse -File

# Layer 4 gate: scan exactly the bytes that go into the zip. Fail closed without Node.
$node = Get-Command node -ErrorAction SilentlyContinue
if (-not $node) { throw 'Node.js was not found; refusing to package without the doxguard gate.' }

$listPath = Join-Path ([System.IO.Path]::GetTempPath()) "$slug-package-$PID.list"
$staged | ForEach-Object { $_.FullName } | Set-Content -LiteralPath $listPath -Encoding UTF8
try {
  # doxguard (exact 0.2.0) through the shared adapter. It resolves doxguard from DOXGUARD_PACKAGE_ROOT
  # or node_modules, and must run from the repo root (it reads doxguard.ci.json and checks git HEAD there).
  # A missing doxguard, any hit or any error exits nonzero, so packaging stops (fail closed).
  Push-Location -LiteralPath $root
  try {
    & $node.Source (Join-Path $PSScriptRoot 'secrets-scan.dual.mjs') '--native-only' '--path-from' $listPath
    $scanExit = $LASTEXITCODE
  } finally { Pop-Location }
  if ($scanExit -ne 0) {
    throw ("doxguard scan blocked packaging (exit code $scanExit)." + [Environment]::NewLine +
      'If doxguard is not installed: run "npm install doxguard@0.2.0 --prefix <dir>" and set DOXGUARD_PACKAGE_ROOT to <dir>/node_modules/doxguard, then run this script again. Otherwise fix the reported hits.')
  }
} finally {
  Remove-Item -LiteralPath $listPath -Force -ErrorAction SilentlyContinue
}

if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }

# ZipFile rather than Compress-Archive: it writes forward-slash entry names on every platform,
# which is what the store expects for icons/icon16.png.
Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
[System.IO.Compression.ZipFile]::CreateFromDirectory(
  $stagingDir,
  $zipPath,
  [System.IO.Compression.CompressionLevel]::Optimal,
  $false
)

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToLowerInvariant()
$sumsPath = Join-Path $releaseDir "SHA256SUMS-v$version.txt"
$zipName = Split-Path -Leaf $zipPath
[System.IO.File]::WriteAllText($sumsPath, "$hash  $zipName`n", [System.Text.UTF8Encoding]::new($false))

Write-Host ''
Write-Host "Packaged: $zipPath"
Write-Host "Files:    $($staged.Count)"
Write-Host "SHA256:   $hash"
Write-Host "Sums:     $sumsPath"
Write-Host ''
Write-Host "Next: docs/store/listing.ja.md and docs/store/submission-notes-v$version.ja.md go into the"
Write-Host 'dashboard by hand. The store pages cannot be driven by browser automation.'
