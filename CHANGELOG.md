# Changelog

## 4.2.3

Compared with 4.2.2, this release improves domestic-first downloads, per-game
presets and game discovery, and publishes the Rust manager source with automated
GitHub/Gitee cloud maintenance. The existing signed game DLLs are unchanged.

### Downloads and updates

- Apply the current route preference when downloading, even after checking on
  another mirror. Handle public Gitee APIs that reject HEAD but accept anonymous
  GET, so a working domestic endpoint is not incorrectly skipped.

- Share one download preference across catalogs, patch ZIPs and EXE updates.
  New installations try domestic Gitee first; an explicit GitHub preference is
  remembered. Body downloads use the included aria2 downloader.
- Show the current file, source, downloaded size and speed alongside compact
  circular progress. Report a failed or persistently slow source before falling
  back, while retaining resume, cancellation and integrity checks.
- Keep update size, SHA-256 and publisher-signature checks. Preserve the update
  format used by older managers; publish the new update manifest only after the
  signed EXE is uploaded and downloaded again for verification.

### Game library and presets

- Keep multiplier controls visible and fold advanced controls into a separate
  panel. Mark changed parameters as pending and allow applying only the current
  game's INI when the same scheme is already deployed.
- Correct MFG multiplier relationships and the enabled state of dynamic controls.
  Preserve separate settings for each game and scheme and keep unrelated INI
  fields and comments.
- Fix overly broad scanner exclusions for utility-like names and retain multiple
  game candidates in one directory. Record scan exclusions and update the library
  in batches without replacing install/uninstall safety checks with UI caches.
- Check running games and file conflicts before deployment. Batch results show
  successful, failed and unprocessed games separately.
- Make failed preference saves visible and retryable. Recover missing or malformed
  settings from the last successful backup when available, preserve damaged
  originals, and reject unknown schemas or unsafe paths.

### Source and cloud maintenance

- Publish the Rust manager source, pinned build inputs, build instructions and a
  Windows CI workflow. Retire the old Python/Tk application. The manager remains
  independent of Python at runtime; maintenance scripts are separate tools.
- Maintain cloud schemes in `cloud/schemes.json`. An automated workflow mirrors
  immutable ZIPs from the fixed GitHub resource release to Gitee, verifies both
  downloads, then publishes the index and catalog. Routine private-drive uploads
  are no longer needed; old resources remain available for older clients.
- Keep the manager source and documentation mirrored to Gitee. Gitee resource
  assets use a separate repository so resource releases cannot replace the
  manager's latest release seen by old clients.

### Validation boundaries

GPUI 0.6.6 was reviewed for compatibility; this release retains the existing
pinned component versions and Windows patches. It does not claim a GPUI upgrade,
new game DLL changes, additional GPU/game validation, or measured domestic
performance with the VPN disabled. Network speed depends on the actual route,
provider and proxy configuration; displayed source and speed help identify it.

Local formatting, Cargo check/test, strict Clippy and Release build completed.
A clean Git checkout with Windows line-ending conversion enabled passed all
102 default tests; three explicitly opt-in tests remained ignored. Exact-byte
catalog fixtures now survive that checkout. These results are not a claim that
the separate remote Windows CI run has completed every stage.

The existing signed aria2 binary is pinned as a build input. Publishing the
manager source does not recover aria2's unavailable historical source/build
records; see [third-party notices](THIRD_PARTY_NOTICES.txt).

Earlier published versions: [GitHub Releases](https://github.com/pandaligx/RTX-FG-Manager/releases).
