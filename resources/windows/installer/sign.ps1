param(
  [Parameter(Mandatory = $true)]
  [string]$filePath
)

$ErrorActionPreference = "Stop"

if (-not $env:ENDPOINT) { throw "ENDPOINT env is required." }
if (-not $env:ACCOUNT_NAME) { throw "ACCOUNT_NAME env is required." }
if (-not $env:CERT_PROFILE_NAME) { throw "CERT_PROFILE_NAME env is required." }
if (-not $env:FILE_DIGEST) { throw "FILE_DIGEST env is required." }
if (-not $env:TIMESTAMP_DIGEST) { throw "TIMESTAMP_DIGEST env is required." }
if (-not $env:TIMESTAMP_SERVER) { throw "TIMESTAMP_SERVER env is required." }

Invoke-TrustedSigning `
  -Endpoint $env:ENDPOINT `
  -CodeSigningAccountName $env:ACCOUNT_NAME `
  -CertificateProfileName $env:CERT_PROFILE_NAME `
  -FileDigest $env:FILE_DIGEST `
  -TimestampDigest $env:TIMESTAMP_DIGEST `
  -TimestampRfc3161 $env:TIMESTAMP_SERVER `
  -Files $filePath

