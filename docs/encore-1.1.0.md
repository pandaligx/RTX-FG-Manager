# RTX Encore 1.1.0 and Neural Rendering

For RTX-FG-Manager 4.3.2. Updating the manager does not replace mods already installed in games.

1. Exit the game, select its rendering executable, and install **RTX Encore · 1.1.0**.
2. Use **DLSS Neural Rendering** in the selected game’s right sidebar. Select a version and click **Install** when DLSS Super Resolution components are detected.
3. Choose **310.8.0** for the upstream compatibility option, or experimental **310.9.16** for A/B models. B is the smaller, default network; A is larger.
4. Enable DLSS SR or DLAA in game. Press **Insert**, then enable Neural Rendering. **Ctrl+Alt+N** toggles the effect by default; custom bindings or a disabled menu take precedence.

The sidebar shows the title and status, a full-width version selector, then Install/Uninstall buttons on a separate row so longer translations have more space. Full paths and details appear in a tooltip; the game list adds an NR badge. Missing files and external installations are distinguished. Installed does not mean enabled in-game. Effect settings remain in the parameter editor, with Apply fixed below its scrolling content.

The fixed footer also contains reset controls and a prominent pending-edit status. Closing with unapplied changes offers a return to editing or closing without applying. Valid drafts are retained; only successful application clears the pending status. Resetting defaults also requires Apply.

Component discovery searches the current installation, including deep Unreal plugin/ThirdParty paths, Unity `_Data/Plugins`, bin/x64 and other plugin folders. It does not follow links or search neighboring games/backups. Budget limits are reported as incomplete detection. Streamline wrappers alone and FG/RR/NR DLLs are not mistaken for an SR runtime.

The optional downloads are about 108 / 180 MiB. GitCode is preferred with GitHub fallback and reusable verified cache. Installing a runtime does not enable NR or replace other NVIDIA/Streamline DLLs.

DLSS component detection is only an installation hint. NR needs active DLSS output and depth; the RR path needs compatible denoised output. RTX 30/40 are the main targets; RTX 20 and runtime 310.9.16 are experimental. Expect additional GPU cost and compare actual image quality and performance.

| Setting | Default | Application | Scope |
| :--- | :--- | :--- | :--- |
| `dlssSuperResolution.fastSuperResolution` | `false` | Game restart | RTX 30, DX12, DLSS 310.9.x M/L only. Not RR or DLAA; not bit-identical output. |
| `dlssPresets.dlssNrPreset` | `b` | Live | `a`, `b`, `auto`. 310.8.0 has one model; Open ignores this option. |
| `keyboardShortcuts.hotkeyToggleNr` | `Ctrl+Alt+N` | Hotkey setting | Toggle NR for comparison. |

Managed configuration migration maps the old `neuralRendering.advanced.nrPreset` values 0/3 to `auto`, 1 to `a`, and 2 to `b`. An existing canonical field wins. Comments, unknown fields and menu state are preserved. Experimental options stay disabled by default.

Existing NR files are backed up in `.rtx-fg-nr` beside the mod. Uninstall restores the original, or removes the managed file if none existed. Switching runtime versions retains the first original. Externally changed files or damaged backups cause a conflict instead of an overwrite. Known session status/log files are cleaned; unknown files and manual captures remain.

The sidebar Uninstall action removes managed NR independently while keeping Encore and its settings. External NR files without manager ownership are preserved. Pending cleanup exposes the reason and a retry action scoped to the current game. If the game still runs in the background, the manager lists its process name, PID and path. Exit it manually and recheck, or confirm termination and retry; forced termination may lose unsaved progress. Processes whose path and creation time cannot be verified require manual handling.

Game labels now hide a terminal `.exe` without renaming files. The Witcher 3 uses `bin/x64_dx12/witcher3.exe` for its icon and installation target, excluding D3D12 compiler helpers. Stored incorrect targets are repaired on load; previously managed targets remain available for cleanup.

See [upstream NR guide](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/NEURAL-RENDERING.md), [settings](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/SETTINGS.md), [release notes](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/CHANGELOG.md), and the [detailed Chinese analysis and documentation references](encore-1.1.0.zh-CN.md). Manager tests cover configuration, scanning, downloads and ownership recovery; they do not establish game/GPU compatibility or performance. The installed Hub was inspected without modifying real games.
