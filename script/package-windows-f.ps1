[CmdletBinding()]
param(
  [Parameter()][string]$Architecture = "x86_64",
  [Parameter()][string]$Root = "F:\cunning3d",
  [Parameter()][switch]$BuildKnowledgePack,
  [Parameter()][switch]$UseDefaultFeatures,
  [Parameter()][string]$Features = "voice",
  [Parameter()][switch]$Install,
  [Parameter()][string]$VsDevShellPath,
  [Parameter()][string]$InnoSetupPath
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

switch ($Architecture) {
  "x86_64" { $Architecture = "x86_64" }
  "aarch64" { $Architecture = "aarch64" }
  "x64" { $Architecture = "x86_64" }
  "arm64" { $Architecture = "aarch64" }
  default { throw "Unsupported -Architecture=$Architecture (use x86_64/aarch64)" }
}

if (-not (Test-Path (Split-Path -Parent $Root))) { throw "Drive not found for Root=$Root" }
New-Item -ItemType Directory -Force -Path $Root | Out-Null

$out = Join-Path $Root "target"
$cargo = Join-Path $Root "cargo-target"
New-Item -ItemType Directory -Force -Path $out | Out-Null
New-Item -ItemType Directory -Force -Path $cargo | Out-Null

$bundle = Join-Path $PSScriptRoot "bundle-windows.ps1"
$common = @{
  OutputDir      = $out
  CargoTargetDir = $cargo
  Features       = $Features
}
if ($VsDevShellPath) { $common.VsDevShellPath = $VsDevShellPath }
if ($InnoSetupPath) { $common.InnoSetupPath = $InnoSetupPath }

Write-Host "Packaging to:"
Write-Host "  OutputDir       = $out"
Write-Host "  CargoTargetDir  = $cargo"
Write-Host "  Arch            = $Architecture"
Write-Host ""

if ($Architecture -eq "aarch64") {
  & $bundle -Architecture aarch64 @common -BuildKnowledgePack:$BuildKnowledgePack -UseDefaultFeatures:$UseDefaultFeatures -Install:$Install
} else {
  & $bundle @common -BuildKnowledgePack:$BuildKnowledgePack -UseDefaultFeatures:$UseDefaultFeatures -Install:$Install
}

