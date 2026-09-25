# RTX Frame Generation Manager · by 小南瓜

**An easier way to install, tune and manage frame-generation patches for RTX 20 / 30 GPUs.**

A Rust core with a native GPUI interface. One executable for your game library, cloud DLLs, per-game presets and patch removal.

**English** · [简体中文](README.zh-CN.md) · [Download on GitHub](https://github.com/pandaligx/RTX-FG-Manager/releases/latest) · [Gitee mirror](https://gitee.com/pandaligx/RTX-FG-Manager/releases)

<p align="center">
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="release" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="downloads" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?logo=windows&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-GPUI-DEA584?logo=rust&logoColor=white">
  <a href="LICENSE"><img alt="manager license MIT" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

<p align="center">
  <img src="docs/screenshot-home.png" alt="RTX Frame Generation Manager: light theme on the left, dark theme on the right" width="980" />
</p>

<p align="center">Composite theme preview based on the actual interface with demo game entries. System theme is also supported.</p>

## What the manager handles

| Your task | Built-in support |
| --- | --- |
| Find games and deploy patches | Folder, drive and full-drive scans grouped by installation; manual EXE selection; individual or batch deployment with visible scheme and DLL tags |
| Get the right DLL | On-demand cloud downloads, domestic-first routing with GitHub fallback, integrity checks and reusable offline cache |
| Adjust compatibility and quality | Scheme-specific presets, separate settings for each game and scheme, and contextual **?** help |
| Update or remove a patch | Startup update checks, resumable aria2 downloads, speed and compact circular progress; ownership-based cleanup and clear retry notices |
| Use it every day | Five languages, light/dark themes, DPI-aware layout, background tasks and a visible operation log |

**Portable EXE. No Python, Rust, CUDA Toolkit or separate aria2 installation required.** Manager releases and project-distributed DLLs are digitally signed. The Rust manager source, build instructions and verified cloud maintenance workflow are published here. Third-party components retain their respective licenses.

## What this project adds to upstream

The original frame-generation implementation comes from [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86). The manager keeps both the upstream build and this project's extensions available, so you can select or revert per game.

### 0.3.5 · Delta Force extension

- **Vulkan integration:** resource interop, GPU-shared transfers, synchronization and compatible fallback paths. The bridge is merged into all six proxies; no separate bridge DLL is needed.
- **Less transfer overhead:** shared-resource and cache reuse reduce CPU image transfers, repeated allocations and waits, while retaining upstream's **310.9 model and inference optimizations**.
- **Delta Force RTX20 / CMP40HX compatibility:** eligibility adjustments for identified PCI IDs are scoped to the specified game's main module. Real CUDA architecture, VRAM, LUID and registry remain unchanged.
- **Delta Force multiplier control:** follow game, 2X, 3X or 4X in the manager. The 3X/4X path uses matching private runtime components and **does not overwrite the game's original `sl.*.dll` files**.
- **Owned per-game cleanup:** independent versioned caches, with removal of identified DLLs, INIs, logs and runtime components while preserving unknown and original game files.

### Retained 0.2.6 · DX12/Vulkan option

Based on upstream Native 0.2.4 and the **310.1 model**. It includes the Naraka typeless-texture Fix1, Vulkan interop and transfer improvements, plus deferred D3D12 loading and lifecycle fixes for the Where Winds Meet startup regression. **0.2.6 is this project's scheme version.**

Users have reported working configurations in Naraka, Delta Force, Arknights: Endfield and Where Winds Meet, including increased FPS in the RTX2070 Delta Force test. Results depend on hardware, drivers and game versions. **4X does not guarantee four times the FPS or unchanged latency.**

## Dlssg-MFG-Vulkan

An independent integration of [pipotoufikxyz-lgtm's sm86-7 release](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version/releases/tag/sm86-7), with one signed `version.dll`. Upstream announces Vulkan support, including RTX Remix examples, but explicitly leaves RTX20 unconfirmed. This is not universal Vulkan compatibility, Smooth Motion, or this project's Delta Force integration.

Its separate INI protocol uses `MaxInterpolatedFrames=5` (up to 6X) and `ForceMultiplier=0` (follow game). Presets expose requested 2X–6X, dynamic MFG and target FPS, four optimizations and an on/off log switch. Dynamic MFG requires a compatible DX12 path; requests do not prove actual presentation. Upstream optimization defaults are retained and may affect image quality.

RenderScale, hotkeys and other advanced keys are preserved. Blackwell experimental toggles with upstream GPU-hang reports are not exposed. Managed edits preserve comments and unrelated INI keys; cleanup preserves unknown files. Exit the game and uninstall before switching schemes.

## Choose a scheme

| Scheme | API and purpose | Entry DLLs |
| --- | --- | --- |
| **0.3.5 · Github-sdli1995 — default** | Original upstream, D3D12 / SM75 and SM86, 310.9 model | Six |
| **0.3.5 · Delta Force** | This project's Vulkan and Delta Force extensions | Six |
| **Dlssg-MFG-Vulkan** | Upstream sm86-7 adds Vulkan; RTX20 unverified | version.dll |
| **0.2.6 · DX12/Vulkan** | Retained compatibility option, 310.1 model | Five |
| **Initial · GitHub first release** | Original capabilities; files selected for RTX20 or RTX30 | version.dll |

**The original GitHub scheme remains the initial default; manual choices are remembered.** The scheme's name describes its origin, not its download route. A GitHub scheme can still be downloaded from the domestic server.

The six entries are `version.dll`, `winmm.dll`, `dinput8.dll`, `dbghelp.dll`, `dxgi.dll` and `d3d12.dll`. Start with one, preferably `version.dll`. Multiple selection is supported; the first loaded proxy leads and the others forward calls. Choose either `dxgi.dll` or `d3d12.dll`, not both. The 0.2.6 entries are version / winmm / dinput8 / winhttp / dxgi.

Upstream 0.3.5 fixes optimized-kernel selection after feature recreation, which could cause corruption or crashes. See the [upstream release notes](https://github.com/sdli1995/dlssg_for_sm86/releases/tag/0.3.5). That fix belongs to upstream and is distinct from this project's Vulkan additions.

## Delta Force: 2X / 3X / 4X

Use **Manager 4.2.1 or later** and exit the game first:

1. Add the actual `DeltaForceClient-Win64-Shipping.exe` inside `Binaries/Win64`.
2. Select **0.3.5 · Delta Force**, choose the GPU series and expand **Preset parameters**.
3. Set the multiplier to follow game / 2X / 3X / **4X (default)**, then install/apply.
4. Start the game and enable its frame-generation switch. Exit before applying another multiplier.

The manager updates the same INI automatically. Follow game and 2X retain the original runtime; 3X and 4X use the independent cache. If original components are already loaded, their runtime is retained to avoid mixed versions. **Other games do not enter this special path.** Their generic multiplier is a ceiling and cannot add new menu options.

## Get started

1. Download `RTXManager-v<version>-x64.exe` and run it on Windows 10 / 11 x64. Allow permissions required for the requested operation.
2. Use the home page's **Graphics settings** button. Under Windows Settings → System → Display → Graphics → Change default graphics settings, enable **Hardware-accelerated GPU scheduling** and restart if requested. Labels vary between Windows versions.
3. Exit the game, scan or add its actual EXE, select the GPU series, scheme and entry DLL, then install.
   Verified game executables from one installation appear as one entry. If they occupy separate directories, installation and removal cover every listed directory. Launchers and anti-cheat programs are excluded from automatic deployment; add a missed game EXE manually.
4. Enable DLSS frame generation in the game. Clicking a row selects the current game; checkboxes select batch targets.
5. Uninstall the old patch before changing patch version, scheme or entry DLL. Updating the manager does not automatically replace DLLs already installed in games.

Settings are remembered per **game + scheme**. The 0.3.5 presets include kernel mode, multiplier, UI recomposition and logging; 0.2.6 exposes multiplier, sampling and logging; Initial exposes enablement, multiplier and logging. Updating an identified deployment's presets preserves unrelated INI content and comments. The 0.2.6 Vulkan bridge does not yet follow the INI logging level; detailed 0.3.5 bridge logs do, while limited startup diagnostics remain separate.

## Cleanup, caching and updates

Both 0.3.5 schemes retain the upstream log directory and empty CacheDirectory (user cache). If an older installation crashes in Zenless Zone Zero, exit the game, select the same scheme and d3d12.dll, then install again. Only former manager paths are repaired; custom paths remain. Shared runtime caches are not removed when uninstalling one game.

- **Uninstall:** removes files identified as belonging to this project, including edited INIs, multiple proxies and recognized re-signed components. A game-running warning means the operation has not completed.
- **Pending cache cleanup:** locked caches keep a retry record and an explicit partial-completion message. Close the relevant processes and retry uninstall or **Clear cache** in Settings.
- **Preserved data:** original game files, unknown files, the game list and preferences are retained. Old test-3 restoration backups are not removed automatically. Removing a library entry is not uninstalling its patch.
- **Independent storage:** Delta Force components use `%LOCALAPPDATA%\RTXFG-Delta4X\games\<game-id>\<version>`. Manager data uses `%LOCALAPPDATA%\RTXFGManager`.
- **Cloud resources:** mirrored catalogs and DLL packages on Gitee and GitHub. Gitee is the default first source; failures show a reason before trying GitHub. aria2 handles downloads with size, speed and progress. Verified cache entries are reusable.
- **App updates:** checks run in the background on startup. Downloads are checked for size, SHA-256 and publisher signature, then replace and restart the executable using its new versioned name. The old EXE is removed after successful UI startup; failed startup restores it. Automatic downloading only prefetches a release; applying it still requires confirmation.

Patches, catalogs and app updates share one download preference. New installations default to Gitee first; an explicit GitHub preference is remembered. Existing game libraries, preferences and deployment records remain compatible with upgrades from 3.7.4.

## Compatibility notes

Frame generation depends on the game, driver, OS and GPU. Finding a game during scanning is not a compatibility verdict. Follow each game's rules, especially where anti-cheat is involved. GPU renaming changes Windows display-name fields, not hardware capability, and a game may read its device name differently.

Chinese, English, Russian, Japanese and Korean are supported, with system-language detection and remembered manual choices. See [Release notes](https://github.com/pandaligx/RTX-FG-Manager/releases/latest) for version-specific changes.

## Credits and links

[Github · sdli1995](https://github.com/sdli1995/dlssg_for_sm86) · [Community extension · pipotoufikxyz-lgtm](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version) · [GPUI](https://gpui.rs/) · [GPUI Component](https://github.com/longbridge/gpui-component) · [aria2](https://github.com/aria2/aria2)

Thanks to [大大大怪将军阁下 on Bilibili](https://space.bilibili.com/608531525) for testing, and to users who provide compatibility feedback.

[Website](https://lgxng.cn/) · [GitHub · pandaligx](https://github.com/pandaligx) · [Bilibili](https://b23.tv/5mHCHFn) · [Third-party notices](THIRD_PARTY_NOTICES.txt)

Not affiliated with NVIDIA, game publishers or the upstream project. The manager's license does not change third-party rights. If this tool helps you, a **Star** is welcome.

Thanks also to [Bilibili · 云外逸声](https://space.bilibili.com/256887068).

## Parameters, source and maintenance

Multiplier controls are visible immediately, while other controls stay under **Advanced parameters**. Changes are marked as pending. **Apply to current game** updates only that game’s INI when the same scheme is installed, without downloading DLLs; **Install and apply** also supports checked batches. Exit the game before applying.

The Rust manager source is published under the repository license. [BUILDING.md](BUILDING.md) documents the pinned toolchain, verified third-party tool and checks for a fresh Windows checkout. Game DLLs retain their respective upstream ownership; private DLL patches and NVIDIA SDK headers are not part of this source release.

Maintain cloud schemes in [cloud/schemes.json](cloud/schemes.json). Add immutable assets to the fixed GitHub `payloads` prerelease; automation mirrors them to the separate Gitee resource repository, preserving the manager's latest-release endpoint for older clients. It verifies downloads at both hosts, then publishes the index and finally `cloud/catalog.json`. No private-drive uploads are needed. See [cloud maintenance](docs/cloud-publishing.md). Keep old assets available for older clients.
