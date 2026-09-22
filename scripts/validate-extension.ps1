# Pre-package validation for the Chrome Web Store submission (house pipeline: chrome-webstore-publish).
#
# vtype is a pnpm workspace, so what ships is not the source tree: it is the built, unpacked
# extension in packages/extension/dist. This script validates that built folder, plus the
# repository-level documents the store submission depends on.
#
# Run the build first (it is not run from here: building is the author's call, not a side effect
# of a check):
#
#   pnpm -F vtype-core build
#   pnpm -F vtype-extension build
#   pwsh -NoProfile -File scripts/validate-extension.ps1
#
# Exit code 1 with a list of errors means do not package. Warnings do not block.

param(
  [switch]$SkipNodeCheck,
  [string]$Dist
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$extRoot = Join-Path $root 'packages/extension'
$distDir = if ($Dist) { $Dist } else { Join-Path $extRoot 'dist' }
$sourceManifestPath = Join-Path $extRoot 'manifest.json'

$errors = [System.Collections.Generic.List[string]]::new()
$warnings = [System.Collections.Generic.List[string]]::new()

function Add-ValidationError { param([string]$Message) $script:errors.Add($Message) | Out-Null }
function Add-ValidationWarning { param([string]$Message) $script:warnings.Add($Message) | Out-Null }

function Test-RepoFile {
  param([string]$RelativePath, [string]$Why)
  if (-not (Test-Path -LiteralPath (Join-Path $root $RelativePath) -PathType Leaf)) {
    Add-ValidationError "Missing: $RelativePath ($Why)"
  }
}

function Get-PngDimension {
  param([byte[]]$Bytes, [int]$Offset)
  return (($Bytes[$Offset] -shl 24) -bor ($Bytes[$Offset + 1] -shl 16) -bor ($Bytes[$Offset + 2] -shl 8) -bor $Bytes[$Offset + 3])
}

function Test-PngIcon {
  param([string]$FullPath, [string]$Label, [int]$ExpectedSize)
  if (-not (Test-Path -LiteralPath $FullPath -PathType Leaf)) {
    Add-ValidationError "Missing icon: $Label"
    return
  }
  $bytes = [System.IO.File]::ReadAllBytes($FullPath)
  if ($bytes.Length -lt 24) { Add-ValidationError "Icon is too small to be a PNG: $Label"; return }
  $signature = [byte[]](0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)
  for ($i = 0; $i -lt $signature.Length; $i++) {
    if ($bytes[$i] -ne $signature[$i]) { Add-ValidationError "Icon is not a PNG file: $Label"; return }
  }
  $width = Get-PngDimension -Bytes $bytes -Offset 16
  $height = Get-PngDimension -Bytes $bytes -Offset 20
  if ($width -ne $ExpectedSize -or $height -ne $ExpectedSize) {
    Add-ValidationError "Icon has unexpected dimensions: $Label ($width x $height, expected $ExpectedSize x $ExpectedSize)"
  }
}

# --- The built extension must exist -----------------------------------------------------------

if (-not (Test-Path -LiteralPath $distDir -PathType Container)) {
  throw ("The built extension is missing: $distDir" + [Environment]::NewLine +
    'Run "pnpm -F vtype-core build" then "pnpm -F vtype-extension build" first.')
}
$distDir = (Resolve-Path -LiteralPath $distDir).Path

$manifestPath = Join-Path $distDir 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
  throw "manifest.json is missing from the build: $manifestPath"
}

try {
  $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
} catch {
  throw "The built manifest.json is not valid JSON: $($_.Exception.Message)"
}

# --- manifest ---------------------------------------------------------------------------------

if ($manifest.manifest_version -ne 3) { Add-ValidationError 'manifest_version must be 3.' }
if (-not $manifest.name) { Add-ValidationError 'manifest.name is required.' }
if (-not $manifest.description) { Add-ValidationError 'manifest.description is required (the store shows it).' }
if ($manifest.version -notmatch '^\d+\.\d+\.\d+$') {
  Add-ValidationError "manifest.version must use x.y.z format: $($manifest.version)"
}

# The source manifest is the single source of the version; the build only copies it.
if (Test-Path -LiteralPath $sourceManifestPath -PathType Leaf) {
  $sourceManifest = Get-Content -Raw -LiteralPath $sourceManifestPath | ConvertFrom-Json
  if ($sourceManifest.version -ne $manifest.version) {
    Add-ValidationError "The built manifest is stale: packages/extension/manifest.json is $($sourceManifest.version), dist is $($manifest.version). Rebuild."
  }
}

if (-not $manifest.background.service_worker) {
  Add-ValidationError 'background.service_worker is required.'
}
# Deliberately the opposite of most extensions: every vtype bundle is a classic script, because
# a classic service worker and a classic content script cannot import modules (see build.mjs).
if ($manifest.background.type -eq 'module') {
  Add-ValidationError 'background.type must not be "module": the service worker ships as one classic bundle.'
}

$permissions = @($manifest.permissions)
$expectedPermissions = @('offscreen', 'storage')
foreach ($permission in $expectedPermissions) {
  if ($permissions -notcontains $permission) { Add-ValidationError "Missing manifest permission: $permission" }
}
foreach ($permission in $permissions) {
  if ($expectedPermissions -notcontains $permission) {
    Add-ValidationError "Unexpected permission '$permission'. Every added permission needs a justification in docs/store/listing.*.md and in the submission notes."
  }
}
# The desktop link's permission is optional: asked for from the options page, never at install,
# so that adding it does not disable the extension for everyone who already has it.
$optionalPermissions = @($manifest.optional_permissions | Where-Object { $_ })
foreach ($permission in $optionalPermissions) {
  if ($permission -ne 'nativeMessaging') {
    Add-ValidationError "Unexpected optional permission '$permission'. Only nativeMessaging (the desktop link) is expected."
  }
}
if ($permissions -contains 'nativeMessaging') {
  Add-ValidationError 'nativeMessaging must stay in optional_permissions: as a required permission, the update would disable vtype for every existing user until they accept it.'
}
if ($manifest.host_permissions) {
  Add-ValidationError 'host_permissions must stay empty: vtype reaches pages through the content script only.'
}
if (-not $manifest.content_scripts -or @($manifest.content_scripts).Count -eq 0) {
  Add-ValidationError 'content_scripts is required (that is how the mic reaches a page).'
}
if (-not $manifest.options_ui.page) {
  Add-ValidationError 'options_ui.page is required (the settings page is the only way to the excluded-site list).'
}

# --- the shipped file set ----------------------------------------------------------------------

$localesDir = Join-Path $extRoot '_locales'
$localeCodes = @()
if (Test-Path -LiteralPath $localesDir -PathType Container) {
  $localeCodes = @(Get-ChildItem -LiteralPath $localesDir -Directory | ForEach-Object { $_.Name } | Sort-Object)
}

$expectedFiles = @(
  'manifest.json',
  'background.js',
  'content.js',
  'offscreen.html',
  'offscreen.js',
  'options.html',
  'options.js',
  'permission.html',
  'permission.js',
  'icons/icon16.png',
  'icons/icon32.png',
  'icons/icon48.png',
  'icons/icon128.png'
)
# One more language is one more file, here and in the package: nothing else in this script
# names a locale.
foreach ($code in $localeCodes) { $expectedFiles += "_locales/$code/messages.json" }
# Kana mode's kanji reading: kuromoji's IPADIC dictionary (12 gzip files) and the licenses it
# ships under. The dictionary's own terms require NOTICE.md to travel with every copy.
foreach ($name in @('base', 'cc', 'check', 'tid', 'tid_map', 'tid_pos', 'unk', 'unk_char', 'unk_compat', 'unk_invoke', 'unk_map', 'unk_pos')) {
  $expectedFiles += "dict/$name.dat.gz"
}
$expectedFiles += 'dict/LICENSE-kuromoji.txt'
$expectedFiles += 'dict/NOTICE.md'

$actualFiles = Get-ChildItem -LiteralPath $distDir -Recurse -File |
  ForEach-Object { $_.FullName.Substring($distDir.Length).TrimStart('\', '/').Replace('\', '/') } |
  Sort-Object

foreach ($file in $expectedFiles) {
  if ($actualFiles -notcontains $file) { Add-ValidationError "The build did not produce: $file" }
}
foreach ($file in $actualFiles) {
  if ($expectedFiles -notcontains $file) {
    Add-ValidationError "Unexpected file in the build: $file. Nothing ships that is not listed in this script."
  }
}

# --- icons ---------------------------------------------------------------------------------------

foreach ($property in $manifest.icons.PSObject.Properties) {
  $size = 0
  if ([int]::TryParse($property.Name, [ref]$size)) {
    Test-PngIcon -FullPath (Join-Path $distDir ([string]$property.Value)) -Label ([string]$property.Value) -ExpectedSize $size
  }
}

# --- the HTML pages (MV3 CSP) ----------------------------------------------------------------------

foreach ($page in @('offscreen.html', 'options.html', 'permission.html')) {
  $pagePath = Join-Path $distDir $page
  if (-not (Test-Path -LiteralPath $pagePath -PathType Leaf)) { continue }
  $html = Get-Content -Raw -LiteralPath $pagePath
  if ($html -match '(?is)<script(?![^>]*\bsrc=)[^>]*>') {
    Add-ValidationError "$page contains an inline script, which MV3's CSP blocks."
  }
  if ($html -match '(?is)\son[a-z]+\s*=\s*["'']') {
    Add-ValidationError "$page contains an inline event handler, which MV3's CSP blocks."
  }
  if ($html -match '(?is)(?:src|href)\s*=\s*["'']https?://') {
    Add-ValidationError "$page loads a remote resource. Everything must ship inside the package."
  }
  foreach ($match in [regex]::Matches($html, '(?i)<script[^>]*\bsrc="([^"]+)"')) {
    $referenced = $match.Groups[1].Value
    if ($actualFiles -notcontains $referenced) { Add-ValidationError "$page points at a missing file: $referenced" }
  }
}

# --- the shipped JavaScript ---------------------------------------------------------------------

$distScripts = @($actualFiles | Where-Object { $_.EndsWith('.js') })

$node = Get-Command node -ErrorAction SilentlyContinue
if (-not $node) {
  if ($SkipNodeCheck) {
    Add-ValidationWarning 'Node.js was not found; the JavaScript syntax check was skipped (-SkipNodeCheck).'
  } else {
    Add-ValidationError 'Node.js was not found. Install it, or pass -SkipNodeCheck to skip the JavaScript syntax check.'
  }
} else {
  foreach ($script in $distScripts) {
    $output = & $node.Source --check (Join-Path $distDir $script) 2>&1
    if ($LASTEXITCODE -ne 0) { Add-ValidationError "JavaScript syntax check failed for $script`n$output" }
  }
}

foreach ($script in $distScripts) {
  $source = Get-Content -Raw -LiteralPath (Join-Path $distDir $script)

  if ($source -match '(?m)^\s*(import|export)\b') {
    Add-ValidationError "$script still contains module syntax; a classic script cannot run it."
  }

  # What the store submission promises about data has to be true of the shipped bytes. vtype-core
  # also carries a Whisper path that records audio and POSTs it to a transcription endpoint; the
  # extension does not use it and esbuild should drop it. If it ever lands in a bundle, the data
  # disclosure ("vtype itself sends nothing anywhere") stops being true, so this fails closed.
  foreach ($needle in @('createWhisperRecorder', 'encodeWavPcm16', 'audio/wav')) {
    if ($source.Contains($needle)) {
      Add-ValidationError "$script contains the Whisper path ('$needle'). It must not ship: it would send recorded audio to a server and contradict the store's data disclosure."
    }
  }
  # One fetch form is allowed: fetch(chrome.runtime.getURL(...)), which can only ever address a
  # file inside this extension's own package (kana mode reads its dictionary that way; see
  # src/offscreen/packaged-dictionary-loader.cjs). Any other fetch, and XHR at all, still fails.
  foreach ($pattern in @('\bfetch\s*\((?!\s*chrome\.runtime\.getURL\()', '\bXMLHttpRequest\b', '\bnew\s+WebSocket\b', '\bnavigator\.sendBeacon\b')) {
    if ($source -match $pattern) {
      Add-ValidationError "$script makes a network call (matched /$pattern/). vtype sends nothing of its own; if this is intended, the privacy policy and the store data disclosure have to be rewritten first."
    }
  }
}

# --- the documents the submission depends on -------------------------------------------------------

Test-RepoFile 'README.md' 'the store listing links to the repository'
Test-RepoFile 'LICENSE' 'public repository'
Test-RepoFile 'CHANGELOG.md' 'every release adds an entry'
Test-RepoFile 'PRIVACY.md' 'this file is what the store Privacy URL points at'
Test-RepoFile 'docs/store/listing.ja.md' 'what goes in the dashboard fields'
Test-RepoFile 'docs/store/listing.en.md' 'what goes in the dashboard fields'
Test-RepoFile 'docs/store/privacy-policy.ja.md' 'the source of PRIVACY.md'
Test-RepoFile 'docs/store/privacy-policy.en.md' 'the English privacy policy'
Test-RepoFile 'scripts/package-webstore.ps1' 'the packaging step'
Test-RepoFile 'scripts/secrets-scan.mjs' 'the layer 4 gate inside the packaging step'

$submissionNotesJa = Join-Path $root "docs/store/submission-notes-v$($manifest.version).ja.md"
$submissionNotesEn = Join-Path $root "docs/store/submission-notes-v$($manifest.version).en.md"
if (-not (Test-Path -LiteralPath $submissionNotesJa -PathType Leaf)) {
  Add-ValidationError "Missing: docs/store/submission-notes-v$($manifest.version).ja.md (one per version; do not overwrite the previous one)"
}
if (-not (Test-Path -LiteralPath $submissionNotesEn -PathType Leaf)) {
  Add-ValidationError "Missing: docs/store/submission-notes-v$($manifest.version).en.md"
}

$changelogPath = Join-Path $root 'CHANGELOG.md'
if (Test-Path -LiteralPath $changelogPath -PathType Leaf) {
  $changelog = Get-Content -Raw -LiteralPath $changelogPath
  if ($changelog -notmatch [regex]::Escape("## [$($manifest.version)]")) {
    Add-ValidationError "CHANGELOG.md has no '## [$($manifest.version)]' entry."
  }
}

# PRIVACY.md is a copy of the Japanese policy plus a trailing link to the English one, so the two
# cannot drift apart unnoticed (the store links to PRIVACY.md; the docs/store copy is the source).
$privacyRoot = Join-Path $root 'PRIVACY.md'
$privacyJa = Join-Path $root 'docs/store/privacy-policy.ja.md'
if ((Test-Path -LiteralPath $privacyRoot -PathType Leaf) -and (Test-Path -LiteralPath $privacyJa -PathType Leaf)) {
  $rootText = (Get-Content -Raw -LiteralPath $privacyRoot) -replace "`r`n", "`n"
  $jaText = ((Get-Content -Raw -LiteralPath $privacyJa) -replace "`r`n", "`n").Trim()
  $cut = $rootText.LastIndexOf("`n---`n")
  $rootBody = if ($cut -ge 0) { $rootText.Substring(0, $cut).Trim() } else { $rootText.Trim() }
  if ($rootBody -ne $jaText) {
    Add-ValidationError 'PRIVACY.md and docs/store/privacy-policy.ja.md have drifted apart. The docs/store copy is the source; PRIVACY.md is that text plus a trailing "---" section linking to the English version.'
  }
}

# --- the listing text the dashboard will not accept ---------------------------------------------

function Get-MarkdownSection {
  param([string]$Text, [string]$Heading)
  $pattern = '(?ms)^##\s+' + [regex]::Escape($Heading) + '\s*$(.*?)(?=^##\s|\z)'
  $match = [regex]::Match($Text, $pattern)
  if (-not $match.Success) { return $null }
  return $match.Groups[1].Value.Trim()
}

$listingChecks = @(
  @{ File = 'docs/store/listing.ja.md'; Name = '拡張機能名'; Short = '短い説明' },
  @{ File = 'docs/store/listing.en.md'; Name = 'Extension name'; Short = 'Short description' }
)
foreach ($check in $listingChecks) {
  $path = Join-Path $root $check.File
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
  $text = (Get-Content -Raw -LiteralPath $path) -replace "`r`n", "`n"

  $name = Get-MarkdownSection -Text $text -Heading $check.Name
  if (-not $name) {
    Add-ValidationWarning "$($check.File): no '## $($check.Name)' section, so the name could not be checked."
  } elseif ($name.Length -gt 75) {
    Add-ValidationError "$($check.File): the extension name is $($name.Length) characters; the store allows 75."
  }

  $short = Get-MarkdownSection -Text $text -Heading $check.Short
  if (-not $short) {
    Add-ValidationWarning "$($check.File): no '## $($check.Short)' section, so the short description could not be checked."
  } elseif ($short.Length -gt 132) {
    Add-ValidationError "$($check.File): the short description is $($short.Length) characters; the store allows 132."
  }

  # Other companies' product names in the listing are a rejection risk, and a screenshot of their
  # UI is a certain one. Naming them to say "it works there too" is the usual way this slips in.
  foreach ($brand in @('Twitter', 'Gmail', 'Slack', 'ChatGPT', 'Notion', 'Facebook', 'Instagram', 'YouTube')) {
    if ($text -match "(?i)\b$brand\b") {
      Add-ValidationWarning "$($check.File) names another company's product ('$brand'). Describe the kind of field instead (a webmail body, a chat box), and keep that product out of the screenshots."
    }
  }
}

# --- store listing localisation -------------------------------------------------------------------

# Adding a language is adding `_locales/<code>/messages.json`: everything below is derived from
# what is there, and the default locale is whatever the manifest declares.
if ($localeCodes.Count -eq 0) {
  Add-ValidationError 'packages/extension/_locales is missing: the manifest cannot resolve its __MSG_* fields and the store would show one language only.'
} else {
  $defaultLocale = [string]$manifest.default_locale
  if (-not $defaultLocale) {
    Add-ValidationError 'manifest.default_locale is required: Chrome refuses to load an extension that uses __MSG_* without it.'
  } elseif ($localeCodes -notcontains $defaultLocale) {
    Add-ValidationError "manifest.default_locale is '$defaultLocale', but _locales/$defaultLocale does not exist."
  } else {
    $keysByLocale = @{}
    foreach ($code in $localeCodes) {
      $path = Join-Path $localesDir "$code/messages.json"
      try {
        $parsed = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
      } catch {
        Add-ValidationError "_locales/$code/messages.json is not valid JSON: $($_.Exception.Message)"
        continue
      }
      $keysByLocale[$code] = @($parsed.PSObject.Properties.Name | Sort-Object)
      foreach ($property in $parsed.PSObject.Properties) {
        if (-not $property.Value.message -or [string]::IsNullOrWhiteSpace([string]$property.Value.message)) {
          Add-ValidationError "_locales/$code/messages.json: '$($property.Name)' has no message."
        }
      }
    }

    $defaultKeys = $keysByLocale[$defaultLocale]
    foreach ($code in $localeCodes) {
      if ($code -eq $defaultLocale -or -not $keysByLocale.ContainsKey($code)) { continue }
      $missing = @($defaultKeys | Where-Object { $keysByLocale[$code] -notcontains $_ })
      $extra = @($keysByLocale[$code] | Where-Object { $defaultKeys -notcontains $_ })
      if ($missing.Count -gt 0) { Add-ValidationError "_locales/$code is missing keys that $defaultLocale has: $($missing -join ', ')" }
      if ($extra.Count -gt 0) { Add-ValidationError "_locales/$code has keys $defaultLocale does not: $($extra -join ', ')" }
    }

    # A __MSG_ that resolves to nothing makes Chrome refuse the whole extension at load time.
    foreach ($field in @($manifest.name, $manifest.description, $manifest.action.default_title)) {
      if ($field -isnot [string]) { continue }
      $match = [regex]::Match($field, '^__MSG_(\w+)__$')
      if ($match.Success -and $defaultKeys -notcontains $match.Groups[1].Value) {
        Add-ValidationError "The manifest uses __MSG_$($match.Groups[1].Value)__, which _locales/$defaultLocale does not define."
      }
    }
  }
}

# --- report ---------------------------------------------------------------------------------------

foreach ($warning in $warnings) { Write-Warning $warning }

if ($errors.Count -gt 0) {
  Write-Host ''
  Write-Host "validate-extension: $($errors.Count) error(s)" -ForegroundColor Red
  foreach ($validationError in $errors) { Write-Host "  - $validationError" -ForegroundColor Red }
  exit 1
}

Write-Host "validate-extension: OK (v$($manifest.version), $($actualFiles.Count) files, $($warnings.Count) warning(s))" -ForegroundColor Green
