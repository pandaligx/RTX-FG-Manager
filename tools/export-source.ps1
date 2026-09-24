[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Destination,
    [switch]$ExcludeCloudMetadata
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd([IO.Path]::DirectorySeparatorChar)
$stageRoot = [IO.Path]::GetFullPath((Join-Path $root '.workspace'))
$target = [IO.Path]::GetFullPath((Join-Path $root $Destination)).TrimEnd([IO.Path]::DirectorySeparatorChar)
if (-not $target.StartsWith($stageRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Source exports must use a new directory inside this checkout''s .workspace/ folder.'
}
function Assert-NoLinks([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Linked path is not permitted: $current" }
        }
        $parent = [IO.Path]::GetDirectoryName($current)
        if ($parent -eq $current) { break }
        $current = $parent
    }
}
Assert-NoLinks $target
if (Test-Path -LiteralPath $target) {
    if (-not (Test-Path -LiteralPath $target -PathType Container) -or @(Get-ChildItem -LiteralPath $target -Force).Count -ne 0) {
        throw 'The export directory must be absent or empty; this tool never removes or overwrites an existing export.'
    }
}
$files = [Collections.Generic.SortedSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
function Add-File([string]$Relative, [bool]$Required = $true) {
    $source = Join-Path $root $Relative
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        if ($Required) { throw "Required source file is missing: $Relative" }
        return
    }
    Assert-NoLinks $source
    $null = $files.Add($Relative.Replace('\', '/'))
}
function Add-Tree([string]$Relative, [string[]]$Extensions) {
    $source = Join-Path $root $Relative
    if (-not (Test-Path -LiteralPath $source -PathType Container)) { return }
    Assert-NoLinks $source
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($source)
    while ($pending.Count) {
        $directory = $pending.Pop()
        foreach ($item in Get-ChildItem -LiteralPath $directory -Force) {
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Linked source entry is not permitted: $($item.FullName)" }
            if ($item.PSIsContainer) { $pending.Push($item.FullName); continue }
            if ($Extensions -contains $item.Extension.ToLowerInvariant()) {
                Add-File $item.FullName.Substring($root.Length + 1)
            }
        }
    }
}

# This is a source release allowlist, not a recursive copy of the private workspace.
foreach ($name in @(
    'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', '.gitignore',
    'BUILDING.md', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'rust/build.rs',
    'rust/cloud-catalog.json', 'rust/cloud-identities.json', 'rust/delta-runtime.json', 'rust/ui-translations.json',
    'app/assets/cleanup-catalog.json', 'app/assets/FONTAWESOME_LICENSE.txt', 'app/assets/licenses/aria2/COPYING',
    'app/locales/en.json', 'app/locales/ru.json', 'app/locales/ja.json', 'app/locales/ko.json', 'app/tools/aria2.conf',
    'tools/build-resources.json', 'tools/Get-VerifiedResources.ps1', 'tools/prepare-build.ps1',
    'tools/prepare-fixtures.ps1', 'tools/export-source.ps1', 'tests/fixture-manifest.json', 'tests/source_export.ps1', 'tests/test_publication.py', '.github/workflows/build.yml',
    'vendor/gpui_windows/Cargo.toml', 'vendor/gpui_windows/build.rs', 'vendor/gpui_windows/LICENSE-APACHE',
    'vendor/sum_tree/Cargo.toml', 'vendor/sum_tree/LICENSE-APACHE', 'vendor/sum_tree/LOCAL_CHANGES.md'
)) { Add-File $name }
foreach ($name in @('README.md', 'README.zh-CN.md', 'CHANGELOG.md', 'CHANGELOG.zh-CN.md', 'CONTRIBUTING.md', 'SECURITY.md')) { Add-File $name $false }
Add-Tree 'rust/src' @('.rs')
Add-Tree 'rust/assets' @('.json', '.svg', '.png', '.ico', '.txt', '.md')
Add-Tree 'vendor/gpui_windows/src' @('.rs', '.hlsl')
Add-Tree 'vendor/sum_tree/src' @('.rs')
foreach ($item in Get-ChildItem -LiteralPath (Join-Path $root 'tests') -File -Filter '*.rs') { Add-File "tests/$($item.Name)" }
foreach ($version in @('420', '421', '422')) { Add-Tree "tests/fixtures/catalog$version" @('.json') }
# Public cloud metadata and publish automation are separately reviewed source inputs.
foreach ($name in @('cloud/schemes.json', 'tools/cloud_release.py', 'docs/cloud-publishing.md', '.github/scripts/mirror_gitee.py')) { Add-File $name $false }
if (-not $ExcludeCloudMetadata) {
    Add-File 'cloud/catalog.json' $false
    Add-Tree 'cloud/indexes' @('.json')
}
foreach ($name in @('.github/workflows/mirror-gitee.yml', '.github/workflows/payloads.yml', '.github/workflows/cloud-probe.yml')) { Add-File $name $false }
foreach ($name in @('docs/screenshot-home.png', 'docs/screenshot-home-zh.png', 'docs/screenshot-home-themes.png')) { Add-File $name $false }

$binaryExtensions = @('.png', '.ico')
$findings = [Collections.Generic.List[string]]::new()
$rules = [ordered]@{
    'private-key material' = '-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----'
    'GitHub credential' = '(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})'
    'credential-bearing URL' = 'https?://[^\s/"''<>]+:[^\s/"''<>]+@[^\s"''<>]+'
    'assigned long credential' = '(?i)(?:access_token|gitee_token|github_token|token|api_key|auth_token|password|client_secret|secret)["'']?\s*[:=]\s*["''][A-Za-z0-9_+/.=-]{20,}["'']'
    'developer home path' = '(?i)[A-Z]:[\\/]+Users[\\/]+(?:Administrator|[^\\/\s"'']+)[\\/]+(?:Desktop|AppData|Documents)'
}
# Reviewed synthetic invalid-URL inputs in rejection tests. Both path and exact
# input hash must match; changed inputs and non-test occurrences still fail.
$syntheticInputs = @{
    'rust/src/cloud.rs' = '4f85bc59f748edddacc5e3639f8e3c99362c24cdeaf8adaed0dee75faded566c'
    'tests/v423_transfer.rs' = 'e4b4359e2f12069916814ed61dcde4be96912ac5b4c35b87b2822cf2d4b83972'
}
foreach ($relative in $files) {
    if ($relative -match '(?i)(^|/)(development|runtime|payloads|log|\.git|rust/native)(/|$)|\.(?:dll|exe|pfx|p12|key|pml|dmp)$|(^|/)(agent\.md|AGENTS\.md|RELEASE_WORKFLOW\.md|\.env(?:\..*)?)$') {
        throw "Forbidden item reached source allowlist: $relative"
    }
    $source = Join-Path $root $relative
    if ($binaryExtensions -contains [IO.Path]::GetExtension($relative).ToLowerInvariant()) { continue }
    $text = [IO.File]::ReadAllText($source)
    foreach ($rule in $rules.GetEnumerator()) {
        foreach ($match in [regex]::Matches($text, $rule.Value)) {
            $synthetic = $false
            if ($rule.Key -eq 'credential-bearing URL' -and $syntheticInputs.ContainsKey($relative)) {
                $testContext = $relative.StartsWith('tests/') -or $text.LastIndexOf('#[cfg(test)]', $match.Index, [StringComparison]::Ordinal) -ge 0
                $hasher = [Security.Cryptography.SHA256]::Create()
                try { $hash = [BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($match.Value))).Replace('-', '').ToLowerInvariant() } finally { $hasher.Dispose() }
                $synthetic = $testContext -and $hash -eq $syntheticInputs[$relative]
            }
            if (-not $synthetic) { $findings.Add("$relative [$($rule.Key)]") }
        }
    }
}
if ($findings.Count) {
    # Report only filenames and rule names. Never echo the matched credential itself.
    throw "Source export needs review; no files copied:`n$($findings -join "`n")"
}

[IO.Directory]::CreateDirectory($target) | Out-Null
$manifest = [Collections.Generic.List[object]]::new()
foreach ($relative in $files) {
    $source = Join-Path $root $relative
    $destinationFile = Join-Path $target $relative
    Assert-NoLinks $source
    Assert-NoLinks $destinationFile
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destinationFile)) | Out-Null
    $before = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::Copy($source, $destinationFile, $false)
    $after = (Get-FileHash -LiteralPath $destinationFile -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($before -ne $after) { throw "Source changed while exporting: $relative" }
    $manifest.Add([pscustomobject]@{ path = $relative; bytes = (Get-Item -LiteralPath $destinationFile).Length; sha256 = $after })
}
$reportPath = "$target.manifest.json"
if (Test-Path -LiteralPath $reportPath) { throw 'Manifest already exists; use a new staging directory.' }
[pscustomobject]@{
    schema = 1
    files = $manifest
    generated_cloud_metadata_excluded = [bool]$ExcludeCloudMetadata
    scope = 'Public Rust manager and required resources; native DLL bridge sources, SDKs, binaries, private records and old Python application excluded'
    checks = @('explicit allowlist', 'no reparse-point sources or targets', 'credential-pattern review', 'copied byte hashes')
    limitations = 'Pattern review is not a full security or legal audit. Source build, dependency licenses and public documentation must also be reviewed before publishing.'
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Host "Exported $($manifest.Count) reviewed source/resource files to $target"
Write-Host "Evidence manifest (outside the source export): $reportPath"
Write-Host 'Nothing has been committed, signed, uploaded or published.'
