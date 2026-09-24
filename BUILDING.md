# Build the Rust manager / 编译 Rust 管理器

The manager is a Windows x64 Rust/GPUI application. A normal build does not require Python, CUDA, Vulkan or NGX SDKs, game files, or the historical `development/` tree. It does require the native Windows compiler and SDK used by GPUI and Windows resources.

管理器采用 Rust/GPUI。普通编译不依赖 Python、CUDA、Vulkan/NGX SDK、游戏文件或历史开发目录；需要 Visual Studio C++ 生成工具和 Windows SDK。

## Prerequisites

- Windows 10/11 x64; Visual Studio 2022 Build Tools with **Desktop development with C++**, an x64 MSVC toolset, and Windows SDK.
- Rustup. `rust-toolchain.toml` pins the compiler, target, Clippy and rustfmt; do not replace this with an untested nightly toolchain.
- Use a Developer PowerShell for Visual Studio. Network access is needed for Cargo dependencies and the initial pinned downloader resource. Administrator elevation is not required for normal compilation.

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

Output: `target/x86_64-pc-windows-msvc/release/RTXManager.exe`. The result is **unsigned**. Release signing is separate; never rebuild over a publisher-signed release. Normal debug and release builds embed only the pinned aria2 executable and its configuration, not game DLLs.

`prepare-build.ps1` fetches the unchanged publisher-signed aria2 binary identified by size and SHA-256 in `tools/build-resources.json`. This makes the manager's input reproducible; **it does not claim that this historical aria2 binary can be reproduced from source**. Its original build/source records were not retained; see `THIRD_PARTY_NOTICES.txt`. Bootstrap may use Windows HTTPS download once, because aria2 cannot download itself before it exists. Once prepared, resource downloads use the verified aria2 executable. Existing mismatched files cause a clear error and are not overwritten automatically.

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

Keep `Cargo.lock`, `.cargo/config.toml`, Rust sources/assets, current JSON language resources, `vendor/gpui_windows`, and `vendor/sum_tree` together. The vendored patches preserve Windows compatibility and initial-window behavior. The GitHub Windows workflow builds an **unsigned test artifact**, never signs, publishes, or replaces the release update manifest. Signed EXE and game DLL releases remain separate from source commits.

Secrets, certificates/private keys, local logs, game binaries, historical development outputs and runtime caches must not be committed. The published source export uses an explicit file allowlist in addition to `.gitignore`. DLL ownership records used for safe cleanup are required application data and are not disposable build caches.
