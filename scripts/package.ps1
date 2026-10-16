# Builds a portable folder: dist\AI Video Editor\ with the app and its bundled FFmpeg.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\package.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$out = Join-Path $root "dist\AI Video Editor"
$running = Get-Process ai-video-editor -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$out*" }
if ($running) { throw "AI Video Editor is running from $out - close it first (save your project), then run this again." }
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force (Join-Path $out "ffmpeg") | Out-Null

Copy-Item "target\release\ai-video-editor.exe" $out
foreach ($f in "ffmpeg.exe", "ffprobe.exe", "LICENSE.txt") {
    $src = Join-Path "vendor\ffmpeg" $f
    if (-not (Test-Path $src)) { throw "Missing $src - see README (Setup)" }
    Copy-Item $src (Join-Path $out "ffmpeg")
}
Write-Output "Packaged to $out"
