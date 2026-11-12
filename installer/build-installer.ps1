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
  .\installer\build-installer.ps1 -Arch x64            # bundles the v2 model package from dist\model-cache
  .\installer\build-installer.ps1 -Arch x64 -Model fp32   # the downloadable onnx-community export instead
#>
param(
  [string]$Version = "",
  [ValidateSet("x64", "arm64")][string]$Arch = "x64",
  # v2   = the compact package built by tools\convert\export_onnx.py (588 MB: shared fp16 weights, GPU graph, CPU graph).
  #        It is not downloadable: build it once into -ModelDir (see tools\convert\README.md).
  # fp32 = the onnx-community export (973 MB, downloaded at a pinned URL); fp16 = the same at half precision (490 MB);
  # none = no model bundled.
  [ValidateSet("v2", "fp32", "fp16", "none")][string]$Model = "v2",
  # Folder that already contains the model file(s); fp32/fp16 are downloaded into it when missing (pinned URL + SHA-256).
  [string]$ModelDir = "",
  # With -Model fp32: also bundle the fp16 file that the GPU (DirectML) path of the original export uses.
  [switch]$WithGpuModel,
  [switch]$SkipBuild,
  [string]$Iscc = "ISCC.exe"
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

if (-not $Version) {
  $m = Select-String -Path (Join-Path $root "app\Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
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

# Do not embed the build machine's user name / paths (panic locations) in the shipped binary.
$env:RUSTFLAGS = "--remap-path-prefix=$($env:USERPROFILE)=~ --remap-path-prefix=$root=src"
if (-not $SkipBuild) {
  Write-Host "== cargo build --release ($triple)"
  $ErrorActionPreference = "Continue"; & cargo build --release --locked --target $triple -p cutout-desktop; $ErrorActionPreference = "Stop"
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$exe = Join-Path $root "target\$triple\release\CutOut.exe"
# A plain host build (cargo build --release) puts an x64 exe in target\release.
if (-not (Test-Path $exe) -and $Arch -eq "x64") { $exe = Join-Path $root "target\release\CutOut.exe" }
if (-not (Test-Path $exe)) { throw "Missing $exe" }

Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item $exe (Join-Path $stage "CutOut.exe")

# CutOut.exe (via the ONNX Runtime static library) imports the Visual C++ 2015-2022 runtime. A clean PC may not
# have the Microsoft redistributable, so ship the four DLLs app-local (permitted redistributable files). They are
# copied from the build machine's System32 and must carry a valid Microsoft Authenticode signature.
if ($Arch -eq "x64") {
  foreach ($n in "vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "msvcp140_1.dll") {
    $src = Join-Path $env:SystemRoot "System32\$n"
    if (-not (Test-Path $src)) { throw "Missing $src - install the Microsoft Visual C++ Redistributable (x64) on the build machine" }
    $sig = Get-AuthenticodeSignature $src
    if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") { throw "$n is not a validly signed Microsoft file" }
    Copy-Item $src (Join-Path $stage $n)
  }
} else {
  Write-Warning "ARM64: bundle the ARM64 Visual C++ runtime DLLs into $stage manually (not done by this script)."
}

# The v2 package: three files pinned in tools\convert\OUTPUT-onnx.txt (and in app\src\models.rs).
$v2 = @(
  @{ file = "birefnet-general-v2-cpu.onnx"; bytes = 82962350;  sha256 = "813d040fd23b5d1449b453110d22aa99c56d7e8ba87748b9e7ece9a86fe86274" },
  @{ file = "birefnet-general-v2-gpu.onnx"; bytes = 42173987;  sha256 = "3f755e07a36f20600f8fe6b776ddb25f7dc097caac3101ea48688d29d6f6e96e" },
  @{ file = "birefnet-general-v2.weights";  bytes = 462621184; sha256 = "8fd62d7e2a1792a64ef6ce6d8365716ff95c4f84d8a42fb812f471c24937ef53" }
)
$modelFile = ""
if ($Model -eq "v2") {
  if (-not $ModelDir) { $ModelDir = Join-Path $root "dist\model-cache" }
  New-Item -ItemType Directory -Force -Path (Join-Path $stage "Models") | Out-Null
  foreach ($f in $v2) {
    $src = Join-Path $ModelDir $f.file
    if (-not (Test-Path $src)) { throw "Missing $src - build the package with tools\convert\export_onnx.py (see tools\convert\README.md), or use -Model fp32" }
    Write-Host "== verifying $($f.file)"
    if ((Get-Item $src).Length -ne $f.bytes) { throw "$($f.file): unexpected size" }
    if ((Get-FileHash -Algorithm SHA256 $src).Hash.ToLower() -ne $f.sha256) { throw "$($f.file): SHA-256 mismatch" }
    Copy-Item $src (Join-Path $stage "Models\$($f.file)")
  }
  $modelFile = "v2"
} elseif ($Model -ne "none") {
  $spec = $models[$Model]
  $modelFile = $spec.file
  if (-not $ModelDir) { $ModelDir = Join-Path $root "dist\model-cache" }
  New-Item -ItemType Directory -Force -Path $ModelDir | Out-Null
  $src = Join-Path $ModelDir $spec.file
  if (-not (Test-Path $src)) {
    Write-Host "== downloading $($spec.file) (pinned URL)"
    $ErrorActionPreference = "Continue"; & curl.exe --fail --silent --show-error --location --retry 3 --output $src $spec.url; $ErrorActionPreference = "Stop"
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

$gpuFile = ""
if ($WithGpuModel -and $Model -eq "fp32") {
  $g = $models["fp16"]; $gsrc = Join-Path $ModelDir $g.file
  if (-not (Test-Path $gsrc)) { $ErrorActionPreference = "Continue"; & curl.exe --fail --silent --show-error --location --retry 3 --output $gsrc $g.url; $ErrorActionPreference = "Stop"; if ($LASTEXITCODE -ne 0) { throw "fp16 model download failed" } }
  if ((Get-Item $gsrc).Length -ne $g.bytes -or (Get-FileHash -Algorithm SHA256 $gsrc).Hash.ToLower() -ne $g.sha256) { throw "fp16 model verification failed" }
  Copy-Item $gsrc (Join-Path $stage "Models\$($g.file)"); $gpuFile = $g.file
}
Write-Host "== Inno Setup"
# "0.1.0-preview.2" -> file version 0.1.0.2; "0.1.0" -> 0.1.0.0
$numeric = if ($Version -match '^(\d+\.\d+\.\d+)(?:-[A-Za-z]+\.(\d+))?$') { "$($Matches[1]).$(if ($Matches[2]) { $Matches[2] } else { '0' })" } else { throw "Unsupported version format: $Version" }
$defs = @("/DAppVersion=$Version", "/DVersionNumeric=$numeric", "/DArch=$Arch", "/DSourceDir=$stage")
if ($modelFile) { $defs += "/DBundleModels" }
if ($env:CUTOUT_SIGNTOOL) { $defs += "/DSignTool=$($env:CUTOUT_SIGNTOOL)" }
$ErrorActionPreference = "Continue"; & $Iscc @defs (Join-Path $root "installer\CutOut.iss"); $ErrorActionPreference = "Stop"
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
