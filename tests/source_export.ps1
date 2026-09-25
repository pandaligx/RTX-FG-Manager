# Filesystem-only export tests. No uploads, credentials, registry or game access.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$id = [Guid]::NewGuid().ToString('N')
$relative = ".workspace/source-export-test-$id/seed"
& (Join-Path $root 'tools/export-source.ps1') -Destination $relative
$seed = Join-Path $root $relative
$tool = Join-Path $seed 'tools/export-source.ps1'
$fake = 'ghp_' + ('X' * 36)
[IO.File]::WriteAllText((Join-Path $seed 'README.md'), $fake)
$rejected = $false
try { & $tool -Destination '.workspace/rejected' } catch {
    $message = $_.Exception.Message
    $rejected = $message.Contains('README.md [GitHub credential]') -and -not $message.Contains($fake)
}
if (-not $rejected -or (Test-Path -LiteralPath (Join-Path $seed '.workspace/rejected'))) {
    throw 'A credential-like source file must fail before copying and must not be printed.'
}
[IO.File]::WriteAllText((Join-Path $seed 'README.md'), '# Synthetic export test fixture')
foreach ($relativeFile in @('log/private.json', 'app/main.py', 'rust/native/private.hpp', 'runtime/private.txt', 'AGENTS.md', 'RELEASE_WORKFLOW.md')) {
    $path = Join-Path $seed $relativeFile
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
    [IO.File]::WriteAllText($path, 'excluded synthetic marker')
}
& $tool -Destination '.workspace/accepted'
$accepted = Join-Path $seed '.workspace/accepted'
foreach ($relativeFile in @('log/private.json', 'app/main.py', 'rust/native/private.hpp', 'runtime/private.txt', 'AGENTS.md', 'RELEASE_WORKFLOW.md', 'app/tools/aria2c.exe')) {
    if (Test-Path -LiteralPath (Join-Path $accepted $relativeFile)) { throw "Private/binary file was exported: $relativeFile" }
}
foreach ($relativeFile in @('Cargo.toml', 'Cargo.lock', '.gitattributes', 'rust/build.rs', 'rust/assets/help.json', 'rust/assets/cache-hashes.json', 'rust/assets/licenses/GPUI-COMPONENT-LICENSE', 'rust/assets/licenses/LUCIDE-LICENSE', 'app/assets/cleanup-catalog.json', 'vendor/gpui_windows/Cargo.toml', 'vendor/sum_tree/Cargo.toml', 'app/tools/aria2.conf', 'tests/test_publication.py', '.github/scripts/mirror_gitee.py')) {
    if (-not (Test-Path -LiteralPath (Join-Path $accepted $relativeFile))) { throw "Required source resource is missing: $relativeFile" }
}
$nonempty = $false
try { & $tool -Destination '.workspace/accepted' } catch { $nonempty = $_.Exception.Message.Contains('absent or empty') }
if (-not $nonempty) { throw 'Existing export must be preserved.' }
$outside = $false
try { & $tool -Destination '../outside' } catch { $outside = $_.Exception.Message.Contains('inside this checkout') }
if (-not $outside) { throw 'Out-of-workspace export must be rejected.' }
$cloudIndex = Join-Path $seed 'cloud/indexes/synthetic-unpublished.json'
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($cloudIndex)) | Out-Null
[IO.File]::WriteAllText($cloudIndex, '{"synthetic":true}')
[IO.File]::WriteAllText((Join-Path $seed 'cloud/catalog.json'), '{"synthetic":true}')
& $tool -Destination '.workspace/metadata-withheld' -ExcludeCloudMetadata
$withheld = Join-Path $seed '.workspace/metadata-withheld'
foreach ($relativeFile in @('cloud/catalog.json', 'cloud/indexes/synthetic-unpublished.json')) {
    if (Test-Path -LiteralPath (Join-Path $withheld $relativeFile)) { throw 'Unpublished generated metadata must be withheld.' }
}
foreach ($relativeFile in @('cloud/schemes.json', 'tools/cloud_release.py')) {
    if (-not (Test-Path -LiteralPath (Join-Path $withheld $relativeFile))) { throw 'Withholding generated metadata must preserve its reviewed source and tool.' }
}
Write-Host 'PASS source export: includes, exclusions, secret redaction, empty target and workspace boundaries.'
