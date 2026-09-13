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
- Background update checks, optional automatic downloads through bundled aria2, checksum and publisher-signature verification, confirmed installation and a backup of the previous EXE.

## Frame-generation schemes

| Scheme | Graphics API | Maximum multiplier | Proxy choices |
| --- | --- | --- | --- |
| **0.2.6 Stable (default)** | DX12 / Vulkan | 4X | Five; version.dll selected by default |
| **0.2.6 5X/6X Experimental** | DX12 / Vulkan | 6X | Five; version.dll selected by default |
| Initial · First GitHub version | Original R2/SM86 capabilities | Depends on the original scheme/game | Only the supplied entries are shown |

The five proxies are `version.dll`, `winmm.dll`, `dinput8.dll`, `winhttp.dll` and `dxgi.dll`. Test one at a time; selecting several can conflict and does not guarantee better compatibility. Each proxy includes the bridge, so no external `rtxfg_vk_bridge.dll` is needed. RTX 20 uses SM75; RTX 30 uses SM86.

6X is a capability limit requested by the game; it **does not add game-menu options**. Naraka's menu remains limited to 4X even with the experimental scheme.

## Changes to the upstream DLLs

Based on [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86) Native 0.2.4. **0.2.6 is this project's compatibility-scheme version, not an upstream release number.**

- **Fix1 texture compatibility:** correct typeless SRV/UAV view formats, addressing the reproduced Naraka crash when entering a match with frame generation enabled.
- **Vulkan integration:** Vulkan resources, DX12 backend interoperability and synchronization, integrated into all five stable and experimental proxies.
- **Transfer/scheduling improvements:** GPU shared transfers, resource reuse and combined submissions reduce CPU image staging, allocation and waiting; a fallback remains for unavailable sharing capabilities.
- **0.2.6 startup compatibility:** load D3D12 on demand to avoid premature access to uninitialized host Agility SDK information, addressing the Where Winds Meet startup regression. Process-lifetime protection also addresses immediate-unload crashes found in isolated tests.
- **Experimental multipliers:** combine the community 5X/6X extension with Fix1, Vulkan and the new startup changes. The initial GitHub scheme remains unchanged.

The **310.1 model and native inference kernels are unchanged**. The scheme number does not mean a newer NVIDIA model. No fixed FPS or universal game compatibility is promised. See [Release notes](https://github.com/pandaligx/RTX-FG-Manager/releases/latest) for changes, validation scope and outstanding issues.

## Quick start

1. Enable Hardware-accelerated GPU scheduling in Windows Settings → System → Display → Graphics → Change default graphics settings. Labels vary by Windows version. Use the app's Graphics settings shortcut and restart when Windows requests it.
2. Exit the game completely, then add its actual EXE or select a scan range using the arrow beside Scan. Manual library addition accepts more EXE architectures; installing an x64 patch still requires a compatible executable.
3. Select the game row, GPU series, scheme and one proxy, then install. For batches, check the games and review the numbered list.
4. Launch the game and enable its DLSS frame generation and supported multiplier. Closing the manager does not disable the deployed patch.
5. Exit the game and uninstall before switching schemes or entries. A running-game warning means the uninstall has not completed.

Cleanup recognizes known project files, edited INIs and re-signed known DLLs, while retaining unrelated and unknown files. **Updating the manager does not update DLLs already installed in games.** Exit the game and redeploy them separately.

GPU renaming only changes Windows display-name fields, not the actual GPU or CUDA capabilities. Games may read other names; a restart may be required. Device changes and uncertain backups remain protected. Follow game rules and confirm whether third-party patches are permitted, especially with anti-cheat software.

## Upgrading from 3.7.4

Upgrade directly to 4.0.8; intermediate builds are not required. Existing games, preferences and deployment records are retained. Old deployment badges are not falsely relabeled as 0.2.6. Diagnostic and compatibility-self-test panels have been removed; the activity log remains.

Preferences and games are stored in `%LOCALAPPDATA%\RTXFGManager`. Change language, theme and update source in Settings. Automatic checks are enabled by default; automatic downloads are off. Automatic routing uses the **Windows system region**: Gitee first for China, GitHub elsewhere, with manual selection and fallback for unavailable/outdated endpoints. This is not IP geolocation.

## Credits and contact

- [Github · sdli1995](https://github.com/sdli1995/dlssg_for_sm86): upstream project.
- [pipotoufikxyz-lgtm](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86): community 5X/6X extension.
- [GPUI](https://gpui.rs/) · [GPUI Component](https://github.com/longbridge/gpui-component): native UI and components.
- [aria2](https://github.com/aria2/aria2): independent downloader.
- [Bilibili · 大大大怪将军阁下](https://space.bilibili.com/608531525): testing assistance.
- [Website](https://lgxng.cn/) · [GitHub](https://github.com/pandaligx) · [Bilibili](https://b23.tv/5mHCHFn).

See [third-party notices](THIRD_PARTY_NOTICES.txt). The manager license does not relicense NVIDIA or other third-party material. This project is not officially affiliated with NVIDIA or the upstream projects.
