# Turns plain screen captures into Chrome Web Store screenshots: 1280x800, 24-bit PNG (no alpha).
#
# A capture of packages/extension/testbed/store-demo.html is cut along the inside of the page's
# magenta frame, so the capture can include the whole browser window and any display scaling
# (125 %, 150 %) comes out the same. A capture without the frame (the settings page) is fitted
# inside 1280x800 instead, padded with the colour of its top-left pixel.
#
#   pwsh -NoProfile -File scripts/store-screenshot.ps1 <capture.png> [<capture.png> ...]
#
# Writes dist/store-screenshots/<name>-1280x800.png (gitignored). Warns when the stage was
# captured smaller than 1280x800, because the result is then enlarged and looks soft.

param(
  [Parameter(Mandatory, Position = 0, ValueFromRemainingArguments)]
  [string[]]$Path,
  [string]$OutDir
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $root 'dist/store-screenshots' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$W = 1280
$H = 800

function Test-Magenta([byte[]]$px, [int]$i) {
  # BGRA order. Pure #ff00ff, with room for the colour management of a screen capture.
  return ($px[$i + 2] -ge 235 -and $px[$i + 1] -le 30 -and $px[$i] -ge 235)
}

# Frame or the blend of frame and stage that scaling leaves on the boundary. The stage's edges are
# a pale background whose green is never far below its red and blue, so this cannot eat into it.
function Test-FrameEdge([byte[]]$px, [int]$i) {
  $g = [int]$px[$i + 1]
  return (([int]$px[$i + 2] - $g) -gt 60 -and ([int]$px[$i] - $g) -gt 60)
}

# Returns the rectangle inside the magenta frame, or $null when the capture has no frame.
function Find-Stage([System.Drawing.Bitmap]$bmp) {
  $rect = [System.Drawing.Rectangle]::new(0, 0, $bmp.Width, $bmp.Height)
  $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  try {
    $stride = $data.Stride
    $px = [byte[]]::new($stride * $bmp.Height)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $px, 0, $px.Length)
  } finally {
    $bmp.UnlockBits($data)
  }

  # Outer bounds of the frame, from every 4th row.
  $minX = [int]::MaxValue; $maxX = -1; $minY = [int]::MaxValue; $maxY = -1
  for ($y = 0; $y -lt $bmp.Height; $y += 4) {
    $row = $y * $stride
    for ($x = 0; $x -lt $bmp.Width; $x++) {
      if (Test-Magenta $px ($row + $x * 4)) {
        if ($x -lt $minX) { $minX = $x }
        if ($x -gt $maxX) { $maxX = $x }
        if ($y -lt $minY) { $minY = $y }
        if ($y -gt $maxY) { $maxY = $y }
      }
    }
  }
  if ($maxX -lt 0 -or ($maxX - $minX) -lt 200 -or ($maxY - $minY) -lt 120) { return $null }

  # The sampled rows can miss the frame's top and bottom edge by up to 3 pixels.
  $midX = [int](($minX + $maxX) / 2)
  while ($minY -gt 0 -and (Test-Magenta $px (($minY - 1) * $stride + $midX * 4))) { $minY-- }
  while ($maxY -lt $bmp.Height - 1 -and (Test-Magenta $px (($maxY + 1) * $stride + $midX * 4))) { $maxY++ }

  # Walk inwards from each outer edge, through the frame's thickness, at the middle of the side.
  $midY = [int](($minY + $maxY) / 2)
  $l = $minX; while ($l -lt $maxX -and (Test-FrameEdge $px ($midY * $stride + $l * 4))) { $l++ }
  $r = $maxX; while ($r -gt $minX -and (Test-FrameEdge $px ($midY * $stride + $r * 4))) { $r-- }
  $t = $minY; while ($t -lt $maxY -and (Test-FrameEdge $px ($t * $stride + $midX * 4))) { $t++ }
  $b = $maxY; while ($b -gt $minY -and (Test-FrameEdge $px ($b * $stride + $midX * 4))) { $b-- }
  return [System.Drawing.Rectangle]::new($l, $t, ($r - $l) + 1, ($b - $t) + 1)
}

foreach ($p in $Path) {
  $src = [System.Drawing.Bitmap]::new((Resolve-Path -LiteralPath $p).Path)
  $out = [System.Drawing.Bitmap]::new($W, $H, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [System.Drawing.Graphics]::FromImage($out)
  try {
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.Clear([System.Drawing.Color]::White)

    $stage = Find-Stage $src
    if ($null -ne $stage) {
      $ratio = $stage.Width / $stage.Height
      if ([math]::Abs($ratio - ($W / $H)) -gt 0.02) {
        Write-Warning ("{0}: the framed area is {1}x{2}, not 16:10. Was part of the frame outside the capture?" -f $p, $stage.Width, $stage.Height)
      }
      # Scaling straight from the capture would sample the frame just outside the stage and tint
      # the edge pink, so the stage is cut out first and its own edge pixels are mirrored instead.
      $cut = $src.Clone($stage, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
      $attr = [System.Drawing.Imaging.ImageAttributes]::new()
      $attr.SetWrapMode([System.Drawing.Drawing2D.WrapMode]::TileFlipXY)
      $g.DrawImage($cut, [System.Drawing.Rectangle]::new(0, 0, $W, $H), 0, 0, $cut.Width, $cut.Height, [System.Drawing.GraphicsUnit]::Pixel, $attr)
      $attr.Dispose()
      $cut.Dispose()
      $how = "stage {0}x{1} at ({2},{3})" -f $stage.Width, $stage.Height, $stage.X, $stage.Y
      $small = $stage.Width -lt $W
    } else {
      # Pad with the capture's own background (its top-left pixel), so a dark settings page does
      # not float on white.
      $g.Clear($src.GetPixel(0, 0))
      $scale = [math]::Min($W / $src.Width, $H / $src.Height)
      $dw = [int][math]::Round($src.Width * $scale)
      $dh = [int][math]::Round($src.Height * $scale)
      $g.DrawImage($src, [System.Drawing.Rectangle]::new([int](($W - $dw) / 2), [int](($H - $dh) / 2), $dw, $dh))
      $how = "no frame: fitted {0}x{1} onto its own background colour" -f $src.Width, $src.Height
      $small = $scale -gt 1
    }
  } finally {
    $g.Dispose()
    $src.Dispose()
  }

  $name = [System.IO.Path]::GetFileNameWithoutExtension($p) + '-1280x800.png'
  $dest = Join-Path $OutDir $name
  $out.Save($dest, [System.Drawing.Imaging.ImageFormat]::Png)
  $out.Dispose()
  Write-Output ("{0}  <-  {1} ({2})" -f $dest, $p, $how)
  if ($small) { Write-Warning "$p was captured smaller than 1280x800 and has been enlarged; it may look soft. Capture it larger if you can." }
}
