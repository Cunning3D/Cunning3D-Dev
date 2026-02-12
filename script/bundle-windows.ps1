[CmdletBinding()]
param(
  [Parameter()][Alias('a')][ValidateSet('x86_64','aarch64')][string]$Architecture,
  [Parameter()][Alias('i')][switch]$Install,
  [Parameter()][switch]$BuildKnowledgePack,
  [Parameter()][switch]$UseDefaultFeatures,
  [Parameter()][string]$Features = "voice",
  [Parameter()][string]$OutputDir,
  [Parameter()][string]$CargoTargetDir,
  [Parameter()][string]$VsDevShellPath,
  [Parameter()][string]$InnoSetupPath
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

function Get-OSArch { switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) { "X64" { "x86_64" } "Arm64" { "aarch64" } default { throw "Unsupported OS architecture." } } }
function Get-VSArch([string]$Arch) { switch ($Arch) { "x86_64" { "amd64" } "aarch64" { "arm64" } default { throw "Unsupported build architecture: $Arch" } } }
function Find-VsDevShell([string]$ProvidedPath) {
  if ($ProvidedPath -and (Test-Path $ProvidedPath)) { return $ProvidedPath }
  $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  if (Test-Path $vswhere) {
    $installPath = & $vswhere -latest -property installationPath
    if ($installPath) {
      $candidate = Join-Path $installPath "Common7\Tools\Launch-VsDevShell.ps1"
      if (Test-Path $candidate) { return $candidate }
    }
  }
  $fallback = "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\Launch-VsDevShell.ps1"
  if (Test-Path $fallback) { return $fallback }
  throw "Unable to locate Launch-VsDevShell.ps1. Install Visual Studio 2022, or pass -VsDevShellPath."
}
function Find-InnoSetupIscc([string]$ProvidedPath) {
  if ($ProvidedPath -and (Test-Path $ProvidedPath)) { return $ProvidedPath }
  $cmd = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  $userInstall = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
  if (Test-Path $userInstall) { return $userInstall }
  $fallback = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
  if (Test-Path $fallback) { return $fallback }
  throw "ISCC.exe not found. Install Inno Setup 6, or pass -InnoSetupPath."
}
function Find-MakeAppx {
  $cmd = Get-Command "makeAppx.exe" -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  $roots = @(
    "C:\Program Files (x86)\Windows Kits\10\bin",
    "C:\Program Files (x86)\Windows Kits\11\bin"
  )
  foreach ($r in $roots) {
    if (-not (Test-Path $r)) { continue }
    $cand = Get-ChildItem -Path $r -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending
    foreach ($d in $cand) {
      $p = Join-Path $d.FullName "x64\makeAppx.exe"
      if (Test-Path $p) { return $p }
    }
  }
  throw "makeAppx.exe not found. Install Windows SDK (MakeAppx), or ensure it is on PATH."
}
function Get-CargoVersion([string]$CargoTomlPath) {
  $t = Get-Content -Raw $CargoTomlPath
  $m = [regex]::Match($t, "(?ms)^\[package\]\s+.*?^version\s*=\s*`"([^`"]+)`"")
  if (-not $m.Success) { throw "Unable to parse version from Cargo.toml." }
  return $m.Groups[1].Value
}
function Ensure-EmptyDir([string]$Path) { if (Test-Path $Path) { Remove-Item -Recurse -Force $Path }; New-Item -ItemType Directory -Force -Path $Path | Out-Null }
function Robocopy-Mirror([string]$Src, [string]$Dst) {
  if (-not (Test-Path $Src)) { return }
  New-Item -ItemType Directory -Force -Path $Dst | Out-Null
  $null = & robocopy $Src $Dst /MIR /NFL /NDL /NJH /NJS /NC /NS /NP
  if ($LASTEXITCODE -ge 8) { throw "robocopy failed (exit=$LASTEXITCODE): $Src -> $Dst" }
}
function Resize-PngSquare([string]$Src, [string]$Dst, [int]$Size) {
  Add-Type -AssemblyName System.Drawing
  $img = [System.Drawing.Image]::FromFile($Src)
  $bmp = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.Clear([System.Drawing.Color]::Transparent)
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $scale = [Math]::Min($Size / $img.Width, $Size / $img.Height)
  $w = [int]([Math]::Round($img.Width * $scale))
  $h = [int]([Math]::Round($img.Height * $scale))
  $x = [int](($Size - $w) / 2)
  $y = [int](($Size - $h) / 2)
  $g.DrawImage($img, $x, $y, $w, $h)
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Dst) | Out-Null
  $bmp.Save($Dst, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose(); $img.Dispose()
}
function Invoke-Native {
  param([Parameter(Mandatory=$true)][string]$Exe, [Parameter(ValueFromRemainingArguments=$true)][string[]]$Args)
  & $Exe @Args
  if ($LASTEXITCODE -ne 0) { throw "$Exe failed (exit=$LASTEXITCODE): $Args" }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$OSArch = Get-OSArch
$Architecture = if ($Architecture) { $Architecture } else { $OSArch }
$target = "$Architecture-pc-windows-msvc"
$cargoToml = Join-Path $repoRoot "Cargo.toml"
$version = Get-CargoVersion $cargoToml
$outDir = if ($OutputDir) { $OutputDir } else { (Join-Path $repoRoot "target") }
$outDir = (Resolve-Path (New-Item -ItemType Directory -Force -Path $outDir)).Path

$cargoTargetDir = if ($CargoTargetDir) { $CargoTargetDir } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { (Join-Path $repoRoot ".cargo-target") }
$env:CARGO_TARGET_DIR = $cargoTargetDir
$cargoOutDir = Join-Path $cargoTargetDir "$target\release"

if ($BuildKnowledgePack) {
  if (-not $env:CUNNING_KNOWLEDGE_KEY) { throw "Missing env var: CUNNING_KNOWLEDGE_KEY (required for -BuildKnowledgePack)." }
  $env:CUNNING_BUILD_KNOWLEDGE_PACK = "1"
}

$vsDevShell = Find-VsDevShell -ProvidedPath $VsDevShellPath
Push-Location
& $vsDevShell -Arch (Get-VSArch $Architecture) -HostArch (Get-VSArch $OSArch) | Out-Null
Pop-Location

Push-Location -Path $repoRoot
$cargoArgs = @("build", "--release", "--target", $target, "--package", "cunning3d")
if (-not $UseDefaultFeatures) { $cargoArgs += @("--no-default-features", "--features", $Features) }
Write-Host "Building Cunning3D ($target)..."
Invoke-Native cargo @cargoArgs

Write-Host "Building explorer command injector ($target)..."
$injectorManifest = Join-Path $repoRoot "crates\cunning_explorer_command_injector\Cargo.toml"
Invoke-Native cargo build --release --target $target --manifest-path $injectorManifest --no-default-features --features stable

$staging = Join-Path $outDir "inno\$Architecture"
Ensure-EmptyDir $staging
Ensure-EmptyDir (Join-Path $staging "appx")
Ensure-EmptyDir (Join-Path $staging "make_appx")

Robocopy-Mirror (Join-Path $repoRoot "assets") (Join-Path $staging "assets")
Robocopy-Mirror (Join-Path $repoRoot "Ltools") (Join-Path $staging "Ltools")

$exeSrc = Join-Path $cargoOutDir "cunning3d.exe"
if (-not (Test-Path $exeSrc)) { $exeSrc = Join-Path $cargoOutDir "Cunning3D.exe" }
if (-not (Test-Path $exeSrc)) { throw "Cunning3D executable not found in $cargoOutDir." }
Copy-Item -Force $exeSrc (Join-Path $staging "Cunning3D.exe")
Copy-Item -Force $exeSrc (Join-Path $staging "appx\Cunning3D.exe")

$dllSrc = Join-Path $cargoOutDir "cunning_explorer_command_injector.dll"
if (-not (Test-Path $dllSrc)) { throw "Explorer injector DLL not found: $dllSrc" }
$dllDstName = "cunning3d_explorer_command_injector.dll"
Copy-Item -Force $dllSrc (Join-Path $staging "appx\$dllDstName")
Copy-Item -Force $dllSrc (Join-Path $staging "make_appx\$dllDstName")

$appxManifestSrc = Join-Path $repoRoot "resources\windows\appx\AppxManifest.xml"
Copy-Item -Force $appxManifestSrc (Join-Path $staging "make_appx\AppxManifest.xml")
Copy-Item -Force (Join-Path $staging "appx\Cunning3D.exe") (Join-Path $staging "make_appx\Cunning3D.exe")

$logoSrc = Join-Path $repoRoot "..\Cunning3d_website\public\logo.png"
if (-not (Test-Path $logoSrc)) { throw "Logo source not found for appx resources: $logoSrc" }
Resize-PngSquare $logoSrc (Join-Path $staging "make_appx\resources\logo_150x150.png") 150
Resize-PngSquare $logoSrc (Join-Path $staging "make_appx\resources\logo_70x70.png") 70

$makeAppx = Find-MakeAppx
Write-Host "Packing appx..."
Invoke-Native $makeAppx pack /d (Join-Path $staging "make_appx") /p (Join-Path $staging "appx\cunning3d_explorer_command_injector.appx") /nv

$iscc = Find-InnoSetupIscc -ProvidedPath $InnoSetupPath
$iss = Join-Path $repoRoot "resources\windows\installer\cunning3d.iss"
$baseName = "Cunning3D-$Architecture"
$defs = @(
  "/dVersion=`"$version`"",
  "/dOutputDir=`"$outDir`"",
  "/dOutputBaseFilename=`"$baseName`"",
  "/dResourcesDir=`"$staging`""
)
if ($env:CI) {
  $signPs1 = Join-Path $repoRoot "resources\windows\installer\sign.ps1"
  $signTool = "powershell.exe -ExecutionPolicy Bypass -File `"$signPs1`" `$f"
  $defs += "/sDefaultsign=`"$signTool`""
}

Write-Host "Running Inno Setup..."
$p = Start-Process -FilePath $iscc -ArgumentList (@($iss) + $defs) -NoNewWindow -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "Inno Setup failed with exit code $($p.ExitCode)." }

$setupPath = Join-Path $outDir ("$baseName.exe")
Write-Host "Built installer: $setupPath"
if ($Install) { Start-Process -FilePath $setupPath }
Pop-Location

