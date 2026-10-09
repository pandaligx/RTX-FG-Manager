# Changelog

## 4.2.9

Adapt [RTX MFG 1.4.2](https://github.com/dashdogy/RTX40MFG-Unlock/releases/tag/v1.4.2) and [RTX Encore 1.0.0-beta.2](https://github.com/SilyNoMeta/rtx-encore), which succeeds DLSSG-Transfusion 1.4.5.3. Existing scheme identities and per-game selections are retained.

- **RTX40 MFG · 1.4.2** improves experimental path-traced hair detection in The Witcher 3 after a game update made the option unavailable. Use `winmm.dll` beside `witcher3.exe`. The manager keeps this scheme RTX40-only; Vulkan is experimental and lacks Dynamic MFG.
- Preserve settings when upgrading RTX MFG to a newer version with a valid manager record and the same scheme and DLL entry. Exit the game, then choose **Install and apply**. Ownership checks protect unknown modifications instead of overwriting them.
- Encore uses one signed universal DLL with one of 19 loader names selected. Byte-preserving copying and renaming retain its signature; this does not apply to separately compiled alternative-proxies or ASI files. ASI is not deployed. Filenames do not add graphics API support.
- Bink entries require original game components beside the patch as `binkw64Hooked.dll` / `bink2w64Hooked.dll`. Existing files are protected; the manager does not rename or delete original Bink files.
- Add separate JSONC v4 and advanced controls for frame generation, Smooth Motion, super resolution, Neural Rendering, menus/shortcuts, statistics and compatibility using the actual configuration fields. Write explicit user edits only, preserving comments, unknown fields, menu state and inactive settings instead of overwriting game-side values with defaults.
- Upgrade valid manager-owned Transfusion installs in place after the game exits: back up the DLL, JSONC and record before migrating to `rtx-encore.jsonc`. Failures roll back; interrupted transactions recover from the journal. Unknown changes or altered backups are retained and reported. The stable ID neither mislabels old DLLs as upgraded nor permits new parameters to be written into the old protocol.
- The native menu opens once initially and then toggles with **Insert**, without ReShade. RTX40 MFG's **Backspace** menu remains separate. Shorten five-language help into eight practical sections while retaining contents, scrolling and copy support. Update actual deployment labels and the full `1.0.0-beta.2` version display.
- Include upstream third-party notices in each Encore ZIP and deployment. Removal handles owned patch files, configuration, matching notices and recognized logs while preserving unknown files, original Bink components, shared components and user-supplied NVIDIA DLLs.

**NR defaults to off** and requires user-supplied NVIDIA `nvngx_dlssnr.dll` **310.8.0** with DLSS SR/DLAA enabled in game; the manager does not distribute that file. RTX20 remains experimental. Open, precision, multiple passes and optimization options require target-game checks for performance and image trade-offs. **Smooth Motion defaults to off**; this unlock route requires RTX30 and exactly **617.42 / 617.14 / 616.92 / 616.64**. Upstream reports actual game validation only for **617.14**. A DX11/DX12/Vulkan menu does not imply every feature works in every game, and renaming adds no API capabilities.

Updating the manager alone never replaces game DLLs. Use Install and apply for the Transfusion migration or an eligible RTX MFG upgrade. Changing schemes or DLL entries still requires removal of the old patch; different DLL versions of an existing Encore install retain their prior removal requirement. This release has no new target-game or GPU validation; manager and offline file checks are not game compatibility or performance results.

## 4.2.8

Compared with 4.2.7, this release fixes six manager issues, adds RTX40 selection to the Delta Force scheme and replaces the help pages with a seven-section Markdown guide.

- Check all associated game processes when a custom plugin folder is used, covering installation, parameter changes and removal.
- Wait cancellably when DLL preparation and manager downloads share the cache, instead of immediately failing on contention; report a timeout with a retry instruction.
- Keep the newest successful cloud catalog received during scanning, file selection or deployment, then apply it after the operation. The active operation keeps its original settings.
- Refresh search indices when rescanning changes game contents or order, even if the number of games stays the same.
- Report cleanup as incomplete when temporary files await verification or caches still need removal. Show a warning and retain ownership records for retry.
- Exclude Unreal server builds with suffixes such as `Win64-Shipping` from game detection while retaining legitimate game names.
- Allow **RTX40 (SM89)** selection only for **0.3.5 · Delta Force**. Reuse the existing signed DLLs without rebuilding them or modifying device IDs. Original Github upstream remains the default, and other schemes retain their GPU ranges.
- Reorganize offline help into seven sections with Markdown headings, steps and tips, a contents list, scrolling, copy support and a narrow-window layout in all five languages.

Updating the manager preserves the game library and preferences and does not automatically replace game DLLs. Existing signed DLLs and cloud packages are unchanged. Compatibility, image quality, multipliers and performance still depend on the game, driver and GPU; no result is generalized to all hardware. **The reported RTX3060 6GB VRAM issue is not fixed by this release.**

## 4.2.7

- Fix update checks failing when the Gitee release API returns HTTP 403 due to rate limits. Domestic-first checks now use the repository's static `update.json`, without credentials or an API call.
- Fall back to GitHub on failed connections or invalid metadata. Preserve explicit GitHub preference and the legacy GitHub Release manifest endpoint. Failed checks never report that the app is up to date.
- Promote the static manifest only after release files on both sites are verified. File-size, SHA-256, version and Windows signature checks remain required.
- Add published 4.2.6 notes to offline history. Game DLLs, presets, deployment and removal behavior are unchanged.

An older client that cannot reach either update source needs one manual EXE download to receive this fix. Version 4.2.5's "manager update required" message for newer schemes protects against unsupported configuration formats.

## 4.2.6

Compared with 4.2.5, this release adds live theme previews, custom plugin folders,
in-app release history and a separate DLSSG-Transfusion integration.

- Upgrade to GPUI Kit 0.7.1 with GPUI Fast 0.1.2. Retain Windows first-frame, DPI and font compatibility adjustments, including operation without an external ICU runtime.
- Embed 21 GPUI Kit theme sets: 36 variants plus two defaults, for 38 offline color schemes. Open the sidebar palette for one searchable, scrollable list. Hover or use arrow keys to preview, click/Enter to save, or Esc/click outside to restore. System mode remembers light/dark choices separately; the list adapts to shorter windows and outside clicks do not activate controls underneath.
- Add custom deployment folders associated with the selected game's real EXE, including plugin directories without an EXE. Preserve game-process checks, settings, other MOD files and ownership-based removal.
- Request Windows administrator permission at startup, while retaining the existing updater entry path and user data location. Cancelling the UAC prompt stops startup.
- Keep a compatible scheme when switching RTX20/30/40, otherwise choose an available scheme. Disable unsupported choices and add links to the relevant projects.
- Update RTX40 MFG from 1.3.3 Hotfix 2 to 1.4.1 Hotfix 1, retaining its independent JSON profile and RTX40 restriction. Add VSync and a fixed-mode Reflex FPS limit; Dynamic uses its own target and preserves the inactive fixed limit. **The in-game Backspace menu remains upstream English.**
- Add DLSSG-Transfusion 1.4.5.3 with separate JSONC v3 settings. Select one matching `version.dll`, `dinput8.dll`, `dxgi.dll` or `winmm.dll` proxy; use Follow game, fixed 2X–6X or Dynamic mode. Preserve comments and unrelated settings. 5X/6X remain experimental; optional ASI/ReShade components and Smooth Motion controls are not bundled.
- Add current notes and offline history for 4.2.0–4.2.5 to Settings, with a version selector. Update prompts can display localized server notes while retaining support for older manifests without notes. Update five-language help.
- Exclude NVIDIA Vulkan diagnostic tools from game scanning and strengthen configuration-retention, log-cleanup and uninstall checks.

Updating the manager preserves the game library and preferences; it does not
automatically replace deployed game DLLs. Exit the game and uninstall its old
patch before installing a different version, scheme or entry.

Local format, Cargo check/test, strict Clippy and Release checks passed
(151 tests passed, 3 explicitly ignored), with final-build language/theme startup
and interactive Windows UI checks. No new physical-GPU, target-game or
cross-monitor validation is claimed. No CPU or game-performance gain has been
established; Transfusion RTX20 and older-GPU Vulkan paths still need physical testing.

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
