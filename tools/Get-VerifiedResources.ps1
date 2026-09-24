[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Manifest,
    [Parameter(Mandatory = $true)][string]$Destination,
    [switch]$VerifyOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-NoLinks([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Refusing linked path: $current"
            }
        }
        $parent = [IO.Path]::GetDirectoryName($current)
        if ($parent -eq $current) { break }
        $current = $parent
    }
}
function Target-Path([string]$Relative) {
    if ([IO.Path]::IsPathRooted($Relative) -or $Relative -match '(^|[\\/])\.\.([\\/]|$)|:') {
        throw "Invalid resource path: $Relative"
    }
    $path = [IO.Path]::GetFullPath((Join-Path $script:OutputRoot $Relative))
    if (-not $path.StartsWith($script:OutputRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Resource path escapes destination: $Relative"
    }
    Assert-NoLinks $path
    return $path
}
function Is-Verified([string]$Path, $Spec) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    Assert-NoLinks $Path
    if ((Get-Item -LiteralPath $Path -Force).Length -ne [long]$Spec.bytes) { return $false }
    $hashProperty = $Spec.PSObject.Properties['sha256']
    if ($hashProperty -and $hashProperty.Value) {
        if ($hashProperty.Value -notmatch '^[a-f0-9]{64}$') { throw 'Invalid pinned SHA-256' }
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ieq $hashProperty.Value
    }
    return $true
}
function Download-Resource($Spec, [string]$Path) {
    $urls = @($Spec.url)
    if ($Spec.PSObject.Properties['fallback_urls']) { $urls += $Spec.fallback_urls }
    $lastError = ''
    foreach ($url in $urls) {
        $uri = [Uri]$url
        if ($uri.Scheme -ne 'https' -or $uri.UserInfo) { throw 'Only credential-free HTTPS resource URLs are accepted' }
        $temp = "$Path.$([Guid]::NewGuid().ToString('N')).part"
        try {
            Write-Host "Downloading pinned resource: $($uri.Host)/$([IO.Path]::GetFileName($uri.AbsolutePath))"
            $aria = Join-Path $PSScriptRoot '../app/tools/aria2c.exe'
            if (Test-Path -LiteralPath $aria -PathType Leaf) {
                # Only execute the exact downloader pinned by this checkout.
                $toolManifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'build-resources.json') -Raw | ConvertFrom-Json
                if (-not (Is-Verified $aria $toolManifest.resources[0])) { throw 'The existing aria2 binary does not match the pinned tool manifest' }
                & $aria '--no-conf=true' '--check-certificate=true' '--enable-rpc=false' '--allow-overwrite=false' '--auto-file-renaming=false' '--file-allocation=none' '--connect-timeout=15' '--timeout=30' '--max-tries=2' '--split=8' '--max-connection-per-server=8' '--min-split-size=1M' "--dir=$([IO.Path]::GetDirectoryName($temp))" "--out=$([IO.Path]::GetFileName($temp))" $url
                if ($LASTEXITCODE -ne 0) { throw "aria2 exited $LASTEXITCODE" }
            } else {
                # Bootstrap only: aria2 cannot download itself before it exists.
                Invoke-WebRequest -Uri $url -OutFile $temp -UseBasicParsing -TimeoutSec 90
            }
            if (-not (Is-Verified $temp $Spec)) { throw 'Downloaded resource differs from its pinned size/hash' }
            [IO.File]::Move($temp, $Path)
            return
        } catch {
            $lastError = $_.Exception.Message
        } finally {
            if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Force }
            if (Test-Path -LiteralPath "$temp.aria2") { Remove-Item -LiteralPath "$temp.aria2" -Force }
        }
    }
    throw "All resource mirrors failed: $lastError"
}

$script:OutputRoot = [IO.Path]::GetFullPath($Destination).TrimEnd([IO.Path]::DirectorySeparatorChar)
Assert-NoLinks $script:OutputRoot
$data = Get-Content -LiteralPath $Manifest -Raw -Encoding UTF8 | ConvertFrom-Json
if ($data.schema -ne 1) { throw 'Unsupported resource manifest' }
$missing = [Collections.Generic.List[string]]::new()
foreach ($resource in $data.resources) {
    $pending = @()
    foreach ($output in $resource.outputs) {
        if (-not $output.PSObject.Properties['sha256'] -or $output.sha256 -notmatch '^[a-f0-9]{64}$') { throw 'Every output must have a pinned SHA-256' }
        $target = Target-Path $output.path
        if (Is-Verified $target $output) { continue }
        if (Test-Path -LiteralPath $target) { throw "Existing resource differs from pinned bytes; move it aside manually: $target" }
        $pending += $output
    }
    if ($pending.Count -eq 0) { continue }
    if ($VerifyOnly) {
        foreach ($output in $pending) { $missing.Add($output.path) }
        continue
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { $id = [BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($resource.url))).Replace('-', '').ToLowerInvariant() } finally { $hasher.Dispose() }
    $download = Target-Path ".downloads/$id.bin"
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($download)) | Out-Null
    if (-not (Is-Verified $download $resource)) {
        if (Test-Path -LiteralPath $download) { throw "Invalid cached download; remove this one file and retry: $download" }
        Download-Resource $resource $download
    }
    foreach ($output in $pending) {
        $target = Target-Path $output.path
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target)) | Out-Null
        $temporary = "$target.$([Guid]::NewGuid().ToString('N')).part"
        try {
            if ($output.PSObject.Properties['member']) {
                $archive = [IO.Compression.ZipFile]::OpenRead($download)
                try {
                    $entry = $archive.GetEntry($output.member)
                    if (-not $entry -or $entry.Length -ne [long]$output.bytes) { throw "Pinned ZIP entry missing or wrong size: $($output.member)" }
                    $inputStream = $entry.Open()
                    $outputStream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                    try {
                        $buffer = New-Object byte[] 65536
                        [long]$written = 0
                        while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                            $written += $count
                            if ($written -gt [long]$output.bytes) { throw 'ZIP entry exceeded pinned size' }
                            $outputStream.Write($buffer, 0, $count)
                        }
                    } finally { $outputStream.Dispose(); $inputStream.Dispose() }
                } finally { $archive.Dispose() }
            } else { [IO.File]::Copy($download, $temporary, $false) }
            if (-not (Is-Verified $temporary $output)) { throw "Resource verification failed: $($output.path)" }
            Assert-NoLinks $target
            [IO.File]::Move($temporary, $target)
        } finally {
            if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        }
    }
}
if ($missing.Count) { throw "Missing pinned resources ($($missing.Count)); run preparation without -VerifyOnly:`n$($missing -join "`n")" }
Write-Host 'All requested resource bytes verified. This is not a GPU/game compatibility test.'
