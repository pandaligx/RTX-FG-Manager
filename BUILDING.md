# Build the Rust manager / 编译 Rust 管理器

## Linux AppImage (experimental / 实验性)

The Linux port shares the current manager source and uses a native GPUI window.
Build on Ubuntu 22.04 or newer with the dependencies listed in
`.github/workflows/build-linux.yml`, then run:

```sh
cargo build --locked --release --target x86_64-unknown-linux-gnu
WINEPREFIX=/absolute/path/to/prefix target/x86_64-unknown-linux-gnu/release/RTXManager
APPIMAGETOOL=/absolute/path/to/appimagetool packaging/build-appimage.sh
```

Use an explicit Wine/Proton prefix (`WINEPREFIX` or `--wine-prefix`) and add the
game's Windows EXE or set `RTXFG_ROOTS` to its host installation directory.
This does not install DLLs into native ELF games or configure Proton launch options.
The host supplies Vulkan drivers, X11/Wayland, FreeType, Fontconfig and xkbcommon.
Windows updates, HAGS, native GPU probes, DPAPI login storage and Windows-only
leftover/handle repair are unavailable. Linux never offers a Windows EXE update
or claims Windows Authenticode trust. Keep recovery files when an operation is
unsupported. Community browsing and portable configuration editing remain available.

Linux 构建为实验性独立 AppImage，使用当前源码；需明确选择 Wine/Proton 前缀，
不代表所有游戏已实测。不自动更新 Windows EXE，不修改宿主系统显卡驱动或 GPU 调度。
Windows 的签名、渲染、更新通道和发布工作流保持原实现。

The Linux workflow produces a temporary CI artifact only. It never signs or
publishes a release and cannot modify update manifests. Windows regressions still
run independently in `build.yml`; do not skip them when maintaining this port.

## Windows x64

The Windows build is a Rust/GPUI application. A normal build does not require Python, CUDA, Vulkan or NGX SDKs, game files, or the historical `development/` tree. It does require the native Windows compiler and SDK used by GPUI and Windows resources.

管理器采用 Rust/GPUI。普通编译不依赖 Python、CUDA、Vulkan/NGX SDK、游戏文件或历史开发目录；需要 Visual Studio C++ 生成工具和 Windows SDK。

## Prerequisites

- Windows 10/11 x64; Visual Studio 2022 Build Tools with **Desktop development with C++**, an x64 MSVC toolset, and Windows SDK.
- Rustup. `rust-toolchain.toml` pins the compiler, target, Clippy and rustfmt; do not replace this with an untested nightly toolchain.
- Use a Developer PowerShell for Visual Studio. Network access is needed for Cargo dependencies. Administrator elevation is not required for normal compilation.

```powershell
git clone https://github.com/pandaligx/RTX-FG-Manager.git
cd RTX-FG-Manager
./tools/prepare-build.ps1
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Output: `target/x86_64-pc-windows-msvc/release/RTXManager.exe`. The result is **unsigned**. Release signing is separate; never rebuild over a publisher-signed release. Normal debug and release builds embed neither a downloader executable nor game DLLs.

Downloads use Rust HTTPS requests with a shared 30-connection limit, checked redirects, resumable blocks, and mandatory payload integrity checks. `prepare-build.ps1` remains a no-op compatibility entry point for existing build scripts. Historical aria2 resources are not embedded or executed by the current manager.

## Optional tests using released, signed binaries

普通 `cargo test` 运行不依赖真实补丁的逻辑和文件系统回归。真实签名 DLL/EXE、旧版升级和专项缓存回归单独启用，不能把默认测试通过描述成这些测试或实体显卡游戏验收通过。

```powershell
./tools/prepare-fixtures.ps1
./tools/prepare-fixtures.ps1 -VerifyOnly
cargo test --locked --features fixture-tests
cargo clippy --locked --all-targets --features fixture-tests -- -D warnings
```

`tests/fixture-manifest.json` pins historical release resources and their exact extracted bytes. Files are stored only in ignored `tests/fixtures/runtime/`. Preparation downloads existing published artifacts, not arbitrary game folders. The Streamline fixture is an optional upstream SDK download of approximately 218 MiB; it retains its upstream license. No test downloads network resources automatically. Missing, changed, or unavailable fixtures fail explicitly. Do not enable `fixture-tests` for a distributable release: it embeds historical DLL fixtures for tests.

## Optional native GPU probes

The UI does not need the C++ synthetic GPU probes. They are a **maintainer-only extension**, separate from the complete public manager build. Their additional native sources are retained in the maintainer's workspace and are not part of this source release; this Rust manager release does not publish the DLL bridge/patch implementation. Enabling the feature in a plain public clone intentionally fails with an explicit explanation. Maintainers who already have the extra `rust/native` sources must also supply SDK headers under the applicable upstream licenses; proprietary NGX headers are not included in this source repository.

```powershell
$env:RTXFG_VULKAN_INCLUDE = 'D:\SDKs\Vulkan-Headers\include'
$env:RTXFG_NGX_INCLUDE = 'D:\SDKs\NGX\include'
# Maintainer workspace only: requires the additional native sources.
cargo build --locked --features native-probes,fixture-tests
```

`RTXFG_NGX_INCLUDE` must contain `nvsdk_ngx_params.h` and its required `nvsdk_ngx_defs.h`; `RTXFG_VULKAN_INCLUDE` must contain `vulkan/vulkan.h`. `native-probes` alone can inspect an explicitly supplied, recognized game deployment; the historical default diagnostic payload requires `fixture-tests`. Builds without `native-probes` return an explicit unsupported-build error for the internal native self-test commands. Native tests need real Windows GPU/driver support; compilation and filesystem tests do not establish hardware compatibility.

## Source and CI boundaries

Keep `Cargo.lock`, `.cargo/config.toml`, Rust sources/assets, current JSON language resources, and `vendor/gpui_fast_windows` together. GPUI Kit 0.9.1 is pinned to commit `39ab849a93b5f15507b969164d49891db8d5b470`; its `gpui-fast` feature selects the Fast backend, resolved to crates.io 0.1.14. Shared GPUI Pre support crates are pinned to 0.3.12. The Windows vendor patch starts from the published Fast 0.1.14 crate and retains the manager's first-frame, DPI and font compatibility changes alongside the upstream frame scheduler. These retained changes do not establish a new Windows 10 or physical multi-monitor test result. The former Zed Windows and sum-tree patches are no longer build inputs or part of this source export.

The GPUI Pre 0.3.9 sum-tree, ztracing and ztracing-macro packages are Apache-2.0. The former GPL-only tracing-chain workaround is therefore unnecessary. Preserve the actual upstream license files and local change notes.

For Windows renderer/font regressions, run `cargo test --manifest-path vendor/gpui_fast_windows/Cargo.toml --features test-support fast:: -- --test-threads=1`, then repeat with the `direct_write::` filter. The backend has test-only dependencies and is tested as a standalone crate, with its own generated lockfile; the manager's root lockfile and release features remain separate. These checks need Direct3D or WARP and the upstream test dependencies; they do not execute game patches. The interactive GPU-reset test remains opt-in. Build the distributable from the project root without this test feature. The vendored pixel fixtures come from the same Fast 0.1.10 release.

The 21 offline theme sets in `rust/assets/themes` remain pinned to Kit commit `4921e5b69834e45592c9772c57f39a29ada7de2f`. Their 36 variants plus Kit's two defaults provide 38 color variants. The sidebar uses Kit's `Command` component for one searchable list and ephemeral hover/keyboard previews. Only confirmation persists the selected name and mode; cancellation restores saved preferences, and a generation token rejects stale deferred callbacks. Suspend system-theme polling during previews. Keep independent light/dark selections through `Theme::update`; unavailable selections fall back to the matching mode's default. Preserve theme source attribution, license and embedded JSON files with the Rust source.

The GitHub Windows workflow builds an **unsigned test artifact**, never signs, publishes, or replaces the release update manifest. Signed EXE and game DLL releases remain separate from source commits.

The manager's opaque Windows UI explicitly uses the backend's HWND swap-chain path for monitoring-overlay compatibility. D3D11 hardware rendering and GPUI Fast remain enabled. This does not change system environment variables or monitoring settings; native composition surfaces and transparent backdrops would need separate integration before use.

Secrets, certificates/private keys, local logs, game binaries, historical development outputs and runtime caches must not be committed. The published source export uses an explicit file allowlist in addition to `.gitignore`. DLL ownership records used for safe cleanup are required application data and are not disposable build caches.
