# RTX 帧生成管理器 · by小南瓜

**让 RTX 20 / 30 / 40 系列的帧生成补丁，更容易安装、调节和管理。**

开源 Rust 核心 + GPUI 原生界面。一个 EXE，集中管理游戏、云端 DLL、预设参数与卸载恢复。

[English](README.md) · **简体中文** · [4.2.6 国内下载 · Gitee](https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6) · [GitHub 下载](https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6)

> **4.2.6 更新**：左侧栏一键预览 38 个离线配色，支持自定义插件目录，并可在软件内查看历史更新。界面升级至 GPUI Kit 0.7.1 / Fast 0.1.2，新增管理员权限启动与按显卡系列选择兼容方案；更新 RTX40 MFG 1.4.1 Hotfix 1，加入独立 DLSSG-Transfusion 1.4.5.3。[完整更新日志](CHANGELOG.zh-CN.md)

### 4.2.6 新功能怎么使用

- **主题配色**：点击左侧栏的调色板图标，全部 38 个浅色、深色配色在同一个可搜索、可滚动的列表中显示，并带明暗标记。悬停或用上下方向键预览整个界面，点击或按 Enter 保存；按 Esc（搜索框非空也可）或点击列表外部取消并恢复原主题。顶部“跟随系统”保留分别记忆的浅色、深色配色，按系统自动切换。离线内置 21 套 GPUI Kit 主题，共 36 个变体，加默认浅色和默认深色。
- **原生界面框架**：采用 GPUI Kit 0.7.1 与 GPUI Fast 0.1.2，保留 Windows 首帧、DPI 与字体兼容调整。本次升级尚未证实 CPU 或游戏性能收益。
- **自定义文件夹**：先选中游戏，再点“添加游戏”右侧的“添加文件夹”，指定游戏需要的插件目录，例如 OptiScaler 插件目录。文件夹可没有 EXE，但仍关联原游戏进行运行检查、配置与卸载；这不会替你安装或配置第三方加载器。
- **管理员权限**：启动时请求 Windows UAC 授权，取消则停止启动。它解决受保护目录的写入权限问题，不绕过游戏的加载规则。
- **按系列选方案**：切换 RTX20 / RTX30 / RTX40 时保留兼容方案，或自动选择可用方案，不支持的方案会禁用；“方案来源”可打开对应项目。改选择不等于已安装，仍需应用。
- **更新日志**：设置页内置 4.2.6 及 4.2.0–4.2.5 历史说明，可在软件内按版本切换，离线也能查看；新版更新弹窗还可显示云端提供的变更内容。没有说明的旧清单仍可更新。
- **扫描与清理**：排除被误识别为游戏的 NVIDIA Vulkan 诊断工具，完善配置保留、日志清理和按游戏归属卸载的检查。

#### RTX40 MFG · 1.4.1 Hotfix 1

采用已签名的[上游发行](https://github.com/dashdogy/RTX40MFG-Unlock/releases/tag/v1.4.1-hotfix.1)，继续使用独立 `RTXMFG-Universal.json`，在管理器中仅面向 RTX40。预设增加垂直同步与固定模式 Reflex 限帧；动态模式使用动态目标，并保留未启用的固定限帧值。**Backspace 游戏内菜单仍为上游英文**，管理器内的参数说明支持五语言。

#### DLSSG-Transfusion · 1.4.5.3

独立接入 [SilyNoMeta/DLSSG-Transfusion](https://github.com/SilyNoMeta/DLSSG-Transfusion/releases/tag/v1.4.5.3-rtx20-30-40)，不混用 RTX40 MFG JSON 或原有 INI。默认跟随游戏，可选固定2X—6X或动态倍率；5X/6X为实验选项，动态默认最高4X，目标0跟随显示器刷新率。配置编辑保留注释及其他设置。

只选 `version.dll`、`dinput8.dll`、`dxgi.dll`、`winmm.dll` 中**一个**入口；管理器使用各自匹配的精确导出构建，不将它们互相改名。游戏必须已有 Streamline DLSS 帧生成。RTX20仍只有上游模拟验证，RTX20/30 Vulkan尚待实体测试，不能据此保证所有游戏可用。

游戏内 `Ctrl+Alt+2…6` 改倍率，`Ctrl+Alt+G` 跟随游戏，`Ctrl+Alt+D` 切换动态，`Ctrl+Alt+O` 显示统计。可选 ReShade 面板与 ASI 本轮不捆绑；Vulkan统计需要额外的兼容ReShade组件。Smooth Motion是另一个受驱动限制的实验功能，本版不开放开关。切换方案前退出游戏并卸载旧补丁，不叠加多个帧生成补丁。

<p align="center">
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="版本" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://github.com/pandaligx/RTX-FG-Manager/releases/latest"><img alt="下载量" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?logo=windows&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-GPUI-DEA584?logo=rust&logoColor=white">
  <a href="LICENSE"><img alt="管理器许可 MIT" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

<p align="center">
  <img src="https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/docs/screenshot-home-zh.png" alt="RTX 帧生成管理器：左侧浅色、右侧深色主题对照" width="980" />
</p>

<p align="center">较早版本的实际界面示意，使用演示游戏列表。4.2.6 新增的左侧主题选择器见上方说明。</p>

## 为什么使用管理器

| 你要做的事 | 管理器帮你完成 |
| --- | --- |
| 找到游戏并安装补丁 | 扫描后按安装目录合并同一游戏；手动添加 EXE 与自定义插件目录，单个或批量安装，直观看到已部署方案与入口 |
| 获取合适的 DLL | 云端按需下载，国内优先、GitHub 备用；校验后缓存，已有缓存可离线复用 |
| 调整效果与兼容性 | 按方案显示预设，游戏与方案分别记忆，点 **?** 查看说明，无需逐项手填 INI |
| 更新与还原 | 启动检查更新，aria2 断点下载、速度与圆环进度；补丁按归属卸载，占用时明确提示重试 |
| 日常使用 | 五语言、离线明暗配色与独立记忆、跟随系统主题、高 DPI 布局、后台任务、主页操作日志，关闭管理器后补丁仍生效 |

**单文件运行，无需安装 Python、Rust、CUDA Toolkit 或独立 aria2。** 软件与本项目发布的 DLL 均提供数字签名。本仓库公开 Rust 管理器源码、构建说明与云端维护流程；第三方组件保留各自许可。

0.3.5 原版、三角洲专用和初始 RTX20/RTX30 方案的内部后端也使用匹配的签名文件；每个补丁只嵌入一份后端。数字签名用于确认发布者与文件完整性，不代表游戏或反作弊系统自动允许加载。

## 本项目在上游基础上增加了什么

原始帧生成能力来自 [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86)。管理器同时保留上游原版和本项目扩展版，让你可以按游戏选择和回退。

### 0.3.5 · 三角洲专用 扩展版

- **补上 Vulkan 接入**：增加资源互操作、GPU 共享传输、同步与兼容回退，桥接已合入六个代理，无需额外放一个桥接 DLL。
- **减少传输开销**：复用共享资源与缓存，减少 CPU 图像中转、重复分配和等待；保留上游 **310.9 模型与推理优化**。
- **三角洲 RTX20 / CMP40HX 专项**：针对已识别设备 ID 与指定游戏主模块进行资格兼容，保持真实 CUDA 架构、显存、LUID 和注册表不变。
- **三角洲多帧选择**：在管理器内选跟随游戏、2X、3X、4X；3X/4X 使用匹配的私有运行时组件，**不覆盖游戏原有 `sl.*.dll`**。
- **更完整的清理**：每个游戏拥有独立版本缓存。卸载同时处理已确认归属的 DLL、INI、日志和组件缓存，保留未知文件与原游戏文件。

### 保留 0.2.6 · DX12/Vulkan 兼容方案

基于上游 Native 0.2.4，保留 **310.1 模型**。包含永劫无间无类型纹理 Fix1、Vulkan 互操作与传输优化，以及针对燕云十六声启动回归的 D3D12 延迟加载和进程生命周期修复。**0.2.6 是本项目方案版本号。**

用户已反馈永劫无间、三角洲、终末地、燕云十六声在相应测试配置下可用；三角洲 RTX2070 测试也反馈了帧率提升。不同显卡、驱动和游戏更新仍可能影响结果，**4X 不代表必然得到四倍 FPS，也不承诺延迟不变**。

## Dlssg-MFG-Vulkan

独立集成 [pipotoufikxyz-lgtm 的 sm86-7 发布版](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version/releases/tag/sm86-7)，只提供已签名的 `version.dll`。作者宣布新增 Vulkan（例如 RTX Remix），并注明 RTX20 尚未确认；不代表所有 Vulkan 游戏兼容。本方案与 Smooth Motion 分开，也不包含本项目三角洲专项。

预设使用独立 INI 协议：`MaxInterpolatedFrames` 默认5（最高6X），`ForceMultiplier` 默认0（跟随游戏）；提供请求2X—6X、动态多帧及目标FPS、四项优化和日志开关。动态多帧仅适用于兼容DX12路径；请求不能被当作实际呈现倍率。优化保留作者默认，可能影响画质，需实测。

原样保留 RenderScale、热键和其他高级键；不提供作者已记录GPU挂起的 Blackwell 实验开关。手改INI后可正常更新受管参数和卸载，未知文件仍保留。切换方案前先退出游戏并卸载旧补丁。

## RTX40 MFG

独立接入 [dashdogy/RTX40MFG-Unlock v1.4.1 Hotfix 1](https://github.com/dashdogy/RTX40MFG-Unlock/releases/tag/v1.4.1-hotfix.1)。选择此方案后使用 RTX40 路径，默认跟随游戏；可设置 2X—6X、动态目标、UI 预设、垂直同步与固定模式 Reflex 限帧。**游戏须原生集成 Streamline DLSS 帧生成**；Vulkan 为实验支持，不能使用动态倍率。

单个通用 DLL 按所选入口改名，保持原签名；不与 RTX20/30 的 INI 混用。`RTXMFG-Universal.json` 独立保存参数，游戏内按 **Backspace** 调出作者的英文菜单。遇到同名游戏或 MOD 文件时拒绝覆盖；Bink 入口需要按上游说明保留原始 Hooked 文件。

选中游戏后，在右侧选择方案与 DLL 加载入口。游戏行右上方显示部署信息，下方为紧凑的 **预设参数** 按钮，点击即可打开参数窗口。每款游戏分别记忆方案、入口和参数；部署标签显示实际方案、DLL 与多目录数量，悬停可查看明细。切换方案或入口前先卸载，勾选批量处理时使用各游戏自己的设置。

## 选择哪个方案

| 方案 | 接口与用途 | DLL 入口 |
| --- | --- | --- |
| **0.3.5 · Github-sdli1995（默认）** | 上游原版，D3D12 / SM75、SM86，310.9 模型 | 六入口 |
| **0.3.5 · 三角洲专用** | 本项目扩展；Vulkan 或三角洲专项 | 六入口 |
| **Dlssg-MFG-Vulkan** | 上游 sm86-7，新增 Vulkan；RTX20待实测 | version.dll |
| **RTX40 MFG · 1.4.1 Hotfix 1** | RTX40 / 原生 Streamline DLSS FG；独立 JSON | 单入口，支持 19 个名称 |
| **DLSSG-Transfusion · 1.4.5.3** | RTX20/30/40 / 原生 Streamline DLSS FG；独立 JSONC；RTX20 与旧显卡 Vulkan 验证有限 | 四个匹配代理中单选 |
| **0.2.6 · DX12/Vulkan** | 保留的兼容方案，310.1 模型 | 五入口 |
| **初始方案 · Github 第一版** | 原始能力；按 RTX20 / RTX30 选择对应文件 | version.dll |

两个 0.3.5 方案分开保留，**首次默认 Github 原版，之后记住你的选择**。“Github”是方案来源，不代表必须走国外下载线路。

六入口：`version.dll`、`winmm.dll`、`dinput8.dll`、`dbghelp.dll`、`dxgi.dll`、`d3d12.dll`。建议先选一个 `version.dll`；支持多选，先加载的代理工作，其余转发。`dxgi.dll` 与 `d3d12.dll` 二选一。0.2.6 五入口为 version / winmm / dinput8 / winhttp / dxgi。

上游 0.3.5 本身修复了帧生成特性重建时误用优化内核导致的花屏、崩溃风险，详见[上游说明](https://github.com/sdli1995/dlssg_for_sm86/releases/tag/0.3.5)。这部分归功于上游，不是本项目新增 Vulkan 的内容。

## 三角洲：使用 2X / 3X / 4X

使用 **4.2.1 或更新管理器**，完全退出游戏后：

1. 添加实际游戏本体 `DeltaForceClient-Win64-Shipping.exe`，位于 `Binaries/Win64`。
2. 选择 **0.3.5 · 三角洲专用** 与显卡系列，点击该游戏行的 **预设参数**。
3. 在 **倍率上限** 中选跟随游戏 / 2X / 3X / **4X（默认）**，点击 **安装并应用**；已部署同方案可直接 **应用参数到当前游戏**。
4. 启动游戏，开启游戏内帧生成开关。切换倍率前先退出游戏，再应用设置。

管理器联动同一份 INI，不需要手填专项键。跟随游戏/2X 保留原运行时；3X/4X 使用独立缓存。若原组件已提前加载，将保留原运行时以避免混版。**其他游戏不会启用此专项**，其通用倍率仍只是上限，不会自动增加游戏菜单。

## 快速开始

1. 下载 `RTXManager-v4.2.6-x64.exe`，在 Windows 10 / 11 x64 上运行，接受 Windows 管理员权限提示；取消授权会停止启动。
2. 点击主页 **图形设置**，在 Windows 设置 → 系统 → 屏幕 → 显示卡 → 更改默认图形设置中开启 **硬件加速 GPU 计划**，按系统提示重启。不同系统版本名称可能略有差异。
3. 退出游戏，扫描或手动添加游戏本体 EXE，选择显卡系列、方案及入口，点击 **安装并应用**。
   同一安装目录的多个有效游戏本体只显示一项；若游戏本体分布在多个目录，安装和卸载会处理该项列出的所有目录。不会自动向启动器或反作弊程序部署。扫描遗漏时可手动添加 EXE。
4. 在游戏内开启 DLSS 帧生成。点游戏行选择当前游戏，勾选复选框才进入批量操作。
5. 更换补丁版本、方案或入口时，先卸载旧补丁，再安装新方案。管理器升级不会自动替换游戏中的 DLL。

参数按 **游戏＋方案** 独立记忆。倍率直接可见，其他选项收在 **高级参数**；更改后提示 **有修改，待应用**。同方案已安装时可仅更新当前游戏的 INI，无须重复下载。批量勾选不改变这个单游戏按钮的作用范围。0.3.5 提供内核档位、倍率、UI 重组及日志；0.2.6 提供倍率、采样与日志；初始方案提供启用、倍率与日志。修改已识别部署的参数时保留其余 INI 内容及注释。0.2.6 的 Vulkan 桥接日志暂不受日志级别控制；0.3.5 扩展版详细日志遵循级别，少量启动诊断独立保留。

## 卸载、缓存与更新

0.3.5原版和三角洲专用保留上游日志目录及空的CacheDirectory（使用用户目录缓存）。如旧版安装后《绝区零》崩溃，退出游戏，在新版中选择原方案和d3d12.dll，点击安装补丁重新应用；只修正管理器旧路径，保留自定义路径。共享运行时缓存不随单个游戏卸载。

- **卸载补丁**：清理确认属于本项目的文件；支持手改 INI、多入口、重新签名后的已识别组件。游戏仍运行时会警告，不能视为卸载成功。
- **缓存待清理**：若补丁已移除但缓存占用，保留重试记录，退出相关程序后再次卸载或使用设置中的 **清理缓存**。
- **保留内容**：游戏文件、未知文件、游戏列表和偏好不随缓存清理删除；历史测试3还原备份不自动删除。移出游戏库也不等于卸载。
- **独立缓存**：三角洲组件位于 `%LOCALAPPDATA%\RTXFG-Delta4X\games\<游戏标识>\<版本>`；管理器数据在 `%LOCALAPPDATA%\RTXFGManager`。
- **云端资源**：清单和资源使用 GitHub / Gitee 双站镜像，默认 Gitee 国内优先，失败时提示原因并回退 GitHub。下载统一使用内置 aria2，展示大小、速度和进度；已校验缓存可以复用。相同安装协议的 DLL 可以独立更新。
- **软件更新**：启动后台检查，下载后校验大小、SHA-256 与发布者签名，再按新版名称替换并重启；新界面启动成功后删除旧 EXE，失败则还原。自动下载仅预先下载，替换前仍需确认本次更新。

补丁、云端清单和软件更新共用一个下载线路偏好，首次默认国内优先；手动选择 GitHub 优先后会记住。从 3.7.4 可直接升级，保留原游戏列表、偏好和部署记录。

## 使用前了解

帧生成需要游戏、驱动、系统和硬件配合；扫描发现游戏不代表已验证兼容。请遵守游戏规则，尤其确认带反作弊的游戏是否允许第三方补丁。显卡名称修改只改变 Windows 显示名称，不改变真实硬件能力，也不保证游戏采用这个名称。

支持简体中文、英语、俄语、日语、韩语，默认跟随系统并记住手动选择。完整版本变化见 [4.2.6 Release 更新日志](https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6)。

## 致谢与联系

[Github · sdli1995](https://github.com/sdli1995/dlssg_for_sm86) · [社区扩展 · pipotoufikxyz-lgtm](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version) · [GPUI](https://gpui.rs/) · [GPUI Kit](https://github.com/longbridge/gpui-kit) · [GPUI Fast](https://github.com/longbridge/gpui-fast) · [aria2](https://github.com/aria2/aria2)

新增致谢：[哔哩哔哩 · 云外逸声](https://space.bilibili.com/256887068)。

感谢 [哔哩哔哩 · 大大大怪将军阁下](https://space.bilibili.com/608531525) 协助测试，以及提供兼容性反馈的用户。

[个人网站](https://lgxng.cn/) · [GitHub · pandaligx](https://github.com/pandaligx) · [哔哩哔哩](https://b23.tv/5mHCHFn) · [第三方许可与归属](THIRD_PARTY_NOTICES.txt)

本项目与 NVIDIA、游戏厂商或上游项目无官方隶属关系。管理器许可不改变第三方组件的权利。如果它帮到了你，欢迎点一个 **Star**。

## 源码与云端维护

Rust 管理器源码按仓库许可证公开；[构建说明](BUILDING.md) 记录固定工具链、第三方工具校验和检查命令，可在另一台 Windows 电脑拉取后构建。游戏 DLL 保持各自上游归属，本仓库不包含私有 DLL 补丁源码或 NVIDIA SDK。

云端日常维护编辑 [cloud/schemes.json](cloud/schemes.json)，新资源上传到 GitHub 固定 `payloads` 预发行并使用新的不可变文件名；自动任务同步到 Gitee 独立资源库，避免影响旧客户端查询管理器最新版本。两站回下载验证后生成索引，最后更新 `cloud/catalog.json`；不用再手工上传私人网盘。步骤见 [云端维护说明](docs/cloud-publishing.md)。旧资源保留给旧客户端，管理器不会自动删除游戏里的补丁。
