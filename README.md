# RTX Frame Generation Manager · by小南瓜

[English](README.md) · [简体中文](README.zh-CN.md)

<p align="center">
  <img src="docs/screenshot-home.png" alt="RTX Frame Generation Manager home screen" width="820" />
</p>
<p align="center">
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="release" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="downloads" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="platform" src="https://img.shields.io/badge/platform-Windows_x64-0078D6?logo=windows&logoColor=white">
  <img alt="language" src="https://img.shields.io/badge/language-Python-3776AB?logo=python&logoColor=white">
  <a href="LICENSE"><img alt="manager license" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

A Windows desktop tool for managing RTX 20/30 frame-generation compatibility patches. This is a **binary distribution repository**: releases, documentation, screenshots, update metadata and publishing automation. Manager source code is not published here. Third-party components retain their own licenses.

## Download

- [GitHub Releases](https://github.com/pandaligx/RTX-FG-Manager/releases/latest)
- [Gitee Releases · China mirror](https://gitee.com/pandaligx/RTX-FG-Manager/releases)

Download the signed `RTXManager-v<version>-x64.exe` from a published release and run it. Python and aria2 do not need a separate installation. Release notes describe the changes and any known limitations. A release is available only after its EXE and update manifest have been uploaded; development candidates are not releases.

## Features

- Chinese, English, Russian, Japanese and Korean. First launch follows Windows language; a manual choice is saved. Change it with the globe button or in Settings.
- Left-side icon navigation with tooltips, light/dark/system themes and adaptable window layout.
- Add game EXEs or scan a folder, drive or all drives. Game library and operation log stay together.
- Native 0.2.4 Fix1 and the legacy R2/SM86 modes. Native supports `version.dll`, `winmm.dll`, `dinput8.dll`, `winhttp.dll` and `dxgi.dll`. Default: Native test mode with `version.dll`.
- Install and remove recognized project files, retaining unrelated or unidentified game files. INI edits do not prevent normal cleanup. Incomplete operations show a warning and remain in the log.
- Background update checks and optional automatic downloads through embedded aria2. Installation needs confirmation. Download integrity and the manager publisher's Windows signature are verified before replacement; the previous EXE is retained as a rollback copy.

## Quick start

1. Completely exit the game. Add its actual EXE using **Add game**, or use the **Scan** arrow.
2. Select the game, choose its GPU series and compatibility mode, and install the patch.
3. Start the game and enable its DLSS frame-generation option. Closing the manager does not remove the patch.
4. To change mode or DLL entry, exit the game, remove the existing patch, then install again. Test one DLL entry at a time; multiple entries may conflict.
5. Use **Remove patch** to uninstall. If a game is still running, removal has **not** completed: close it and retry. See **Help** on the home page for more details.

Compatibility depends on the game, GPU and driver. A game appearing in the library does not prove that frame generation will work. The GPU-name feature changes Windows display-name fields only; it does not change hardware capabilities or guarantee a name change in games/Task Manager. Reopen the relevant windows or restart Windows if necessary.

## Updates and preferences

Automatic checks run after startup and every six hours while the app is open. Automatic download is optional and off by default. In automatic source mode, **Windows region China** prefers Gitee; other regions prefer GitHub. You can override this in Settings. If a mirror is unavailable or behind, the updater can use the other site's verified release. This uses the system region, not IP geolocation.

Settings and the game list are stored in `%LOCALAPPDATA%\RTXFGManager`. Updating the manager preserves these preferences and does not automatically replace patches already installed in games. Updates never silently close a game or restart Windows.

## Credits and links

- [dlssg_for_sm86 · sdli1995](https://github.com/sdli1995/dlssg_for_sm86)
- [aria2](https://github.com/aria2/aria2) · separate downloader, GPL-2.0-or-later
- [Website](https://lgxng.cn/) · [GitHub](https://github.com/pandaligx) · [Bilibili](https://b23.tv/5mHCHFn)

See [third-party notices](THIRD_PARTY_NOTICES.txt) for component attribution and license boundaries. The manager's MIT license does not relicense NVIDIA or other third-party material. This project is not affiliated with NVIDIA or the credited upstream projects.
