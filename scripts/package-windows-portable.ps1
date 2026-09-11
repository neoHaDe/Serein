[CmdletBinding()]
param(
    [string]$OutputDirectory = '',
    [switch]$KeepStaging
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$package = Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw | ConvertFrom-Json
$version = [string]$package.version
if (-not $version) { throw 'package.json has no version' }
$helperManifest = Get-Content -LiteralPath (Join-Path $root 'rdp-helper\Cargo.toml')
$helperVersionLine = $helperManifest | Where-Object { $_ -match '^version\s*=\s*"([^"]+)"' } | Select-Object -First 1
if ($null -eq $helperVersionLine) { throw 'rdp-helper/Cargo.toml has no package version' }
$helperVersion = [regex]::Match($helperVersionLine, '"([^"]+)"').Groups[1].Value
if ($helperVersion -ne $version) { throw "Version mismatch: app $version, RDP helper $helperVersion" }

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $root 'src-tauri\target\release\bundle\portable'
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

$app = Join-Path $root 'src-tauri\target\release\serein.exe'
$helper = Join-Path $root 'src-tauri\binaries\serein-rdp-x86_64-pc-windows-msvc.exe'
$license = Join-Path $root 'LICENSE'
$notice = Join-Path $root 'NOTICE'
foreach ($file in @($app, $helper, $license, $notice)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing release input: $file" }
    if ((Get-Item -LiteralPath $file).Length -eq 0) { throw "Empty release input: $file" }
}

# A copied executable from an older build can look perfectly valid and still omit the
# current fixes. Fail before packaging whenever source inputs are newer than the binaries.
$appInputs = @(
    Get-Item -LiteralPath (Join-Path $root 'package.json'), (Join-Path $root 'package-lock.json'), (Join-Path $root 'src-tauri\Cargo.toml')
    Get-ChildItem -LiteralPath (Join-Path $root 'src'), (Join-Path $root 'src-tauri\src') -File -Recurse
)
$newestAppInput = $appInputs | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if ((Get-Item -LiteralPath $app).LastWriteTimeUtc -lt $newestAppInput.LastWriteTimeUtc) {
    throw "serein.exe is older than $($newestAppInput.FullName); run the release build first"
}
$helperInputs = @(
    Get-Item -LiteralPath (Join-Path $root 'rdp-helper\Cargo.toml'), (Join-Path $root 'rdp-helper\Cargo.lock')
    Get-ChildItem -LiteralPath (Join-Path $root 'rdp-helper\src') -File -Recurse
)
$newestHelperInput = $helperInputs | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if ((Get-Item -LiteralPath $helper).LastWriteTimeUtc -lt $newestHelperInput.LastWriteTimeUtc) {
    throw "serein-rdp.exe is older than $($newestHelperInput.FullName); build the helper first"
}

$stage = Join-Path $OutputDirectory ('.stage-' + [guid]::NewGuid().ToString('N'))
$archivePath = Join-Path $OutputDirectory "Serein_${version}_x64-portable.zip"
New-Item -ItemType Directory -Path $stage | Out-Null

try {
    Copy-Item -LiteralPath $app -Destination (Join-Path $stage 'Serein.exe')
    Copy-Item -LiteralPath $helper -Destination (Join-Path $stage 'serein-rdp.exe')
    Copy-Item -LiteralPath $license -Destination (Join-Path $stage 'LICENSE')
    Copy-Item -LiteralPath $notice -Destination (Join-Path $stage 'NOTICE')

    if (Test-Path -LiteralPath $archivePath) { Remove-Item -LiteralPath $archivePath -Force }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archivePath -CompressionLevel Optimal

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        $expected = @('Serein.exe', 'serein-rdp.exe', 'LICENSE', 'NOTICE')
        $entries = @($archive.Entries | Where-Object { -not $_.FullName.EndsWith('/') })
        foreach ($name in $expected) {
            $entry = $entries | Where-Object FullName -eq $name
            if ($null -eq $entry) { throw "Portable archive misses $name" }
            if ($entry.Length -eq 0) { throw "Portable archive contains empty $name" }
        }
        $unexpected = @($entries | Where-Object { $expected -notcontains $_.FullName })
        if ($unexpected.Count -gt 0) {
            throw ('Portable archive has unexpected entries: ' + (($unexpected | ForEach-Object FullName) -join ', '))
        }
    } finally {
        $archive.Dispose()
    }

    $hash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
    [pscustomobject]@{
        Path = $archivePath
        Bytes = (Get-Item -LiteralPath $archivePath).Length
        SHA256 = $hash.Hash.ToLowerInvariant()
        Files = 4
    } | Format-List
} finally {
    if (-not $KeepStaging -and (Test-Path -LiteralPath $stage)) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
}
