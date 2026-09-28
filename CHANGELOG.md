# Changelog

## 4.2.5

- Compact the signed-backend resource replacement to reclaim obsolete file space: approximately 28.8 MiB for upstream 0.3.5 and 32.3 MiB for the Delta variant. Preserve the already-signed initial proxies.

- Remember the scheme and DLL entries for each game. Switching games restores
  that game's selection and presets; mixed batch installs use each game's own settings.
- Keep DLL entries in the right sidebar, following the selected game. Right-align
  the deployment badge above a compact, neutral Presets button. Use Small buttons
  consistently across the toolbar, settings, deployment actions and dialog footers.
  Separate edits awaiting application from the actual deployment badge; show real scheme names,
  proxy filenames and directory counts instead of mislabelling grouped installs R2.
- Add the separate RTX40MFG-Unlock 1.3.3 Hotfix 2 profile, using one renamed,
  signed universal DLL and its own JSON configuration. Preserve unrelated menu
  settings; require existing Streamline DLSS FG. Vulkan is experimental; Dynamic
  mode is DX12 only. Original game and other MOD files remain protected.
- Repackage both 0.3.5 groups and the two initial backends with code-identical
  signed internal backends. Update their extraction hashes and cleanup identities;
  all modified outer proxies are publisher-signed. Each contains one matching
  backend without duplicate storage. This addresses unsigned embedded backends,
  not a guarantee of anti-cheat acceptance or game compatibility.
- Update five-language help, protocol documentation and filesystem regressions.
  The default remains upstream 0.3.5; 0.2.6 and Dlssg-MFG-Vulkan DLLs are unchanged.
- Updating the manager preserves the library and preferences and does not replace
  deployed game DLLs automatically. Exit the game, uninstall its old patch, and
  reinstall the chosen scheme to obtain the newly signed backend package.

Format, Cargo check/test, strict Clippy and Release build passed, with Windows
loader and five-language UI checks. No new physical RTX20/30/40 game validation
is claimed; RTX40 Bink entry loading was skipped without original game components.

## 4.2.4

- Group verified game executables by installation into one library entry. When
  a game uses rendering executables in multiple directories, installation and
  removal process each directory; duplicate EXEs in one directory deploy once.
- Recognize Steam installations from bounded local manifests, select the
  rendering EXE, and exclude the Steam client, protected launchers (including
  Shipping-suffixed anti-cheat launchers), support tools and common game tools
  from automatic deployment. Separate Unreal projects sharing one Engine are
  kept apart. Manual EXE addition remains available.
- Fold older duplicate scan entries into the game card while preserving saved
  presets, selection and ownership-based uninstall access for old deployments.
  Parameter application covers all deployed directories in the group, but
  refuses incomplete or mixed-scheme groups instead of reporting false success.
- Update the five-language help text. No game DLL or cloud catalog changes.

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
