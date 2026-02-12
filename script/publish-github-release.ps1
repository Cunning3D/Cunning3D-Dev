[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$Repo, # "owner/name"
  [Parameter()][string]$Tag = "v0.10.0",
  [Parameter()][string]$ReleaseName = "Cunning3D v0.10.0 (Windows x86_64)",
  [Parameter()][string]$AssetPath = "F:\cunning3d\target\Cunning3D-x86_64.exe",
  [Parameter()][switch]$Prerelease,
  [Parameter()][switch]$Draft
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Require-Env([string]$Name) {
  if (-not (Test-Path "env:$Name") -or [string]::IsNullOrWhiteSpace((Get-Item "env:$Name").Value)) {
    throw "Missing env var: $Name"
  }
  (Get-Item "env:$Name").Value
}

function Api([string]$Method, [string]$Url, $Body = $null, [string]$Accept = "application/vnd.github+json") {
  $token = Require-Env "GITHUB_TOKEN"
  $headers = @{
    "Authorization" = "Bearer $token"
    "Accept"        = $Accept
    "X-GitHub-Api-Version" = "2022-11-28"
    "User-Agent"    = "Cunning3D-ReleaseScript"
  }
  if ($null -ne $Body) {
    return Invoke-RestMethod -Method $Method -Uri $Url -Headers $headers -ContentType "application/json" -Body ($Body | ConvertTo-Json -Depth 12)
  }
  return Invoke-RestMethod -Method $Method -Uri $Url -Headers $headers
}

function Upload-Asset([string]$UploadUrlTemplate, [string]$Path, [string]$Name) {
  $token = Require-Env "GITHUB_TOKEN"
  if (-not (Test-Path -LiteralPath $Path)) { throw "AssetPath not found: $Path" }
  $uploadBase = ($UploadUrlTemplate -split "\{")[0]
  $url = "$uploadBase?name=$([Uri]::EscapeDataString($Name))"
  $headers = @{
    "Authorization" = "Bearer $token"
    "Accept"        = "application/vnd.github+json"
    "X-GitHub-Api-Version" = "2022-11-28"
    "User-Agent"    = "Cunning3D-ReleaseScript"
  }
  Invoke-RestMethod -Method Post -Uri $url -Headers $headers -ContentType "application/octet-stream" -InFile $Path -TimeoutSec 3600
}

$repo = $Repo.Trim()
if (-not ($repo -match "^[^/]+/[^/]+$")) { throw "Repo must be like owner/name" }

$asset = (Resolve-Path -LiteralPath $AssetPath).Path
$assetName = [IO.Path]::GetFileName($asset)
$hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash
$size = (Get-Item -LiteralPath $asset).Length

Write-Host "Repo: $repo"
Write-Host "Tag: $Tag"
Write-Host "Asset: $assetName ($size bytes)"
Write-Host "SHA256: $hash"

$apiRoot = "https://api.github.com/repos/$repo"

# Get existing release by tag (if any)
$rel = $null
try { $rel = Api GET "$apiRoot/releases/tags/$Tag" } catch { $rel = $null }

$bodyText = @"
## Download
- Windows (x86_64): $assetName

## SHA256
- $hash
"@.Trim()

if ($null -eq $rel) {
  Write-Host "Creating new release..."
  $rel = Api POST "$apiRoot/releases" @{
    tag_name   = $Tag
    name       = $ReleaseName
    body       = $bodyText
    draft      = [bool]$Draft
    prerelease = [bool]$Prerelease
  }
} else {
  Write-Host "Release exists, updating body/name..."
  $rel = Api PATCH "$apiRoot/releases/$($rel.id)" @{
    name = $ReleaseName
    body = $bodyText
    draft = [bool]$Draft
    prerelease = [bool]$Prerelease
  }
}

# If asset with same name exists, delete it first
if ($rel.assets) {
  $existing = $rel.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
  if ($existing) {
    Write-Host "Deleting existing asset: $assetName"
    Api DELETE "$apiRoot/releases/assets/$($existing.id)" | Out-Null
  }
}

Write-Host "Uploading asset..."
$uploaded = Upload-Asset $rel.upload_url $asset $assetName
Write-Host "Uploaded: $($uploaded.browser_download_url)"
Write-Host "Release: $($rel.html_url)"

