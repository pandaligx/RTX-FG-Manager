# RTX 帧生成管理器 · by 小南瓜

**在一个界面里，完成帧生成补丁的安装、调节与卸载。**
面向 RTX 20 / 30 / 40 的 Windows 便携工具，采用 Rust 核心与 GPUI 原生界面。

[English](README.md) · **简体中文**

<p align="center">
  <a href="https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9"><img alt="版本" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9"><img alt="下载量" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?logo=windows&logoColor=white">
  <a href="LICENSE"><img alt="管理器许可 MIT" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

- **游戏集中管理**：扫描或手动添加，单个安装或批量处理，直观看到实际部署的方案与入口。
- **每款游戏独立记忆**：保存方案、DLL 入口和预设参数，需要时再下载匹配文件。
- **部署目录更灵活**：放在游戏 EXE 旁，也可关联独立插件文件夹，卸载时按归属清理。
- **界面随你调整**：38 个离线配色可实时预览，支持简体中文、英语、俄语、日语和韩语。

## 下载

**当前版本：4.2.9** · [国内下载 · Gitee](https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9) · [GitHub 下载](https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.9) · [更新日志](CHANGELOG.zh-CN.md)

下载 **`RTXManager-v4.2.9-x64.exe`**，适用于 Windows 10 / 11 x64。发行页提供已签名 EXE 和校验清单；无需另装 Python、Rust、CUDA Toolkit 或 aria2，游戏 DLL 按需从云端获取。

## 4.3.0 候选版

以下说明适用于正在验收的 4.3.0。正式 EXE 仍为上方 4.2.9；新版完成统一验收及发布者签名后再发布。

### 三个补丁

| 方案 | 这次更新 | 游戏内菜单 |
| --- | --- | --- |
| **dlssg_for_sm86 0.4.0（默认）** | 完整组件安装，包含 SR；旧 0.3.5 配置迁移；六种代理使用各自文件，一次一个入口 | INI／管理器设置 |
| **MFG Unlocker 1.5.0** | Preset C；独立代理与已有 OptiScaler 10 插件模式互斥 | 独立模式 **Backspace**；插件模式用 OptiScaler 菜单 |
| **RTX Encore 1.0.0-rc.3** | 独立 SR/RR/FG 预设、动态插帧及 Smooth Motion 参数；专用代理和 ASI 使用对应文件 | **Insert** |

三角洲、自定义 Vulkan 和 0.2.6 方案保留。初始第一版不再提供新安装，旧安装仍可识别、卸载与恢复。显卡检测只作参考，不限制现有方案选择；旧方案仍按手选 RTX20/RTX30 组件版本部署。开放选择不代表硬件、驱动或游戏满足补丁要求。

**签名与入口。** 原样复制或改名不改变 DLL 字节及签名。Encore 的 `version.dll`、`winmm.dll`、`dinput8.dll`、`dxgi.dll` 和 ASI 使用各自分发文件；其他支持名称由主 DLL 提供。不能把主 DLL 的签名当作其他独立文件的签名。入口名称只决定加载方式，不增加图形 API 支持。

**Bink 与插件。** Bink 原件须预先保留在旁边，分别命名为 `binkw64Hooked.dll` / `bink2w64Hooked.dll`；管理器不会自动改名或删除它们。MFG 插件使用已有 OptiScaler 10 的默认 `OptiScaler/plugins` 目录，不自动安装 OptiScaler；自定义插件搜索路径暂不自动适配。

### 开始使用

1. 退出游戏，添加实际运行的 EXE；Unreal 游戏通常位于 `Binaries/Win64`。
2. 选一个补丁和空闲加载入口，打开游戏详情查看参数，再点击 **安装并应用**。
3. 启动游戏，开启游戏支持的帧生成；按上表按键打开菜单。标注需要重启的参数，退出游戏后再修改。

游戏的 **玩家配置** 是独立入口，与 **预设参数** 并列。预设参数内保留 **概览、配置、记录与恢复**。配置可命名、导出、预览差异后选择导入；撤销恢复原始文件，遇到后续修改冲突时停止覆盖。切换方案前先卸载；受管理的同方案升级保留配置并提供失败恢复。更新管理器不会自动替换游戏补丁。

### 网页帮助与独立社区

点击 **使用说明** 打开独立 HTML 帮助窗口：搜索、目录、方案切换、复制参数和明暗主题均可离线使用。内容独立更新，失败继续使用缓存或内置版本。缺少 WebView2 时用默认浏览器打开；不自动安装运行时。五种界面语言保留，缺失的新文章明确回退到英语。

[在线帮助](https://rtx-fg-community.1547245493.workers.dev/help/) 与内嵌帮助使用相同静态内容。帮助页没有游戏路径、文件操作或命令执行接口。

打开 **玩家配置**，即可浏览、应用或分享当前游戏的配置；列表每 30 秒刷新。只推荐当前游戏的配置，可比较同一游戏的不同补丁方案。卡片显示方案、补丁版本与 DLL / ASI 入口；实际安装不一致时先引导切换，再预览参数。导入按补丁协议隔离，仅核对 **显卡型号、驱动版本**，在本机优先匹配，再参考“我也能用”确认和点赞。不采集显存、分辨率或显卡架构。导入前查看参数差异；本机路径、诊断选项和默认快捷键不参与分享。发布前显示完整提交内容，每个身份每游戏最多 5 份；可更新、删除、点赞、确认可用和撤销互动。玩家测量不代表管理器实测，匹配度不是性能保证。

这是独立社区，不接入 RTX Encore Hub 的私人数据库。首次写入需要验证；身份私钥由 Windows DPAPI 保护，可主动创建加密备份。服务不可用时，补丁管理、本地配置和离线帮助继续可用。初期社区可能没有玩家配置。

### 下载、升级与清理

GitHub 保留；新版资源优先 GitCode，失败回退 GitHub。Gitee 保留旧客户端过渡入口及历史资源。源码、文档和小附件单向自动同步；本次大补丁采用本机上传 GitCode。缓存下载完成并校验后才用于安装。

卸载只处理当前游戏所属组件、状态和已知日志，包括 dlssg 帧时间/NGX 日志、MFG 游戏专属临时日志和 Encore 会话/异常/性能日志。未知文件、其他游戏、其他插件和手动抓取保留；占用或恢复未完成时提示重试。OptiScaler 只恢复管理器修改且用户未再次改动的字段。

**清理缓存不等于卸载补丁；移除游戏条目也不等于卸载。** 日常操作集中在当前游戏详情中，批量勾选另行确认。

### 验证边界

管理器的离线回归与窗口检查不等于实际游戏、GPU 性能或反作弊验收。RTX3060 6GB 的已反馈显存问题未作为本版修复项。补丁功能仍受各上游的硬件、驱动及游戏接口要求约束。

## 开源与致谢

Rust 管理器采用 [MIT 许可证](LICENSE)，提供 [构建说明](BUILDING.md) 与 [云端维护指南](docs/cloud-publishing.md)。游戏 DLL 保留各自许可，公开仓库不包含私有 DLL 补丁源码或 NVIDIA SDK 头文件；详见 [第三方说明](THIRD_PARTY_NOTICES.txt)。

感谢 [sdli1995 / dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86)、[pipotoufikxyz-lgtm / MFG](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version)、[dashdogy / MFG-Unlocker](https://github.com/dashdogy/MFG-Unlocker)、[SilyNoMeta / RTX Encore（原 DLSSG-Transfusion）](https://github.com/SilyNoMeta/rtx-encore)、[GPUI](https://gpui.rs/)、[GPUI Kit](https://github.com/longbridge/gpui-kit)、[GPUI Fast](https://github.com/longbridge/gpui-fast) 与 [aria2](https://github.com/aria2/aria2)。

也感谢 [大大大怪将军阁下](https://space.bilibili.com/608531525)、[云外逸声](https://space.bilibili.com/256887068) 和提供兼容性反馈的用户。本项目与 NVIDIA、游戏厂商及上游项目无官方隶属关系。

[个人网站](https://lgxng.cn/) · [哔哩哔哩](https://b23.tv/5mHCHFn) · [GitHub](https://github.com/pandaligx/RTX-FG-Manager) · [Gitee](https://gitee.com/pandaligx/RTX-FG-Manager)
