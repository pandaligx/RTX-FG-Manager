# Client update channels

新下载统一使用 `RTXManager.exe`。4.3.21 是最后一个兼容过渡版本；此后每版仅上传一个 EXE。每个版本使用独立发行标签，文件同名不会覆盖旧版。

| 客户端 | 固定清单 | 发行附件 | 后续升级 |
| --- | --- | --- | --- |
| 4.3.20 及更早 | `update.json`（永久保留 4.3.21） | `v4.3.21/RTXManager-v4.3.21-x64.exe` | 先升级到 4.3.21，再检查更新 |
| 4.3.21 及以后 | `update-v2.json`（schema 2） | `v版本号/RTXManager.exe` | 直接升级最新正式版 |
| Linux AppImage 4.3.32 及以后 | `update-linux.json`（schema 3） | `v版本号/RTXManager-版本号-x86_64.AppImage` | 检查、下载并重启更新 |

4.3.21 两个文件名对应同一份签名原件，只签名一次。旧安装更新后保留自己的文件名、路径和快捷方式；需要统一名称时，可在设置中主动迁移。不会强制改名。

## Release procedure / 发布顺序

1. Build and test, then let the publisher sign **one** EXE. Do not rebuild it after signing.
2. Prepare assets locally (the command verifies the publisher, file version and signature):

   ```powershell
   python tools/client_release.py --exe path/to/RTXManager.exe --version 4.3.21 --notes release-notes.json --output new-release-directory
   ```

   `release-notes.json` contains language keys and release-note strings, such as `{"zh-CN":"…","en":"…"}`. For **4.3.22 and later**, also pass `--legacy-manifest path/to/verified-4.3.21-update.json`. The tool carries that legacy JSON forward unchanged and produces only `RTXManager.exe`.

3. Upload EXEs **from the publisher machine** to GitHub and GitCode. On 4.3.21 only, upload the identical versioned alias too. Verify complete bytes, size and SHA-256 on both sites.
4. Add the small release assets, including **both** JSON manifests, checksums and third-party information. Every future release retains the old `update.json` because some old clients fall back to `releases/latest/download/update.json`. CI mirrors only these small client assets, never an EXE.
5. Finally promote the verified JSON files on `main`. At the initial 4.3.21 cutover, commit **both** channels together. Later change only `update-v2.json`; keep `update.json` and the 4.3.21 release available permanently. Do not change any 4.3.20 release asset.

The publisher rejects channel swaps, downgrades, changed same-version identities, a missing legacy bridge and future versioned EXE attachments. Never point either channel to an unsigned candidate. Partial uploads must not advance either update pointer.

## Compatibility and validation

Schema 1 installation tickets remain accepted by the new installer, allowing an unmodified legacy client to launch the signed 4.3.21 bridge. Schema 2 requires the exact fixed filename; URLs are constructed from the official repository, version tag and validated filename. Cache entries remain digest-separated, even when different versions share a filename. Signature, version, size and digest verification still apply.

Modern clients check only the modern channel, falling back between GitCode and GitHub. They never silently treat the frozen bridge as the latest version when the modern service is unavailable. Network errors remain visible and can be retried.

Tests cover old/new manifest contracts, retained legacy latest-release fallback, future fixed-name packaging, source promotion guards, Windows replacement/rollback and existing shortcut behavior. Unsigned candidate tests cannot prove an Authenticode-gated signed upgrade. After signing, complete isolated legacy-to-bridge and fixed-path restart checks before publishing, without modifying users' games or shortcuts.

## Linux AppImage channel / 独立更新通道

4.3.31 及更早的 Linux 版需手动下载一次新版。4.3.32 起，直接运行的 AppImage 使用独立 schema 3 清单，跟随设置中的 GitCode/GitHub 来源，校验大小、SHA-256、x86_64 与 Type 2 格式后更新。保留原文件名、数据目录及 Wine 前缀；新版确认窗口已启动后才删除旧文件，启动失败回滚。解包运行、链接路径或不可写安装目录保留手动更新提示。Linux 不使用 Windows Authenticode，也不会下载 EXE。

Prepare the successful Linux CI artifact from the exact release commit:

```sh
python tools/linux_release.py --appimage dist/RTXManager-4.3.32-x86_64.AppImage --version 4.3.32 --notes release-notes.json --output linux-release
```

Upload the AppImage from the publisher machine to **both** release pages, verify the full bytes, then upload `SHA256SUMS-linux.txt` and `update-linux.json`. Promote only that Linux channel after both copies pass. Publish it alongside the Windows release while retaining the two Windows manifests unchanged. The publishing tools reject channel swaps, rollbacks and changes to an already published same-version identity. CI creates unsigned test artifacts; it does not promote an update channel or upload the Windows EXE.
