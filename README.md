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

## 4.3.0 candidate

The following describes the candidate under acceptance testing. The signed stable client remains 4.2.9 above. Version 4.3.0 will be released after combined acceptance and publisher signing.

| Patch | Update | In-game menu |
| --- | --- | --- |
| **dlssg_for_sm86 0.4.0 (default)** | Full component install including SR; migration from 0.3.5; six distinct proxy files, one entry at a time | INI / manager settings |
| **MFG Unlocker 1.5.0** | Preset C; mutually exclusive standalone and existing OptiScaler 10 plugin modes | Standalone: **Backspace**; plugin: OptiScaler menu |
| **RTX Encore 1.0.0-rc.3** | Separate SR/RR/FG presets, dynamic FG and Smooth Motion controls; dedicated proxies and ASI use their own files | **Insert** |

Delta Force, custom Vulkan and 0.2.6 remain available. The first-release scheme is retired; existing installations can still be recognized and removed safely. GPU detection is advisory; scheme selection and deployment are unrestricted. Legacy variants still use the selected RTX20/RTX30 component. Selection does not establish hardware, driver or game compatibility.

Copying or renaming an unchanged signed DLL preserves its signature. Encore's `version.dll`, `winmm.dll`, `dinput8.dll`, `dxgi.dll` and ASI routes use separately distributed files; other supported names use the main DLL. A signature on the main DLL does not sign those separate files. Filenames select loading routes, not additional graphics APIs.

Bink originals must already be preserved alongside the patch as `binkw64Hooked.dll` / `bink2w64Hooked.dll`; the manager does not rename or delete them automatically. MFG plugin mode requires an existing OptiScaler 10 installation and its default `OptiScaler/plugins` directory. It neither installs OptiScaler nor automatically adapts custom plugin search paths.

### Get started

1. Exit the game and add its actual rendering EXE; Unreal games commonly use `Binaries/Win64`.
2. Choose a patch and an unused entry, review settings in game details, then choose **Install and apply**.
3. Start the game, enable its supported frame-generation path and use the menu key above. Apply restart-required changes after exiting the game.

Each game has a separate **Player configurations** entry next to **Preset parameters**. Preset parameters retains **Overview, Configuration, History and recovery** tabs. Name/export local configurations, preview selected imports, and undo to exact original bytes. Conflicting later edits are preserved. Uninstall before switching schemes; supported manager-owned upgrades preserve configuration with transaction recovery. Updating the manager does not automatically replace game patches.

### HTML help and independent community

The separate HTML help window supports offline search, contents, patch switching, parameter copying and themes. Content updates independently; failures retain cached or bundled help. If WebView2 is missing, the default browser opens the same files; no runtime is installed automatically. Five UI languages remain available, with explicit English fallback for untranslated new articles.

[Online help](https://rtx-fg-community.1547245493.workers.dev/help/) uses the same static content. The help page has no game-path, filesystem or command bridge.

Open **Player configurations** to browse, apply or share current settings. The list refreshes every 30 seconds. Recommendations contain only this game and can compare different patch schemes. Cards show the patch, version and DLL / ASI entry. A different installation opens setup guidance before parameter preview; imports remain isolated by patch protocol. Local matching uses only **GPU model and driver version**, followed by working confirmations and likes. VRAM, resolution and GPU architecture are not collected. Preview differences before importing; local paths, diagnostics and default hotkeys are excluded from sharing. Review the full submission before publication. Each identity can keep five configurations per game, with updates, deletion and reversible likes, confirmations and reports. Measurements are player supplied; similarity is not a performance guarantee.

This independent service does not access RTX Encore Hub's private database. First writes require verification. Windows DPAPI protects private identity keys, with explicit encrypted backup/restore. Patch management, local configurations and offline help work during service outages. The initial community can be empty.

### Distribution and removal

GitHub remains the primary project; new domestic resources prefer GitCode and fall back to GitHub. Gitee retains the migration entry for older clients and historical resources. Source, documentation and small assets synchronize one way; this release's bulk patches use local GitCode uploads. Only complete verified cached packages are installed.

Removal cleans this game's owned components, state and known logs: dlssg frame timing/NGX, game-specific MFG temporary logs, and Encore session/crash/performance logs. Unknown files, other games/mods and manual captures remain. Busy files or incomplete recovery prompt retry. OptiScaler settings are restored only where the manager changed them and the user has not edited them afterward.

Removing a library entry or clearing download caches does not uninstall game patches. Offline and UI checks are not game/GPU performance or anti-cheat acceptance. The previously reported RTX3060 6GB VRAM issue is not claimed fixed.

## Open source and credits

The Rust manager is available under [MIT](LICENSE), with [build instructions](BUILDING.md) and a [cloud maintenance guide](docs/cloud-publishing.md). Game DLLs retain their own licenses; private DLL patch sources and NVIDIA SDK headers are not included. See [third-party notices](THIRD_PARTY_NOTICES.txt).

Thanks to [sdli1995 / dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86), [pipotoufikxyz-lgtm / MFG](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version), [dashdogy / MFG-Unlocker](https://github.com/dashdogy/MFG-Unlocker), [SilyNoMeta / RTX Encore (formerly DLSSG-Transfusion)](https://github.com/SilyNoMeta/rtx-encore), [GPUI](https://gpui.rs/), [GPUI Kit](https://github.com/longbridge/gpui-kit), [GPUI Fast](https://github.com/longbridge/gpui-fast) and [aria2](https://github.com/aria2/aria2).

Thanks also to [大大大怪将军阁下](https://space.bilibili.com/608531525), [云外逸声](https://space.bilibili.com/256887068) and everyone sharing compatibility feedback. This project is not affiliated with NVIDIA, game publishers or the upstream projects.

[Website](https://lgxng.cn/) · [Bilibili](https://b23.tv/5mHCHFn) · [GitHub](https://github.com/pandaligx/RTX-FG-Manager) · [Gitee](https://gitee.com/pandaligx/RTX-FG-Manager)
