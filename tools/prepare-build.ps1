[CmdletBinding()]
param([switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Get-VerifiedResources.ps1') -Manifest (Join-Path $PSScriptRoot 'build-resources.json') -Destination (Join-Path $PSScriptRoot '../app/tools') -VerifyOnly:$VerifyOnly
