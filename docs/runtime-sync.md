# Official runtime distribution

The daily GitHub Actions workflow checks stable **NVIDIA/DLSS** releases at 01:43 UTC (09:43 Beijing time; GitHub scheduling may be delayed). It reads only `lib/Windows_x86_64/rel` at the release's resolved commit. The three game runtimes are SR (`nvngx_dlss.dll`), RR (`nvngx_dlssd.dll`) and FG (`nvngx_dlssg.dll`). These are official precompiled DLLs, not open-source implementations of the DLSS models.

Changed files must match the official Git blob and file size, be x64 DLLs, have a valid NVIDIA Authenticode signature, and match the release version. Original bytes and the SDK license are packaged into immutable, SHA-256-named ZIPs. GitCode Resources and GitHub Resources must both pass anonymous full-file readback before `cloud/runtime-catalog.json` advances. Unchanged files are not downloaded again. Failures preserve the previous catalog; existing same-version files are never silently replaced. New major versions require a compatibility review.

The workflow uses existing `GITCODE_TOKEN` and the job's repository-scoped `GITHUB_TOKEN` (`contents: write`). It does not upload client EXEs, publish manager releases or change `update.json`. A run report is retained for 14 days. No additional server is required.

Manager 4.3.10 and later refresh the independent runtime catalog from the selected download source, with cache and bundled fallback. Refreshing a catalog does **not** change game files. Users explicitly choose **Back up and update** and can restore originals. Shared game DLL changes may affect every patch using those files; a valid signature does not prove game compatibility.

## Patch-private dependencies

| Component | Update policy |
| --- | --- |
| dlssg_for_sm86 `dlssg_sm86/` | Updated only with its complete patch; bundled kernels/runtime versions must remain paired. Its XeSS engine does not use Intel `libxess_fg.dll`. |
| MFG standalone `RTXMFG-XeSS/` | Pinned Intel SDK 3.0.2 FG + XeLL pair, optional separate install. OptiScaler mode keeps OptiScaler's own files. |
| Encore `rtx-encore-runtimes/` | Updated with the complete Encore patch, separate from game Intel runtimes. |
| DLSS Neural Rendering | Existing qualified 310.9.16 remains in the same runtime window. NVIDIA/DLSS currently does not include `nvngx_dlssnr.dll`; automatic NR promotion is disabled until an official source and patch compatibility are reviewed. |

The scanner excludes known private directories and backups. Deployment ownership records protect managed files, including custom deployment directories. Existing recovery records remain usable; files changed by a game or another tool are preserved.

## 简体中文

每天北京时间约 09:43 检查 NVIDIA 官方正式 SDK，核验原件后同步 GitCode、GitHub，最后更新独立运行库清单。只同步 x64 正式版 SR、RR、FG，不使用 DLSS Swapper、游戏提取或未知来源文件。未变更的 DLL 不重复下载。

Intel 保留官方 SDK 3.0.2 配套版本；NR 保留现有兼容版本，因为 NVIDIA 当前公开目录中未提供该文件。补丁私有目录不会被通用 DLSS 更新覆盖。客户端更新清单不等于自动修改游戏：由用户点击安装，保留恢复原件入口。不同游戏的实际兼容性仍需游戏验证。

Sources: [NVIDIA DLSS](https://github.com/NVIDIA/DLSS/releases), [Intel XeSS SDK](https://github.com/intel/xess/releases), [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86), [MFG 1.7](https://github.com/dashdogy/MFG-Unlocker/tree/v1.7.0), [Encore Intel engines](https://github.com/SilyNoMeta/rtx-encore/blob/v1.4.1/docs/INTEL-ENGINES.md).
