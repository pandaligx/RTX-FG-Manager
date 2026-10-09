# RTX Frame Generation Manager · by 小南瓜

**Install, tune and remove frame-generation patches from one place.**
A portable Windows app for RTX 20 / 30 / 40, with a native Rust / GPUI interface.

**English** · [简体中文](README.zh-CN.md)

<p align="center">
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9"><img alt="release" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9"><img alt="downloads" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?logo=windows&logoColor=white">
  <a href="LICENSE"><img alt="manager license MIT" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

- **One game library:** scan or add games, install individually or in batches, and see what is actually deployed.
- **Settings that follow each game:** remember the scheme, DLL entry and presets; download matching files when needed.
- **Flexible destinations:** deploy beside the game EXE or to an associated plugin folder, then remove files by ownership.
- **Make it yours:** preview 38 offline color schemes; use Chinese, English, Russian, Japanese or Korean.

## Download

**Current version: 4.2.9** · [Gitee download](https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9) · [GitHub download](https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9) · [Changelog](CHANGELOG.md)

Download **`RTXManager-v4.2.9-x64.exe`** for Windows 10 / 11 x64. The release page includes the signed EXE and checksums. No separate Python, Rust, CUDA Toolkit or aria2 installation is needed; game DLLs are downloaded on demand.

### What changed in 4.2.9: RTX Encore and RTX MFG 1.4.2

Version 4.2.9 adds **[RTX40 MFG · 1.4.2](https://github.com/dashdogy/RTX40MFG-Unlock/releases/tag/v1.4.2)** and updates the existing **DLSSG-Transfusion 1.4.5.3** scheme to **[RTX Encore 1.0.0-beta.2](https://github.com/SilyNoMeta/rtx-encore)**, retaining each game's scheme selection.

- **RTX MFG 1.4.2 improves The Witcher 3's experimental path-traced hair detection** after a game update made the option unavailable. Use `winmm.dll` beside `witcher3.exe` for this game. The manager keeps this scheme RTX40-only; Vulkan remains experimental without Dynamic MFG, and the menu key is **Backspace**.
- **Upgrade RTX MFG while keeping settings:** exit the game and choose **Install and apply**. A valid manager record, the same scheme and DLL entry, and a newer target version are required. The existing installation must pass ownership checks; unknown modifications are protected. Updating the manager alone does not replace game DLLs.

- **Press Insert for Encore's menu.** It opens once on first launch and can be rebound. Its native DX11/DX12/Vulkan menu needs no ReShade; RTX40 MFG retains Backspace.
- Full JSONC v4 advanced settings preserve comments, unknown keys and menu state. Only explicit edits are written; other values come from the latest disk document. For an old manager-owned Transfusion install, exit the game and choose **Install and apply** to migrate in place; do not uninstall first and lose settings. The DLL, configuration and ownership record are backed up before changes. Failures restore originals; unknown modifications and unfinished recovery material remain.
- Copying and renaming the same signed `rtx-encore.dll` preserves its bytes and signature, so aliases need no separate signing. This does not cover separately built `alternative-proxies` or ASI binaries; this version deploys only the 19 DLL entries, without ASI. A filename selects the loading route and does not add graphics API support.
- **NR is off by default.** Supply NVIDIA's `nvngx_dlssnr.dll` **310.8.0** beside the patch and enable DLSS SR/DLAA in the game. The manager does not distribute that NVIDIA file. NVIDIA/Open engines, passes, precision and experimental settings may increase GPU time or VRAM and change image quality; follow restart guidance. RTX20 remains experimental, with no promised performance gain.
- **Smooth Motion is off by default.** This unlock route is RTX30-only and accepts exactly **617.42, 617.14, 616.92 or 616.64**. Upstream reports actual game validation only for **617.14**, not the other three. Select the game's actual API and restart. “Driver prepared” and estimated FPS do not establish that interpolated frames were presented.

Five-language offline help is now a concise eight-section guide covering setup, scheme selection, upgrades and troubleshooting, with contents, scrolling and copy support.

Upstream details: [installation](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/INSTALLATION.md) · [menu](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/MENU-AND-OVERLAY.md) · [settings](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/SETTINGS.md) · [NR](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/NEURAL-RENDERING.md) · [Smooth Motion](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/SMOOTH-MOTION.md). Loader checks and offline regressions are not target-game or GPU acceptance tests.

### What changed in 4.2.8

Compared with **4.2.7**, this release fixes six manager issues:

- Custom plugin folders check every associated game EXE process during installation, parameter changes and removal.
- DLL preparation and manager downloads wait cancellably for the shared cache; a timeout asks you to retry.
- New cloud metadata received during scanning, file selection or deployment is retained and applied after the operation without changing settings already in use.
- Search results refresh when a same-count rescan changes game order or names.
- Removal with temporary files awaiting verification or pending cache cleanup is reported as incomplete with a warning; recovery records remain for retry.
- Unreal server builds with suffixes such as `Win64-Shipping` are excluded from game detection.

**RTX40 (SM89)** selection is now available for **0.3.5 · Delta Force** only, reusing existing signed DLLs without rebuilding them or changing device IDs. Original Github upstream remains the default, and other schemes retain their GPU ranges. Five-language offline help is organized into seven Markdown sections with a contents list, steps, tips, copy support and a narrow-window layout.

<p align="center">
  <img src="https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/docs/screenshot-home.png" alt="RTX Frame Generation Manager 4.2.8 in the light theme with demo games" width="980">
</p>
<p align="center">Actual version 4.2.8 window in the light theme, using a demo game library.</p>

## Choose a scheme

**The default is 0.3.5 · Github-sdli1995, the original upstream build.** Choose your GPU series first: the manager keeps a compatible scheme or selects an available one and disables unsupported choices. The table lists the schemes available in **4.2.9**, including Encore and RTX MFG 1.4.2. GPU selections are not compatibility guarantees for every game.

| Scheme | GPU series in the app | Game/API requirements and purpose | DLL entry | Configuration |
| --- | --- | --- | --- | --- |
| **0.3.5 · Github-sdli1995 — default** | RTX20 / 30 | Compatible D3D12 DLSS FG game; original upstream 310.9 model | Six choices | INI |
| **0.3.5 · Delta Force** | RTX20 / 30 / 40 | D3D12/Vulkan extension; additional multiplier controls for the identified Delta Force game | Six choices | INI |
| **Dlssg-MFG-Vulkan** | RTX20 / 30 | Compatible DLSS FG path; upstream DX12/Vulkan MFG. RTX20 remains unverified; Dynamic requires compatible DX12 | `version.dll` | Separate INI protocol |
| **0.2.6 · DX12/Vulkan** | RTX20 / 30 | Retained compatibility option for supported DLSS FG games, using the 310.1 model | Five choices | INI |
| **Initial · GitHub first release** | RTX20 / 30 | Earlier basic integration for compatible games; a fallback with fewer controls | `version.dll` | INI |
| **RTX40 MFG · 1.4.2** | RTX40 only | Requires existing Streamline DLSS FG. Vulkan is experimental; Dynamic is DX12 only | One of 19 names; universal DLL renamed by the manager | `RTXMFG-Universal.json` |
| **RTX Encore · 1.0.0-beta.2** | RTX20 / 30 / 40 | FG, DLSS SR, NR and Smooth Motion have separate requirements; menu support does not imply support for every feature | One of 19 names for the same signed universal DLL | `rtx-encore.jsonc` (v4) |

- **Six-entry schemes:** `version.dll`, `winmm.dll`, `dinput8.dll`, `dbghelp.dll`, `dxgi.dll`, `d3d12.dll`. Start with one, usually `version.dll`; `dxgi.dll` and `d3d12.dll` cannot be selected together. The five 0.2.6 entries are version / winmm / dinput8 / winhttp / dxgi.
- **Encore's 19 names:** `version.dll`, `dinput8.dll`, `winmm.dll`, `dxgi.dll`, `d3d9.dll`, `d3d10.dll`, `d3d11.dll`, `d3d12.dll`, `dsound.dll`, `wininet.dll`, `winhttp.dll`, `binkw64.dll`, `bink2w64.dll`, `xinput1_1.dll`, `xinput1_2.dll`, `xinput1_3.dll`, `xinput1_4.dll`, `xinput9_1_0.dll`, `xinputuap.dll`. Select one. Keep original Bink files alongside as `binkw64Hooked.dll` / `bink2w64Hooked.dll`; these are names for the originals, not extra entries. The manager does not rename, overwrite or delete them. Fixed 5X/6X remain experimental.
- **Keep schemes separate:** INI, RTX40 JSON, legacy Transfusion JSONC and Encore JSONC v4 are different protocols. Follow the upgrade procedure above for old Transfusion; exit the game and uninstall the old patch before other scheme or entry changes. Do not stack frame-generation patches.

The scheme name indicates its origin, not the download route. **Project source** in the scheme menu opens the relevant upstream project. Choosing a scheme does not install it, and the manager cannot add frame generation to an arbitrary game.

## Get started in four steps

1. **Run the EXE** and accept the Windows administrator prompt. Cancelling stops startup.
2. **Enable hardware-accelerated GPU scheduling** through the home page's **Graphics settings** shortcut, then restart Windows if requested.
3. **Exit the game and add its actual EXE** by scanning or manual selection. Choose the GPU series, scheme and DLL entry, open **Presets** on the game row, then **Install and apply**. A row click selects the current game; checkboxes select batch targets.
4. **Start the game and enable DLSS Frame Generation.** Exit before changing settings. With the same scheme already installed, **Apply to current game** updates its parameters without downloading the DLL again.

**For Delta Force:** add `DeltaForceClient-Win64-Shipping.exe` inside `Binaries/Win64`, choose **0.3.5 · Delta Force**, then select Follow game / 2X / 3X / 4X in Presets; 4X is the default. Its 3X/4X components use a separate cache and do not overwrite the game's original `sl.*.dll` files. This special path applies only to the identified game.

## Themes, folders and maintenance

**Themes.** Open the palette in the left sidebar to search and scroll through all 38 offline color schemes. Hover or use arrow keys to preview; click/Enter to save. Esc or a click outside restores the previous theme. System mode remembers light and dark choices separately.

**Game folders.** Select a game and use **Add folder** for a plugin destination that need not contain an EXE. It remains associated with the real game for process checks, settings and removal; third-party loaders still need their own setup. Grouped games may list several deployment directories, so review the displayed paths before installing. Manager data lives in `%LOCALAPPDATA%\RTXFGManager`.

**Downloads and updates.** Update checks read a public static manifest without the rate-limited Gitee Release API. Gitee is the default first route, with GitHub fallback; an explicit GitHub preference is remembered. Downloads show progress and speed, and verified DLL caches can be reused offline. App updates verify size, SHA-256 and publisher signature and require confirmation before replacement. Updating the manager preserves the library and preferences but does not replace game DLLs automatically. Settings also includes current and historical release notes for offline reading.

**Safe removal.** Exit the game and use **Uninstall patch**. The manager removes recognized files belonging to that deployment while preserving original game files, unknown files and other mods. Locked files leave a retry notice; close the relevant processes and retry. Removing a game from the library is not uninstalling its patch, and **Clear cache** does not delete the library or preferences.

## FAQ and limits

- **A scanned game is not a compatibility verdict.** Results depend on its frame-generation integration, graphics API, driver and GPU. A requested multiplier does not guarantee the same FPS increase or unchanged latency; no performance gain is promised.
- **The reported RTX3060 6GB VRAM issue remains unresolved in 4.2.9.** RTX40 selection does not establish compatibility or performance across all hardware.
- **RTX40 MFG's Backspace menu remains English.** Use the manager's localized presets for multiplier, Dynamic target, UI preset, VSync and fixed-mode Reflex FPS limit. Bink entries require the original Hooked file described by the upstream project.
- **Digital signatures do not grant anti-cheat approval.** Follow the game's rules and check whether third-party patches are allowed. Administrator access provides file permissions; it does not change those rules.
- **Different configuration needs different help.** The **?** beside presets explains the selected scheme. Keep its defaults when unsure, and read [version history](CHANGELOG.md) for release-specific changes.

## Open source and credits

The Rust manager is available under [MIT](LICENSE), with [build instructions](BUILDING.md) and a [cloud maintenance guide](docs/cloud-publishing.md). Game DLLs retain their own licenses; private DLL patch sources and NVIDIA SDK headers are not included. See [third-party notices](THIRD_PARTY_NOTICES.txt).

Thanks to [sdli1995 / dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86), [pipotoufikxyz-lgtm / MFG](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version), [dashdogy / RTX40MFG-Unlock](https://github.com/dashdogy/RTX40MFG-Unlock), [SilyNoMeta / RTX Encore (formerly DLSSG-Transfusion)](https://github.com/SilyNoMeta/rtx-encore), [GPUI](https://gpui.rs/), [GPUI Kit](https://github.com/longbridge/gpui-kit), [GPUI Fast](https://github.com/longbridge/gpui-fast) and [aria2](https://github.com/aria2/aria2).

Thanks also to [大大大怪将军阁下](https://space.bilibili.com/608531525), [云外逸声](https://space.bilibili.com/256887068) and everyone sharing compatibility feedback. This project is not affiliated with NVIDIA, game publishers or the upstream projects.

[Website](https://lgxng.cn/) · [Bilibili](https://b23.tv/5mHCHFn) · [GitHub](https://github.com/pandaligx/RTX-FG-Manager) · [Gitee](https://gitee.com/pandaligx/RTX-FG-Manager)
