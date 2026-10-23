# Changelog

## 4.3.9

- Support RTX Encore 1.3.0: opt-in foveated NR with four custom controls, and the experimental RTX 20 Smooth Motion V1/V2 selector (NVIDIA 621.01, DX12; restart required). Show controls only for supported patch versions.
- Retire OpenNR settings for 1.3.0 while keeping older installations editable. Upgrading an OpenNR configuration turns NR off and explains how to enable NVIDIA NR explicitly; preserve comments, unknown fields, per-directory settings and transaction recovery.
- Keep the 4.3.8 native interface. Add the online-game injection risk notice, improve game-name candidate searches with missing spaces or punctuation, and clarify that settings in player notes require manual configuration. NR runtime remains 310.9.16.

- Choose the download source in Settings; remove the duplicate sidebar selector. Patches, NR runtimes and manager updates follow that preference, retaining fallback sources and offline caches. Match the risk notice to the smaller text above it.

## 4.3.8

- Support RTX Encore 1.2.2 with post-reconstruction sharpening mode and strength, off by default and shown only for supported versions. Preserve configuration/comments and complete bundles with dedicated proxies and shared Intel runtimes.
- Refine the compact native layout: consistent settings icons and independent columns; clearer DLL entries and player hardware in recommendation details, retaining full notes and fixed footer actions.
- Reorganize Settings into Appearance, Software updates, Game library and Cache maintenance, with aligned controls, a theme shortcut and a responsive single-column layout for smaller windows or larger text.
- Fix recommendations remaining on offline records after a retained email session becomes invalid. Clear the rejected session and fetch public setups again; sign in when sharing or rating. Distinguish login expiry from connection failures.
- Prevent a delayed recommendation request from clearing a newer sign-in. Keep local sign-out available during service outages and report errors saving the sign-out state.
- Preserve pending parameter changes after a partial multi-folder apply; a successful first folder no longer hides failures in other folders.
- Animate the sidebar's SVG thumbs-up when selecting a game or hovering over Player recommendations. Interface animations default to On; explicitly saved System/On/Off choices are preserved.

## 4.3.7

- Keep recommendation cards compact with a direct "View full details" button and legible default avatars. Present setup details in a compact table, with full-length notes, copying and fixed actions. Add a brief entrance fade to the recommendation button. Default to font size 13 while preserving saved preferences. Update to Encore 1.2.1, hide its retired compatibility policy and clarify Vulkan engine switching.

- Support RTX Encore 1.2.0: experimental XeSS/XeFG engines, SR input controls and per-pass NR tuning. Gate new settings by the installed patch version and preserve JSONC comments and unknown fields. NR remains on 310.9.16.
- Include official Intel runtimes in the complete Encore bundle, in a private directory with transaction recovery and ownership-based removal. Clean generated frame-time JSONL logs while retaining game-owned files.
- Display recommendation publication dates and rank by useful votes. Selecting a setup that needs a GPU display-name change opens explicit guidance and retains the requirement in the inspector.
- Keep historical shares readable. Confirm use of a newer version of the same project; retain the author's tested version. Removed projects and unavailable loaders cannot be applied automatically. The service remains compatible with older clients.


## 4.3.6

- Fix the Windows file picker rejecting some protected Xbox / Game Pass executables before validation. Recognize the sp26 renderer under the known COD package identity, retaining manifest, architecture and directory checks without substituting launchers or changing game permissions.
- Reuse verified game identities for recommendations, including recognized non-Steam games. Choose once when uncertain and correct the association later. Shares can specify platform and edition; administrators can associate game aliases and undo the latest association without losing setups or feedback.

- Add reversible reports with reasons and temporary hiding after reports from three distinct verified emails, pending admin review. Monitor the verification-email service. Recognize the standalone Wuthering Waves client by its installation directory without renaming unrelated Client executables.
- Add per-game recommendation cards, collapsed by default, with patch schemes, DLL entries, GPU-name guidance and NR. Preview before choosing a setup; verify an email to share notes or leave reversible useful/not-useful feedback. Authors can edit or delete their own shares, retaining feedback across edits. Select common email suffixes. Administrators can search, filter suspected duplicates, hide or delete setups. Retain a local browsing cache for outages.
- Upgrade GPUI Kit to 0.8.0 and GPUI Fast to 0.1.10, incorporating upstream text layout, dropdown, tooltip and retained-rendering fixes.
- Retain the current layout, font sizing, resizable sidebar and mouse/Tab focus behavior. Rebase the manager's Windows startup, DPI and font compatibility patches onto the new renderer.
- Show full deployment paths in the library, wrapping long paths. Slightly enlarge game icons and scale them with the font size. Open each folder with its folder icon; double-clicking a game row no longer opens a folder.
- Add folder hover/open feedback, place NR status alongside deployment status and remove its tooltip. Recognize the WeGame QQ Speed installation and its 64-bit game executable without requiring DLSS components.
- Extend discovery to Godot, Source/Source 2 and verified CRYENGINE layouts, and use platform-declared game entries independently of DLSS. Exclude additional launchers, tools and servers; match folder-button hover/press feedback to the parameter-settings button.
- Fix NR installation reaching only the first folder of a multi-folder game. Reuse one download across all deployment folders and remove owned NR from all of them, preserving per-folder ownership and original-file restoration. The inspector distinguishes partial from complete installation.
- Existing patch resources and per-game configurations remain unchanged.
- Replace generic review warnings with specific causes and per-game actions. Residual cleanup explains its scope before execution, preserves working folders and independent NR, and stops if a patch has been reinstalled. Technical file details remain available in a collapsed section.
- Recover legacy single-DLL records saved during failed cleanup, retaining file-scope validation so cleanup can be retried after files are unlocked.

## 4.3.5

- Support MFG Unlocker 1.6.1: DLSS SR/RR presets, RTX 30 pacing and an experimental per-game FG option. Preserve unknown JSON/INI fields and recover interrupted edits. New settings require MFG 1.6+.
- Remove this game’s MFG current/previous logs and interval traces from both its executable directory and the legacy TEMP location; preserve other games and unknown files.
- Update RTX Encore resources to 1.1.4, including the Witcher 3 DX12 startup fix from 1.1.3 and the neural-rendering flicker/menu-cursor fixes from 1.1.4. Keep the existing configuration protocol and NR 310.9.16. Game upgrades remain user initiated.

## 4.3.4

- Fix [#11](https://github.com/pandaligx/RTX-FG-Manager/issues/11): installed patch versions in the game library now use deployment records, matching the per-folder tooltip. Updating the manager or cloud catalog no longer makes an older installed patch appear updated; different installed versions across folders remain visible.

## 4.3.3

- Show one path line per deployment folder in compact game rows, with rescan updates saved. Deployment status sits between the game name and paths at the same text size. A compact gear beside the name opens parameter settings with hover, pressed and open feedback.
- Expand game naming through launcher records, local localized metadata and reviewed multi-signal rules. Add offline Chinese names, manual overrides and bilingual search; preserve uncertain originals. The GitHub/GitCode name catalog can update independently.
- Move GPU model editing into a centered dialog opened from the inspector. Mouse interaction has no focus ring; Tab navigation retains a thin border.
- Clear pending edits when values return to their original state. Discard unapplied drafts without repeated restart warnings; provide owned-cleanup details and retry.
- Improve Xbox/GDK identity, protected-executable guidance and Minecraft Launcher icons. Add the hardware-accelerated GPU scheduling switch with reboot feedback that clears when reverted to the current boot state.
- Support RTX Encore 1.1.2's smooth X3/X4 field migration. Recommend Neural Rendering 310.9.16 while retaining explicit older-version selection and recovery.

See [game naming coverage and limits](docs/game-names.md). Updating the manager does not automatically replace installed game patches.

## 4.3.2

- Drag game rows to reorder the library; save the order and retain positions on rescan. Removal confirmation shows game icons, display names and paths using the configured font size.
- Hide terminal `.exe` suffixes consistently in game labels while preserving real paths.
- Exclude D3D12 compiler helpers and prefer The Witcher 3's `bin/x64_dx12/witcher3.exe`; repair saved targets and retain cleanup access to previously managed folders.
- Adapt signed RTX Encore 1.1.0: Fast SR, NR A/B models, NR hotkey and legacy preset migration, preserving comments, unknown fields and menu state.
- Offer optional NR 310.8.0 / 310.9.16 downloads with GitCode/GitHub fallback, verified cache, original-file backup and uninstall recovery. NR remains off by default; game output/depth requirements still apply.
- Clean owned Encore session status files while retaining unknown/manual files. Raise the complete-archive size limit and prevent stale catalogs from downgrading bundled resources.
- Compact DLSS Neural Rendering sidebar with installed status and independent uninstall; broaden bounded DLSS component discovery for engine/plugin layouts.
- Keep pending-edit feedback, reset controls and Apply in a fixed parameter footer. Warn before closing with unapplied changes; valid drafts remain available.
- Explain pending game cleanup and offer scoped retry. List blocking background processes and allow confirmed, identity-checked termination followed by retry.

See [Encore 1.1.0 and Neural Rendering](docs/encore-1.1.0.md). Manager checks do not establish game/GPU compatibility or performance.

## 4.3.1

- Adapt signed RTX Encore 1.0.0 stable and MFG-Unlocker 1.5.0-hotfix.1, retaining the complete dlssg_for_sm86 0.4.0 and custom Delta Force/Vulkan routes.
- Add three Encore opt-in controls: disableInGameMenu, logDxgiWrappers and crashDetails. All default off and require restart. Preserve existing JSONC bytes on upgrade, including comments and menu state.
- Recognize Encore's generated session dump filenames during owned uninstall; retain unknown/manual captures and files beyond the safe cleanup size bound.
- Rework navigation and inspector layout: default compact sidebar, static brand, resizable panels, game title/icon, GPU/HAGS on the right, theme switch in the title bar, and font size 12–18. Fix title-bar misalignment after resizing text.
- Remove community recommendations, embedded help and the bottom log panel. Restore the single configuration editor. Retire initial/0.2.6 new-install choices and the GPU-series selector; retain legacy cleanup.
- Permit multiple supported DLL entries while preserving ASI/plugin exclusivity. Remove game uses a trash icon without a count. Correct duplicate Encore version headings and dropdown/tooltip layout.
- Fix metadata requests on services rejecting HEAD. Prefer GitCode, then GitHub. Distribute each scheme as one complete ZIP in the fixed Resources release; retain compatible older-client manifests.
- Rebuild English/Chinese documentation with current workflows, menus, recovery guidance and retained GitHub download badges.

Manager updates do not automatically replace game patches. Validation covers file/configuration transactions and the Windows UI; it is not game/GPU or anti-cheat validation.

## 4.3.0

- Changelog opens online release notes using the domestic/GitHub source preference. Remove the old embedded changelog dialog so published notes can change independently of client signing.

- dlssg_for_sm86 0.4.0, MFG Unlocker 1.5.0 and RTX Encore rc.3, retaining legacy schemes and managed configuration migration.
- Transactional component deployment, dedicated proxies/ASI, exclusive OptiScaler 10 plugin mode with conditional restoration, and game-owned log cleanup.
- Advisory GPU detection with unrestricted scheme selection and manual legacy component choice.
- Retire the first-release installation option while preserving legacy recognition and cleanup; interactive web help covers all six remaining schemes.
- Separate Player configurations entry with simplified sharing; local settings, named configurations, selected import diffs, exact-byte undo and operation history remain under Preset parameters.
- Independently updated offline HTML help with WebView2 and browser fallback.
- Independent signed community identities; match only GPU model and driver version, then working confirmations and likes. Refresh every 30 seconds, with publication preview, revision-specific interactions and moderation.
- GitCode domestic resources, GitHub fallback and Gitee transition; older client metadata remains compatible.

Released on 2026-10-15. These changes do not establish game/GPU performance or anti-cheat compatibility. The community and embedded help described here were removed in 4.3.1.

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
