<#
.SYNOPSIS
  Builds CutOut.exe and the single-file Windows installer (CutOut-Windows-<x64|ARM64>-Setup.exe).

.DESCRIPTION
  1. cargo build --release for the chosen Windows target (unless -SkipBuild)
  2. stages the executable and the verified AI model under dist\win-<arch>
  3. runs Inno Setup (ISCC) on installer\CutOut.iss
  4. writes dist\SHA256SUMS.txt

  Requirements: Rust (MSVC toolchain), Inno Setup 6 (ISCC.exe on PATH or -Iscc).
  The model is verified against the pinned SHA-256 before it is packaged.
  Nothing is uploaded anywhere.

.EXAMPLE
  .\installer\build-installer.ps1 -Arch x64 -Model fp32
#>
param(
  [string]$Version = "",
  [ValidateSet("x64", "arm64")][string]$Arch = "x64",
  # fp32 = the exact published BiRefNet (973 MB); fp16 = the same weights at half precision (490 MB); none = no model bundled.
  [ValidateSet("fp32", "fp16", "none")][string]$Model = "fp32",
  # Folder that already contains the .onnx file(s); otherwise they are downloaded (pinned URL + SHA-256).
  [string]$ModelDir = "",
  [switch]$SkipBuild,
  [string]$Iscc = "ISCC.exe"
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

if (-not $Version) {
  $m = Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
  $Version = $m.Matches[0].Groups[1].Value
}
$triple = if ($Arch -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
$stage = Join-Path $root "dist\win-$Arch"

# Pinned model files (same revisions and hashes as app/src/models.rs and docs/MODELS.md).
$models = @{
  fp32 = @{ file = "birefnet-general-fp32.onnx"; bytes = 972666916; sha256 = "58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c";
            url = "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model.onnx" }
  fp16 = @{ file = "birefnet-general-fp16.onnx"; bytes = 489666272; sha256 = "3654c741eb80bd926ada8fed1713b506ccf8d30eb1f6487e87eb9f234f33df09";
            url = "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model_fp16.onnx" }
}

if (-not $SkipBuild) {
  Write-Host "== cargo build --release ($triple)"
  & cargo build --release --locked --target $triple -p cutout-desktop
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$exe = Join-Path $root "target\$triple\release\CutOut.exe"
# A plain host build (cargo build --release) puts an x64 exe in target\release.
if (-not (Test-Path $exe) -and $Arch -eq "x64") { $exe = Join-Path $root "target\release\CutOut.exe" }
if (-not (Test-Path $exe)) { throw "Missing $exe" }

Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item $exe (Join-Path $stage "CutOut.exe")

$modelFile = ""
if ($Model -ne "none") {
  $spec = $models[$Model]
  $modelFile = $spec.file
  if (-not $ModelDir) { $ModelDir = Join-Path $root "dist\model-cache" }
  New-Item -ItemType Directory -Force -Path $ModelDir | Out-Null
  $src = Join-Path $ModelDir $spec.file
  if (-not (Test-Path $src)) {
    Write-Host "== downloading $($spec.file) (pinned URL)"
    & curl.exe --fail --location --retry 3 --output $src $spec.url
    if ($LASTEXITCODE -ne 0) { throw "model download failed" }
  }
  Write-Host "== verifying $($spec.file)"
  $len = (Get-Item $src).Length
  if ($len -ne $spec.bytes) { throw "Model size mismatch: $len (expected $($spec.bytes))" }
  $hash = (Get-FileHash -Algorithm SHA256 $src).Hash.ToLower()
  if ($hash -ne $spec.sha256) { throw "Model SHA-256 mismatch: $hash" }
  New-Item -ItemType Directory -Force -Path (Join-Path $stage "Models") | Out-Null
  Copy-Item $src (Join-Path $stage "Models\$($spec.file)")
}

Write-Host "== Inno Setup"
$defs = @("/DAppVersion=$Version", "/DArch=$Arch", "/DSourceDir=$stage")
if ($modelFile) { $defs += "/DModelFile=$modelFile" }
if ($env:CUTOUT_SIGNTOOL) { $defs += "/DSignTool=$($env:CUTOUT_SIGNTOOL)" }
& $Iscc @defs (Join-Path $root "installer\CutOut.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

$archLabel = if ($Arch -eq "arm64") { "ARM64" } else { "x64" }
$setup = Join-Path $root "dist\CutOut-Windows-$archLabel-Setup.exe"
if (-not (Test-Path $setup)) { throw "Installer not produced: $setup" }
$h = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLower()
$sums = Join-Path $root "dist\SHA256SUMS.txt"
$line = "$h  $(Split-Path $setup -Leaf)"
if (Test-Path $sums) { (Get-Content $sums | Where-Object { $_ -notmatch [regex]::Escape((Split-Path $setup -Leaf)) }) + $line | Set-Content -Encoding ascii $sums } else { Set-Content -Encoding ascii $sums $line }
Write-Host "== $setup"
Write-Host "   SHA-256 $h"
