[CmdletBinding()]
param([switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Get-VerifiedResources.ps1') -Manifest (Join-Path $PSScriptRoot '../tests/fixture-manifest.json') -Destination (Join-Path $PSScriptRoot '../tests/fixtures/runtime') -VerifyOnly:$VerifyOnly
