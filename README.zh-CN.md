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
- 启动后台检查更新，有新版时居中提示；内置 aria2，支持断点续传、圆环进度、大小/速度/剩余时间。点击“下载并更新”后，校验摘要与发布者签名，自动在原目录替换、使用新版文件名并重启，新版界面启动成功后删除旧 EXE，启动失败则还原。可选自动下载仅提前下载，仍需确认本次更新后才替换。

## 云端 DLL 资源

管理器不再内置游戏 DLL，首次安装按所选方案和入口下载对应压缩包。默认使用国内 HTTPS 文件服务器，失败后尝试 GitHub；DLL 包使用内置 aria2 处理多连接和下载站跳转，国内线路会先完成兼容重试，再进入 GitHub 回退。验证通过的缓存可离线复用。下载完成并校验后才写入游戏目录，卸载不依赖网络。

方案清单可独立更新名称、版本、DLL 和 INI；沿用支持的安装协议时，不必重新下载管理器。管理器升级不会自动替换游戏中已安装的补丁，切换版本仍需先卸载再安装。

## 适配方案与预设参数

| 方案 | 图形接口 | 参数与入口 |
| --- | --- | --- |
| **0.3.5 · DX12/Vulkan（默认）** | DX12 / Vulkan，SM75/SM86 | 本项目扩展，保留上游 310.9 模型；六入口可多选，默认倍率上限 4X |
| **0.2.6 · DX12/Vulkan** | DX12 / Vulkan | 本项目正式方案，五入口；最高 4X |
| **初始方案 · Github 第一版** | 原始 R2/SM86 能力 | 一个方案自动按 RTX20/RTX30 选择对应 version.dll 与 INI |

上游 0.3.5 使用 310.9 模型，修复帧生成特性重建时误用优化内核造成的花屏、崩溃风险；同时包含 0.3.4 的 RTX30 架构识别修复和 0.3.2 起的四档优化。`Optimized=1` 默认逐位一致；0 为原厂数值，2/3 接受画质损失以进一步加速。以上是[上游发布说明](https://github.com/sdli1995/dlssg_for_sm86/releases/tag/0.3.5)及其测试结论，不代表本项目已重新完成所有游戏实测。

0.3.5 六入口是 `version.dll`、`winmm.dll`、`dinput8.dll`、`dbghelp.dll`、`dxgi.dll`、`d3d12.dll`。建议先用 `version.dll`；前四种优先，后两种按需使用，`dxgi.dll` 与 `d3d12.dll` 二选一。多个入口中只有先加载的一个工作，其余转发。**本项目的 0.3.5 · DX12/Vulkan 在上游版本上加入 Vulkan 扩展；上游原版仍为 D3D12。** 六个入口均已集成，无需额外桥接 DLL。0.2.6 五入口为 `version.dll`、`winmm.dll`、`dinput8.dll`、`winhttp.dll`、`dxgi.dll`，建议每次选择一个。

右侧 **预设参数** 折叠面板按方案显示选项，点 **?** 查看说明。0.3.5 沿用上游参数：内核档位、倍率上限、UI 重组及日志；0.2.6 提供倍率、采样与日志；初始方案提供启用、倍率与日志。每游戏、每方案独立记忆，保持默认即可。完全退出游戏后点安装应用；相同 DLL 只更新受管参数，保留其他 INI 内容。**倍率上限不会添加游戏菜单，也不保证帧率按倍数增长。** 0.3.5 可将上限调至 6X，但实际倍率仍由游戏插件请求；《三角洲》当前实测为 2X，不支持通过修改此上限强制 4X。0.2.6 Vulkan 桥接日志暂不受日志级别控制；0.3.5 的详细桥接日志遵循级别设置，少量启动诊断独立保留。

云端默认国内优先，可选择 GitHub 优先。清单入口固定为 `https://www.lgxng.cn/1814328088/g/new/catalog.json`；简明清单负责方案与默认值，版本化索引负责文件完整性。以后同一参数/安装协议的版本可只更新云端，新增协议仍需升级管理器。旧版客户端会继续使用其有效缓存，建议升级管理器。

设置可清理已下载 DLL 与更新暂存，不删除游戏列表、参数或清理归属记录。清理后需要重新下载 DLL 才能离线安装。软件自动更新在新版界面成功启动后删除旧 EXE，失败时恢复旧版；不会扫描删除同目录其他 EXE。

## 本项目对原有 DLL 的改进

**0.3.5 · DX12/Vulkan** 基于 [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86) 0.3.5，保留其 **310.9 模型及推理优化**，由本项目增加：

- **Vulkan 帧生成接入**：资源互操作、GPU 共享传输、同步与回退路径，集成到全部六个代理。
- **传输与资源复用**：复用共享资源与缓存，减少 CPU 图像中转和重复等待；共享能力不可用时保留兼容路径。
- **三角洲 RTX2070 兼容**：修复已测试 RTX2070 Max-Q 的帧生成开关灰色问题，限定于已确认设备和游戏调用范围，不修改注册表或真实 CUDA 架构。该游戏目前只验证到 2X。
- **入口兼容保护**：各代理分别保留系统转发、主/待机机制及关闭开关保护；全部 DLL 已签名。

这是独立云端补丁更新，**现有 4.2.0 管理器无需更换 EXE**。重新打开管理器获取新目录，退出游戏后卸载旧补丁，再选择新方案安装。具体更新与测试范围见 [0.3.5 · DX12/Vulkan 发布说明](https://github.com/pandaligx/RTX-FG-Manager/releases/tag/payloads-20260920-dx12-vulkan)。开发机回归不能替代所有 RTX20/30 显卡与游戏实测。

**保留的 0.2.6 · DX12/Vulkan** 基于上游 Native 0.2.4；**0.2.6 是本项目方案版本，不是上游版本号**。它继续提供：

- **纹理兼容（Fix1）**：修正无类型纹理的 SRV/UAV 视图格式处理，解决已复现的永劫无间开启帧生成后进对局崩溃。
- **Vulkan 接入**：补充 Vulkan 资源接入、DX12 后端互操作与同步，集成至正式方案的全部五种代理。
- **传输与调度优化**：GPU 共享传输、资源缓存复用、合并提交，减少 CPU 图像中转、重复分配和等待；不具备共享条件时保留回退路径。
- **0.2.6 启动兼容修复**：延迟至实际需要时加载 D3D12，避免游戏 Agility SDK 信息尚未初始化时提前触发加载，针对燕云十六声启动回归；增加进程生命周期保护，修复测试中发现的刚加载即卸载崩溃。

**用户实测反馈**：RTX20 系列在《燕云十六声》中已可开启帧生成，此前缺少该选项的问题已确认解决；这是用户测试反馈，不代表所有显卡、驱动和游戏版本均已验收。

本项目 0.2.6 模型仍为 **310.1**，原生推理内核未更新。不能把本项目版本号理解成更新了 NVIDIA 模型，也不承诺固定帧率或所有游戏兼容。完整改动、测试范围与待验证问题见 [Release 更新说明](https://github.com/pandaligx/RTX-FG-Manager/releases/latest)。

## 简单使用

1. 在 Windows 设置 → 系统 → 屏幕 → 显示卡 → 更改默认图形设置中开启“硬件加速 GPU 计划”；不同系统版本名称略有差异。可点击主页“图形设置”，按 Windows 提示重启。
2. 完全退出游戏，添加游戏本体 EXE，或通过扫描旁的箭头选择范围。手动添加允许更多 EXE 架构，但安装 x64 补丁时仍检查兼容性。
3. 点击游戏行，选择显卡系列、方案和一个 DLL 入口，再安装补丁。批量处理时勾选多个游戏，核对编号名单。
4. 启动游戏，在游戏内开启 DLSS 帧生成及其支持的倍率。关闭管理器不会影响已安装补丁。
5. 切换方案或入口前，退出游戏、卸载旧补丁再安装。若提示游戏仍运行，代表尚未完成卸载，请退出后重试。

卸载可识别本项目新旧补丁、修改过的 INI 和重新签名的已识别 DLL，保留游戏自带和未知文件。更新管理器**不会自动更新游戏中的 DLL**，需自行退出游戏并重新部署。

显卡名称修改只影响 Windows 显示名称字段，不改变真实硬件或 CUDA 能力；游戏是否采用新名称取决于其读取方式，可能需要重启。换卡或无法可靠识别备份时仍保留保护。请遵守游戏规则，带反作弊的游戏应先确认是否允许第三方补丁。

## 从 3.7.4 升级

可直接升级到最新版，无需安装中间版本。保留已有游戏列表、偏好和部署记录；旧补丁标签不会因管理器升级而自动变成 0.2.6。诊断日志与兼容性自检面板已移除，主页操作日志保留。

设置和游戏列表保存在 `%LOCALAPPDATA%\RTXFGManager`。语言、主题、更新线路等在设置页调整；默认自动检查更新、自动下载关闭。自动线路按 **Windows 系统地区**选择：中国优先 Gitee，其余地区优先 GitHub，也可手动指定；可用性或版本异常时尝试另一站点。这不是 IP 定位。

## 致谢与联系

- [Github · sdli1995](https://github.com/sdli1995/dlssg_for_sm86)：上游项目。
- [pipotoufikxyz-lgtm](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86)：社区 5X/6X 扩展。
- [GPUI](https://gpui.rs/) · [GPUI Component](https://github.com/longbridge/gpui-component)：原生界面与组件。
- [aria2](https://github.com/aria2/aria2)：独立下载工具。
- [哔哩哔哩 · 大大大怪将军阁下](https://space.bilibili.com/608531525)：协助测试。
- [个人网站](https://lgxng.cn/) · [GitHub](https://github.com/pandaligx) · [哔哩哔哩](https://b23.tv/5mHCHFn)。

许可与归属见 [第三方声明](THIRD_PARTY_NOTICES.txt)。管理器许可不改变 NVIDIA 或其他第三方材料的权利；本项目与 NVIDIA 或上游项目无官方隶属关系。
