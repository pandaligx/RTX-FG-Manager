[CmdletBinding()]
param([switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
# Kept as a compatible CI/bootstrap entry point. Downloads are implemented in Rust.
Write-Output 'No external downloader is required for the current manager build.'
