<#
.SYNOPSIS
  Builds the Microsoft Store MSIX package of vtype desktop from the release executable.

.DESCRIPTION
  Stages vtype.exe, tile images drawn from assets/icons/icon-512.png, and the tracked manifest
  (packages/native/packaging/msix/AppxManifest.xml) with its {{…}} fields filled; packs it with
  MakeAppx; unpacks the result and checks the manifest again.

  The Identity defaults are the values Partner Center gave when the name was reserved
  (2026-09-24, Store ID 9PJSKZSMRV57, Apps and games > vtype > Product identity). They are not
  secret: every published package carries them.

  Build the executable first (from packages/native): cargo build --release --locked

  -Sign signs with a local test certificate (for sideloading); -Install sideloads the package.
  Store submissions are not signed here: the Store signs what it accepts.
  Nothing is submitted by this script.
#>
param(
  [string]$ExePath,
  [string]$OutDir,
  [string]$IdentityName = "ishizakahiroshi.vtype",
  [string]$Publisher = "CN=A454C7F3-0506-42C1-AB41-2BE056B76ABF",
  [string]$PublisherDisplayName = "ishizakahiroshi",
  [switch]$Sign,
  [switch]$Install
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$NativeDir = Join-Path $RepoRoot "packages\native"
if (-not $ExePath) { $ExePath = Join-Path $NativeDir "target\release\vtype.exe" }
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot "dist\msix" }
$ManifestTemplate = Join-Path $NativeDir "packaging\msix\AppxManifest.xml"
$IconSource = Join-Path $RepoRoot "assets\icons\icon-512.png"
$CargoToml = Join-Path $NativeDir "Cargo.toml"
$WorkDir = Join-Path $RepoRoot "dist\msix-work"
$StageDir = Join-Path $WorkDir "package"
$VerifyDir = Join-Path $WorkDir "verified-package"

function Find-WindowsSdkTool {
  param([Parameter(Mandatory = $true)][string]$Name)
  $sdkBin = "C:\Program Files (x86)\Windows Kits\10\bin"
  if (-not (Test-Path -LiteralPath $sdkBin)) { return $null }
  Get-ChildItem -LiteralPath $sdkBin -Recurse -Filter $Name -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match "\\x64\\" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
}

# The parts of the manifest that vtype relies on; a change to any of them should be deliberate.
function Assert-Manifest {
  param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Version)
  [xml]$doc = Get-Content -Raw -LiteralPath $Path -Encoding UTF8
  $ns = [System.Xml.XmlNamespaceManager]::new($doc.NameTable)
  $ns.AddNamespace("f", "http://schemas.microsoft.com/appx/manifest/foundation/windows10")
  $ns.AddNamespace("uap5", "http://schemas.microsoft.com/appx/manifest/uap/windows10/5")
  $ns.AddNamespace("desktop", "http://schemas.microsoft.com/appx/manifest/desktop/windows10")
  $ns.AddNamespace("rescap", "http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities")
  $checks = [ordered]@{
    "Identity Version $Version"             = "/f:Package/f:Identity[@Version='$Version']"
    "Application vtype.exe (full trust)"   = "/f:Package/f:Applications/f:Application[@Executable='vtype.exe' and @EntryPoint='Windows.FullTrustApplication']"
    "StartupTask"                           = "//desktop:Extension[@Category='windows.startupTask']/desktop:StartupTask"
    "execution alias vtype.exe"             = "//uap5:ExecutionAlias[@Alias='vtype.exe']"
    "runFullTrust"                          = "/f:Package/f:Capabilities/rescap:Capability[@Name='runFullTrust']"
  }
  foreach ($name in $checks.Keys) {
    if (-not $doc.SelectSingleNode($checks[$name], $ns)) { throw "Manifest check failed ($name): $Path" }
  }
  if ((Get-Content -Raw -LiteralPath $Path) -match "\{\{") { throw "Manifest still has {{…}} fields: $Path" }
  # Nothing registers with Chrome any more, so no restricted capability besides runFullTrust.
  $restricted = @($doc.SelectNodes("/f:Package/f:Capabilities/rescap:Capability[@Name!='runFullTrust']", $ns))
  if ($restricted.Count -gt 0) { throw "Unexpected restricted capability ($($restricted.Name -join ', ')): $Path" }
}

function Save-Resized {
  param([string]$Source, [int]$Size, [string]$Destination)
  Add-Type -AssemblyName System.Drawing
  $src = [System.Drawing.Image]::FromFile($Source)
  try {
    $bmp = New-Object System.Drawing.Bitmap $Size, $Size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.DrawImage($src, 0, 0, $Size, $Size)
    $g.Dispose()
    $bmp.Save($Destination, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
  } finally {
    $src.Dispose()
  }
}

$makeAppx = Find-WindowsSdkTool -Name "makeappx.exe"
if (-not $makeAppx) { throw "makeappx.exe was not found. Install the Windows SDK." }
if (-not (Test-Path -LiteralPath $ExePath)) {
  throw "The release executable was not found: $ExePath (run: cargo build --release --locked in packages/native)"
}

$cargoText = Get-Content -Raw -LiteralPath $CargoToml
$match = [regex]::Match($cargoText, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"')
if (-not $match.Success) { throw "Could not read the version from $CargoToml" }
$packageVersion = "$($match.Groups[1].Value).0"
Write-Host "Package version: $packageVersion"

if (Test-Path -LiteralPath $WorkDir) { Remove-Item -LiteralPath $WorkDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $StageDir "Assets") | Out-Null
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Copy-Item -LiteralPath $ExePath -Destination (Join-Path $StageDir "vtype.exe")

$tiles = [ordered]@{ "Square44x44Logo.png" = 44; "StoreLogo.png" = 50; "Square71x71Logo.png" = 71; "Square150x150Logo.png" = 150 }
foreach ($name in $tiles.Keys) {
  Save-Resized -Source $IconSource -Size $tiles[$name] -Destination (Join-Path $StageDir "Assets\$name")
}

$manifest = Get-Content -Raw -LiteralPath $ManifestTemplate -Encoding UTF8
$manifest = $manifest.Replace("{{VERSION}}", $packageVersion).
  Replace("{{IDENTITY_NAME}}", $IdentityName).
  Replace("{{PUBLISHER}}", $Publisher).
  Replace("{{PUBLISHER_DISPLAY_NAME}}", $PublisherDisplayName)
# The header comment names the fields in {{…}} form; keep it out of the package.
$manifest = [regex]::Replace($manifest, '(?s)<!--.*?-->\s*', '', [System.Text.RegularExpressions.RegexOptions]::None, [TimeSpan]::FromSeconds(5))
$staged = Join-Path $StageDir "AppxManifest.xml"
Set-Content -LiteralPath $staged -Value $manifest -Encoding UTF8 -NoNewline
Assert-Manifest -Path $staged -Version $packageVersion

$msixPath = Join-Path $OutDir "vtype-$($match.Groups[1].Value).msix"
if (Test-Path -LiteralPath $msixPath) { Remove-Item -LiteralPath $msixPath -Force }
& $makeAppx pack /d $StageDir /p $msixPath /o
if ($LASTEXITCODE -ne 0) { throw "MakeAppx pack failed with exit code $LASTEXITCODE" }
& $makeAppx unpack /p $msixPath /d $VerifyDir /o
if ($LASTEXITCODE -ne 0) { throw "MakeAppx unpack failed with exit code $LASTEXITCODE" }
Assert-Manifest -Path (Join-Path $VerifyDir "AppxManifest.xml") -Version $packageVersion
Write-Host "Checked the manifest inside the package."

if ($Sign) {
  $signTool = Find-WindowsSdkTool -Name "signtool.exe"
  if (-not $signTool) { throw "signtool.exe was not found. Install the Windows SDK." }
  $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $Publisher } | Select-Object -First 1
  if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher -KeyUsage DigitalSignature `
      -FriendlyName "vtype MSIX (local test)" -CertStoreLocation "Cert:\CurrentUser\My" `
      -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}Subject Type:End Entity")
  }
  & $signTool sign /fd SHA256 /sha1 $cert.Thumbprint $msixPath
  if ($LASTEXITCODE -ne 0) { throw "SignTool failed with exit code $LASTEXITCODE" }
  Export-Certificate -Cert $cert -FilePath (Join-Path $OutDir "vtype-local-test.cer") -Force | Out-Null
  Write-Host "Signed with a local test certificate (trust it in Cert:\LocalMachine\TrustedPeople to sideload)."
}

if ($Install) {
  if ($Sign) { Add-AppxPackage -Path $msixPath } else { Add-AppxPackage -Path $msixPath -AllowUnsigned }
  Write-Host "Sideloaded: $msixPath"
}

Remove-Item -LiteralPath $WorkDir -Recurse -Force
Write-Host ""
Write-Host "MSIX package: $msixPath"
