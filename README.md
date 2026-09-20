# RTX Frame Generation Manager · by小南瓜

[English](README.md) · [简体中文](README.zh-CN.md)

The screenshot shows the Chinese interface; English and three other UI languages are available in Settings.

<p align="center">
  <img src="docs/screenshot-home.png" alt="RTX Frame Generation Manager home screen" width="820" />
</p>
<p align="center">
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="release" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="downloads" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="platform" src="https://img.shields.io/badge/platform-Windows_x64-0078D6?logo=windows&logoColor=white">
  <img alt="language" src="https://img.shields.io/badge/language-Rust-DEA584?logo=rust&logoColor=white">
  <a href="LICENSE"><img alt="manager license" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

A Windows frame-generation patch manager for RTX 20/30 GPUs, built with a **Rust core and native GPUI interface**. This is a **binary distribution repository** containing releases, documentation, screenshots and update assets. Manager source code is not published; third-party components retain their own licenses.

## Download

- [GitHub Releases](https://github.com/pandaligx/RTX-FG-Manager/releases/latest)
- [Gitee Releases — China mirror](https://gitee.com/pandaligx/RTX-FG-Manager/releases)

Run the signed `RTXManager-v<version>-x64.exe` directly on Windows 10/11 x64. No Python, Rust, CUDA Toolkit or separate aria2 installation is required. Frame-generation compatibility depends on the GPU, driver and game.

## Features

- Chinese, English, Russian, Japanese and Korean; follows the system language initially and remembers manual selection in Settings. Light, dark and system themes.
- Rust background tasks for scanning, deployment, system operations and updates; native GPUI components/icons, centered launch, scaling-aware layout and higher-resolution executable icons.
- Directory, drive and full-drive scans, plus manual EXE selection. Expanded Unreal, Unity and DLSS component discovery; discovery does not establish frame-generation compatibility.
- Deployment badges in the game library and a persistent activity log. Clicking a row selects the current game; independent checkboxes select batch targets.
- Single/batch installation, uninstall and removal from the library. Right-click to open a folder or remove a game; double-click to open its folder. Removing a library entry does not uninstall its patch.
- Single-game deployment avoids redundant confirmations; batch confirmation lists numbered targets. Errors and running-game blockers produce clear warnings.
- Change/restore Windows GPU display names, with backup migration after driver updates on the same device, fresh driver-value preservation and rollback protection.
- Help in five languages, shown on first use, and a Graphics settings shortcut. No recurring HAGS detection popup when selecting games.
- Startup update checks with a centered new-version prompt. Bundled aria2 provides resumable downloads, a compact ring, size, speed and ETA. After choosing Download and update, the verified file replaces the app in its current folder under the new release filename and restarts it, deleting the old EXE after the new UI starts successfully. Optional automatic downloads only prefetch; replacement still requires accepting the update.

## Cloud DLL resources

Game DLLs are downloaded on demand for the selected scheme and proxy instead of embedded in the EXE. The China HTTPS file server is tried first. Bundled aria2 handles parallel transfers and download-host redirects, including a China-mirror compatibility retry before GitHub fallback. Verified cached files work offline. Files are fully validated before deployment; uninstall does not require a network connection.

The catalog can update scheme names, versions, DLLs and INIs independently when using a supported deployment protocol. Updating the manager does not replace patches already installed in games: uninstall the previous patch before installing another version.

## Schemes and preset settings

| Scheme | Graphics API | Parameters and proxies |
| --- | --- | --- |
| **0.3.5 · Github-9.20 (default)** | D3D12, SM75/SM86 | Six selectable proxies; default 4X, up to 6X where the game supports it |
| **0.2.6 · DX12/Vulkan** | DX12 / Vulkan | This project's stable scheme; five proxies, up to 4X |
| **Initial · First GitHub release** | Original R2/SM86 capabilities | One scheme selects the correct version.dll and INI for RTX20/RTX30 |

Upstream 0.3.5 uses model 310.9 and fixes selection of the wrong optimized kernel after feature recreation, which could corrupt frames or crash. It includes the 0.3.4 RTX30 architecture-reporting fix and the four optimization tiers introduced in 0.3.2. `Optimized=1` defaults to bit-exact optimization; 0 uses stock numerics, while 2/3 trade image quality for additional speed. These are [upstream findings](https://github.com/sdli1995/dlssg_for_sm86/releases/tag/0.3.5), not a claim that this project retested every game.

Upstream offers `version.dll`, `winmm.dll`, `dinput8.dll`, `dbghelp.dll`, `dxgi.dll` and `d3d12.dll`. Prefer the first four; use the last two when needed. Only the first loaded proxy is active, with other proxies forwarding. Upstream does not include this project's Vulkan extension. The five 0.2.6 proxies are `version.dll`, `winmm.dll`, `dinput8.dll`, `winhttp.dll` and `dxgi.dll`; prefer one at a time. No external bridge DLL is needed.

The **Preset settings** accordion shows parameters for the chosen scheme; **?** opens contextual help. Upstream exposes kernel tiers, multiplier limit, UI recomposition and logs. Native 0.2.6 exposes multiplier, sampling and logs; Initial exposes enable, multiplier and logs. Choices are remembered per game and scheme; defaults are recommended. Exit the game and click Install to apply. If the same DLLs are installed, only managed keys change and other INI content is preserved. **A multiplier limit does not add game menus or guarantee proportional FPS.** The 0.2.6 Vulkan bridge logs remain independent of this setting.

Downloads default to China first, with GitHub first available. The fixed catalog endpoint is `https://www.lgxng.cn/1814328088/g/new/catalog.json`. A concise catalog selects schemes and defaults; a versioned index supplies integrity metadata. Future releases using the same deployment and parameter protocols can update through the cloud alone. New protocols still require a manager update. Older clients retain their valid cached catalog; upgrading is recommended.

Settings can clear downloaded DLLs and update staging while preserving the library, settings and cleanup ownership records. DLLs must be downloaded again before offline installation. App updates delete the old EXE once the new UI starts successfully and restore it on failure; unrelated executables in the same folder are not scanned or removed.

## Changes to the upstream DLLs

Based on [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86) Native 0.2.4. **0.2.6 is this project's compatibility-scheme version, not an upstream release number.**

- **Fix1 texture compatibility:** correct typeless SRV/UAV view formats, addressing the reproduced Naraka crash when entering a match with frame generation enabled.
- **Vulkan integration:** Vulkan resources, DX12 backend interoperability and synchronization, integrated into all five stable proxies.
- **Transfer/scheduling improvements:** GPU shared transfers, resource reuse and combined submissions reduce CPU image staging, allocation and waiting; a fallback remains for unavailable sharing capabilities.
- **0.2.6 startup compatibility:** load D3D12 on demand to avoid premature access to uninitialized host Agility SDK information, addressing the Where Winds Meet startup regression. Process-lifetime protection also addresses immediate-unload crashes found in isolated tests.

For this project's 0.2.6 schemes, the **310.1 model and native inference kernels are unchanged**. The scheme number does not mean a newer NVIDIA model. No fixed FPS or universal game compatibility is promised. See [Release notes](https://github.com/pandaligx/RTX-FG-Manager/releases/latest) for changes, validation scope and outstanding issues.

**User-tested compatibility:** RTX20-series GPUs can now enable frame generation in Where Winds Meet; the previously missing option is confirmed resolved by the user. This feedback does not validate every GPU, driver or game version.

## Quick start

1. Enable Hardware-accelerated GPU scheduling in Windows Settings → System → Display → Graphics → Change default graphics settings. Labels vary by Windows version. Use the app's Graphics settings shortcut and restart when Windows requests it.
2. Exit the game completely, then add its actual EXE or select a scan range using the arrow beside Scan. Manual library addition accepts more EXE architectures; installing an x64 patch still requires a compatible executable.
3. Select the game row, GPU series, scheme and one proxy, then install. For batches, check the games and review the numbered list.
4. Launch the game and enable its DLSS frame generation and supported multiplier. Closing the manager does not disable the deployed patch.
5. Exit the game and uninstall before switching schemes or entries. A running-game warning means the uninstall has not completed.

Cleanup recognizes known project files, edited INIs and re-signed known DLLs, while retaining unrelated and unknown files. **Updating the manager does not update DLLs already installed in games.** Exit the game and redeploy them separately.

GPU renaming only changes Windows display-name fields, not the actual GPU or CUDA capabilities. Games may read other names; a restart may be required. Device changes and uncertain backups remain protected. Follow game rules and confirm whether third-party patches are permitted, especially with anti-cheat software.

## Upgrading from 3.7.4

Upgrade directly to the latest release; intermediate builds are not required. Existing games, preferences and deployment records are retained. Old deployment badges are not falsely relabeled as 0.2.6. Diagnostic and compatibility-self-test panels have been removed; the activity log remains.

Preferences and games are stored in `%LOCALAPPDATA%\RTXFGManager`. Change language, theme and update source in Settings. Automatic checks are enabled by default; automatic downloads are off. Automatic routing uses the **Windows system region**: Gitee first for China, GitHub elsewhere, with manual selection and fallback for unavailable/outdated endpoints. This is not IP geolocation.

## Credits and contact

- [Github · sdli1995](https://github.com/sdli1995/dlssg_for_sm86): upstream project.
- [pipotoufikxyz-lgtm](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86): community 5X/6X extension.
- [GPUI](https://gpui.rs/) · [GPUI Component](https://github.com/longbridge/gpui-component): native UI and components.
- [aria2](https://github.com/aria2/aria2): independent downloader.
- [Bilibili · 大大大怪将军阁下](https://space.bilibili.com/608531525): testing assistance.
- [Website](https://lgxng.cn/) · [GitHub](https://github.com/pandaligx) · [Bilibili](https://b23.tv/5mHCHFn).

See [third-party notices](THIRD_PARTY_NOTICES.txt). The manager license does not relicense NVIDIA or other third-party material. This project is not officially affiliated with NVIDIA or the upstream projects.
