# Generate NSIS MUI bitmaps (24bpp BMP) in Serein colors.
# header 150x57, sidebar 164x314 -- Tauri/NSIS recommended sizes.
Add-Type -AssemblyName System.Drawing

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $root 'src-tauri\nsis'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$bg = [System.Drawing.Color]::FromArgb(0x1A, 0x1B, 0x26)
$bgAlt = [System.Drawing.Color]::FromArgb(0x16, 0x16, 0x21)
$accent = [System.Drawing.Color]::FromArgb(0x7A, 0xA2, 0xF7)
$text = [System.Drawing.Color]::FromArgb(0xC0, 0xCA, 0xF5)
$muted = [System.Drawing.Color]::FromArgb(0x56, 0x5F, 0x89)

$iconPath = Join-Path $root 'src-tauri\icons\128x128.png'
$icon = [System.Drawing.Image]::FromFile($iconPath)

function Save-Bmp24([System.Drawing.Bitmap]$bmp, [string]$path) {
  $clone = New-Object System.Drawing.Bitmap $bmp.Width, $bmp.Height, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [System.Drawing.Graphics]::FromImage($clone)
  $g.Clear($bg)
  $g.DrawImage($bmp, 0, 0, $bmp.Width, $bmp.Height)
  $g.Dispose()
  $clone.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
  $clone.Dispose()
}

function New-Canvas([int]$w, [int]$h) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit
  @{ Bmp = $bmp; G = $g }
}

# --- sidebar: welcome/finish left panel ---
$s = New-Canvas 164 314
$g = $s.G
$g.Clear($bg)
$g.FillRectangle((New-Object System.Drawing.SolidBrush $bgAlt), 0, 0, 164, 48)
$g.FillRectangle((New-Object System.Drawing.SolidBrush $accent), 0, 0, 4, 314)
$iconSize = 96
$ix = [int]((164 - $iconSize) / 2)
$g.DrawImage($icon, $ix, 48, $iconSize, $iconSize)

$titleFont = New-Object System.Drawing.Font 'Segoe UI Semibold', 16, ([System.Drawing.FontStyle]::Bold)
$subFont = New-Object System.Drawing.Font 'Segoe UI', 9
$brush = New-Object System.Drawing.SolidBrush $text
$mutedBrush = New-Object System.Drawing.SolidBrush $muted
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$g.DrawString('Serein', $titleFont, $brush, (New-Object System.Drawing.RectangleF 8, 160, 148, 32), $sf)
$g.DrawString('SSH / SFTP', $subFont, $mutedBrush, (New-Object System.Drawing.RectangleF 8, 192, 148, 24), $sf)
Save-Bmp24 $s.Bmp (Join-Path $outDir 'sidebar.bmp')
$g.Dispose(); $s.Bmp.Dispose()

# --- header: top-right of inner pages ---
$h = New-Canvas 150 57
$g = $h.G
$g.Clear($bg)
$g.FillRectangle((New-Object System.Drawing.SolidBrush $accent), 0, 0, 150, 3)
$g.DrawImage($icon, 10, 10, 36, 36)
$hdrFont = New-Object System.Drawing.Font 'Segoe UI Semibold', 12, ([System.Drawing.FontStyle]::Bold)
$g.DrawString('Serein', $hdrFont, $brush, 52, 18)
Save-Bmp24 $h.Bmp (Join-Path $outDir 'header.bmp')
$g.Dispose(); $h.Bmp.Dispose()

$icon.Dispose()
Write-Host "Wrote $outDir\header.bmp and $outDir\sidebar.bmp"
