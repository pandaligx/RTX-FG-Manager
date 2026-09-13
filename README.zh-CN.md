# RTX 帧生成管理器 · by小南瓜

[English](README.md) · [简体中文](README.zh-CN.md)

<p align="center">
  <img src="https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/docs/screenshot-home-zh.png" alt="RTX 帧生成管理器主界面" width="820" />
</p>

图片无法显示？[在 Gitee 查看原图](https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/docs/screenshot-home-zh.png) · [国内文档镜像](https://gitee.com/pandaligx/RTX-FG-Manager/blob/main/README.zh-CN.md)

<p align="center">
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="版本" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="下载量" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="平台" src="https://img.shields.io/badge/platform-Windows_x64-0078D6?logo=windows&logoColor=white">
  <img alt="开发语言" src="https://img.shields.io/badge/language-Rust-DEA584?logo=rust&logoColor=white">
  <a href="LICENSE"><img alt="管理器许可" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

管理 RTX 20/30 系列帧生成适配补丁的 Windows 工具，采用 **Rust 核心 + GPUI 原生界面**。本仓库是**软件发行仓库**，提供 EXE、文档、截图和更新附件，不公开管理器源码；第三方组件保留各自许可。

## 下载

- [GitHub Releases](https://github.com/pandaligx/RTX-FG-Manager/releases/latest)
- [Gitee Releases · 国内镜像](https://gitee.com/pandaligx/RTX-FG-Manager/releases)

下载已签名的 `RTXManager-v<版本>-x64.exe`，直接运行，无需安装 Python、Rust、CUDA Toolkit 或独立 aria2。支持 Windows 10/11 x64；帧生成兼容性取决于显卡、驱动和游戏。

## 主要功能

- 中文、英文、俄文、日文、韩语；默认跟随系统语言，在设置中手动选择后记忆。支持浅色、深色和跟随系统主题。
- Rust 后台处理扫描、安装卸载、系统操作和更新；GPUI 原生界面及组件图标，支持窗口居中、系统缩放与布局适配。游戏图标优先读取高分辨率资源。
- 扫描目录、磁盘或全盘，也可手动添加游戏 EXE。补充 Unreal、Unity、DLSS 组件识别线索；扫描结果不代表已经验证帧生成兼容。
- 游戏库显示部署方案/入口标签，主页直接展示操作日志。点击游戏行选中当前游戏；前方复选框用于批量操作，两者独立。
- 支持单个与批量安装、卸载、移出游戏库；右键打开目录或移除，双击打开目录。移出游戏库不等于卸载补丁。
- 单个安装/卸载不再重复确认；批量操作显示编号目标列表。失败或游戏未退出会明确警告，不能当作操作成功。
- 显卡页面支持修改/还原 Windows 显示名称。同一显卡更新驱动后可迁移备份，保留新驱动名称，并安全处理工具遗留的别名。
- 五语言使用说明，首次使用自动展示；主页提供“图形设置”按钮。不会在每次选游戏时重复检测或弹出硬件加速 GPU 计划提醒。
- 后台检查更新、可选自动下载，内置 aria2；校验下载摘要及发布者签名，安装前确认，并保留旧 EXE 备份。

## 帧生成方案

| 方案 | 图形接口 | 倍率上限 | DLL 入口 |
| --- | --- | --- | --- |
| **0.2.6 正式（默认）** | DX12 / Vulkan | 4X | 五种可选，默认 version.dll |
| **0.2.6 5X/6X 实验** | DX12 / Vulkan | 6X | 五种可选，默认 version.dll |
| 初始方案 · GitHub 第一版 | 沿用原 R2/SM86 能力 | 取决于原方案和游戏 | 仅显示随该方案提供的入口 |

五种入口为 `version.dll`、`winmm.dll`、`dinput8.dll`、`winhttp.dll`、`dxgi.dll`，建议每次仅选一个。多选可能产生冲突，不代表兼容性更好。每个代理已集成桥接，无需外接 `rtxfg_vk_bridge.dll`。RTX20 使用 SM75 路径，RTX30 使用 SM86 路径。

6X 是可请求的倍率上限，**不会自动添加游戏菜单**。例如永劫无间菜单最高 4X，选择实验方案也不会出现 5X/6X。

## 本项目对原有 DLL 的改进

基于 [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86) Native 0.2.4，由本项目维护兼容扩展；**0.2.6 是本项目方案版本，不是上游版本号**。

- **纹理兼容（Fix1）**：修正无类型纹理的 SRV/UAV 视图格式处理，解决已复现的永劫无间开启帧生成后进对局崩溃。
- **Vulkan 接入**：补充 Vulkan 资源接入、DX12 后端互操作与同步，集成至正式/实验方案的全部五种代理。
- **传输与调度优化**：GPU 共享传输、资源缓存复用、合并提交，减少 CPU 图像中转、重复分配和等待；不具备共享条件时保留回退路径。
- **0.2.6 启动兼容修复**：延迟至实际需要时加载 D3D12，避免游戏 Agility SDK 信息尚未初始化时提前触发加载，针对燕云十六声启动回归；增加进程生命周期保护，修复测试中发现的刚加载即卸载崩溃。
- **实验倍率扩展**：基于社区 5X/6X 扩展合入 Fix1、Vulkan 及本次启动修复。初始 GitHub 第一版保持原有文件。

模型仍为 **310.1**，原生推理内核未更新。不能把本项目版本号理解成更新了 NVIDIA 模型，也不承诺固定帧率或所有游戏兼容。完整改动、测试范围与待验证问题见 [Release 更新说明](https://github.com/pandaligx/RTX-FG-Manager/releases/latest)。

## 简单使用

1. 在 Windows 设置 → 系统 → 屏幕 → 显示卡 → 更改默认图形设置中开启“硬件加速 GPU 计划”；不同系统版本名称略有差异。可点击主页“图形设置”，按 Windows 提示重启。
2. 完全退出游戏，添加游戏本体 EXE，或通过扫描旁的箭头选择范围。手动添加允许更多 EXE 架构，但安装 x64 补丁时仍检查兼容性。
3. 点击游戏行，选择显卡系列、方案和一个 DLL 入口，再安装补丁。批量处理时勾选多个游戏，核对编号名单。
4. 启动游戏，在游戏内开启 DLSS 帧生成及其支持的倍率。关闭管理器不会影响已安装补丁。
5. 切换方案或入口前，退出游戏、卸载旧补丁再安装。若提示游戏仍运行，代表尚未完成卸载，请退出后重试。

卸载可识别本项目新旧补丁、修改过的 INI 和重新签名的已识别 DLL，保留游戏自带和未知文件。更新管理器**不会自动更新游戏中的 DLL**，需自行退出游戏并重新部署。

显卡名称修改只影响 Windows 显示名称字段，不改变真实硬件或 CUDA 能力；游戏是否采用新名称取决于其读取方式，可能需要重启。换卡或无法可靠识别备份时仍保留保护。请遵守游戏规则，带反作弊的游戏应先确认是否允许第三方补丁。

## 从 3.7.4 升级

可直接升级到 4.0.8，无需安装中间版本。保留已有游戏列表、偏好和部署记录；旧补丁标签不会因管理器升级而自动变成 0.2.6。诊断日志与兼容性自检面板已移除，主页操作日志保留。

设置和游戏列表保存在 `%LOCALAPPDATA%\RTXFGManager`。语言、主题、更新线路等在设置页调整；默认自动检查更新、自动下载关闭。自动线路按 **Windows 系统地区**选择：中国优先 Gitee，其余地区优先 GitHub，也可手动指定；可用性或版本异常时尝试另一站点。这不是 IP 定位。

## 致谢与联系

- [Github · sdli1995](https://github.com/sdli1995/dlssg_for_sm86)：上游项目。
- [pipotoufikxyz-lgtm](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86)：社区 5X/6X 扩展。
- [GPUI](https://gpui.rs/) · [GPUI Component](https://github.com/longbridge/gpui-component)：原生界面与组件。
- [aria2](https://github.com/aria2/aria2)：独立下载工具。
- [哔哩哔哩 · 大大大怪将军阁下](https://space.bilibili.com/608531525)：协助测试。
- [个人网站](https://lgxng.cn/) · [GitHub](https://github.com/pandaligx) · [哔哩哔哩](https://b23.tv/5mHCHFn)。

许可与归属见 [第三方声明](THIRD_PARTY_NOTICES.txt)。管理器许可不改变 NVIDIA 或其他第三方材料的权利；本项目与 NVIDIA 或上游项目无官方隶属关系。
