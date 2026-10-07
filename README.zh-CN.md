# RTX 帧生成管理器 · by 小南瓜

**在一个界面里，完成帧生成补丁的安装、调节与卸载。**
面向 RTX 20 / 30 / 40 的 Windows 便携工具，采用 Rust 核心与 GPUI 原生界面。

[English](README.md) · **简体中文**

<p align="center">
  <a href="https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6"><img alt="版本" src="https://img.shields.io/github/v/release/pandaligx/RTX-FG-Manager"></a>
  <a href="https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6"><img alt="下载量" src="https://img.shields.io/github/downloads/pandaligx/RTX-FG-Manager/total"></a>
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?logo=windows&logoColor=white">
  <a href="LICENSE"><img alt="管理器许可 MIT" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

- **游戏集中管理**：扫描或手动添加，单个安装或批量处理，直观看到实际部署的方案与入口。
- **每款游戏独立记忆**：保存方案、DLL 入口和预设参数，需要时再下载匹配文件。
- **部署目录更灵活**：放在游戏 EXE 旁，也可关联独立插件文件夹，卸载时按归属清理。
- **界面随你调整**：38 个离线配色可实时预览，支持简体中文、英语、俄语、日语和韩语。

## 下载

**当前版本：4.2.6** · [国内下载 · Gitee](https://gitee.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6) · [GitHub 下载](https://github.com/pandaligx/RTX-FG-Manager/releases/tag/v4.2.6) · [更新日志](CHANGELOG.zh-CN.md)

下载 **`RTXManager-v4.2.6-x64.exe`**，适用于 Windows 10 / 11 x64。发行页提供已签名 EXE 和校验清单；无需另装 Python、Rust、CUDA Toolkit 或 aria2，游戏 DLL 按需从云端获取。

<p align="center">
  <img src="https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/docs/screenshot-home-zh.png" alt="较早版本的管理器界面，使用演示游戏展示浅色与深色主题" width="980">
</p>
<p align="center">较早版本的界面示意；4.2.6 新增的左侧主题选择器见下方说明。</p>

## 选择方案

**首次默认使用 0.3.5 · Github-sdli1995，也就是上游原版。** 先选择显卡系列，管理器会保留兼容方案或选择可用方案，并禁用不支持的选项。下表显卡范围表示管理器允许的选择，不代表所有游戏都已验证兼容。

| 方案 | 管理器可选显卡 | 游戏／接口要求与用途 | DLL 入口 | 配置格式 |
| --- | --- | --- | --- | --- |
| **0.3.5 · Github-sdli1995（默认）** | RTX20 / 30 | 兼容的 D3D12 DLSS 帧生成游戏；保留上游原版与 310.9 模型 | 六入口 | INI |
| **0.3.5 · 三角洲专用** | RTX20 / 30 | D3D12/Vulkan 扩展；为识别到的三角洲游戏提供额外倍率控制 | 六入口 | INI |
| **Dlssg-MFG-Vulkan** | RTX20 / 30 | 兼容的 DLSS 帧生成路径；上游 DX12/Vulkan 多帧方案。RTX20 待实测，动态模式需兼容 DX12 | `version.dll` | 独立 INI 协议 |
| **0.2.6 · DX12/Vulkan** | RTX20 / 30 | 面向受支持 DLSS 帧生成游戏的保留兼容方案，使用 310.1 模型 | 五入口 | INI |
| **初始方案 · Github 第一版** | RTX20 / 30 | 用于兼容游戏的早期基础方案，供回退选择，参数较少 | `version.dll` | INI |
| **RTX40 MFG · 1.4.1 Hotfix 1** | 仅 RTX40 | 游戏须已有 Streamline DLSS 帧生成；Vulkan 属实验支持，动态模式仅限 DX12 | 19 个名称中单选，由管理器给通用 DLL 改名 | `RTXMFG-Universal.json` |
| **DLSSG-Transfusion · 1.4.5.3** | RTX20 / 30 / 40 | 游戏须已有 Streamline DLSS 帧生成；RTX20 及 RTX20/30 Vulkan 仍需实机验证 | 四个匹配代理中单选 | `DLSSG-Transfusion.json`（JSONC） |

- **六入口方案**：`version.dll`、`winmm.dll`、`dinput8.dll`、`dbghelp.dll`、`dxgi.dll`、`d3d12.dll`。建议先选一个，通常从 `version.dll` 开始；`dxgi.dll` 与 `d3d12.dll` 不能同时选择。0.2.6 的五入口为 version / winmm / dinput8 / winhttp / dxgi。
- **Transfusion**：只选 `version.dll`、`dinput8.dll`、`dxgi.dll`、`winmm.dll` 中一个，各入口使用匹配的独立文件，不能相互改名。固定 5X/6X 为实验选项；不捆绑可选 ASI/ReShade 组件，也不提供 Smooth Motion 开关。
- **各方案保持独立**：INI、RTX40 JSON 与 Transfusion JSONC 不能混用，即使 INI 文件同名，参数也可能不同。更换方案或入口前，退出游戏并卸载旧补丁，不要叠加帧生成补丁。

方案名称表示来源，不决定下载线路；菜单中的 **方案来源** 可打开对应上游项目。选中方案不等于已经安装，管理器也不能为任意游戏凭空增加帧生成功能。

## 四步开始使用

1. **运行 EXE**，接受 Windows 管理员权限提示；取消授权会停止启动。
2. 通过主页 **图形设置** 进入 Windows 设置，开启 **硬件加速 GPU 计划**，按系统提示重启。
3. **退出游戏，添加实际游戏 EXE**：可扫描或手动选择。选好显卡系列、方案与 DLL 入口，点击游戏行的 **预设参数**，再点 **安装并应用**。单击游戏行选择当前游戏，复选框用于批量处理。
4. **启动游戏，开启游戏内 DLSS 帧生成**。改参数前先退出游戏；已安装同一方案时，可用 **应用参数到当前游戏** 更新配置，无需再次下载 DLL。

**三角洲设置**：添加 `Binaries/Win64` 中的 `DeltaForceClient-Win64-Shipping.exe`，选择 **0.3.5 · 三角洲专用**，在预设中选择跟随游戏 / 2X / 3X / 4X，默认 4X。3X/4X 组件使用独立缓存，不覆盖游戏原有 `sl.*.dll`；这个专项只对识别到的对应游戏启用。

## 主题、目录与日常维护

**主题配色。** 点击左侧栏调色板，在同一列表中搜索、滚动浏览 38 个离线配色。悬停或方向键预览，点击/Enter 保存；Esc 或点击外部恢复原主题。跟随系统时，浅色与深色配色分别记忆。

**游戏目录。** 选中游戏后，用 **添加文件夹** 指定插件目录，文件夹本身无需包含 EXE；它仍关联真实游戏进行运行检查、参数管理与卸载，第三方加载器需要自行配置。合并后的游戏条目可能列出多个部署目录，安装前请确认显示的路径。管理器数据保存在 `%LOCALAPPDATA%\RTXFGManager`。

**下载与更新。** 默认先用 Gitee，失败时回退 GitHub；手动选择 GitHub 优先后会记住。下载显示速度与进度，已校验的 DLL 缓存可离线复用。软件更新会检查大小、SHA-256 和发布者签名，替换前仍需确认。管理器升级保留游戏库与偏好，但不会自动替换游戏中的 DLL；设置页也能离线阅读当前与历史更新日志。

**安全卸载。** 退出游戏后点击 **卸载补丁**，仅清理已确认属于该部署的文件，保留原游戏文件、未知文件和其他 MOD。文件被占用时会提示重试，关闭相关程序后再操作。从游戏库移除条目不等于卸载补丁；**清理缓存** 不会删除游戏库与偏好。

## 常见问题与限制

- **扫描到游戏不等于确认兼容。** 可用性取决于游戏的帧生成接入、图形接口、驱动和显卡。请求倍率不代表同等倍数的 FPS 提升，也不保证延迟不变；本项目不承诺性能收益。
- **RTX40 MFG 的 Backspace 游戏内菜单仍为英文。** 可在管理器的中文预设中调整倍率、动态目标、UI 预设、垂直同步与固定模式 Reflex 限帧。Bink 入口需要按上游说明保留原始 Hooked 文件。
- **数字签名不代表反作弊系统允许加载。** 请遵守游戏规则，并确认是否允许第三方补丁。管理员权限只提供文件操作权限，不改变这些规则。
- **不同方案有不同参数。** 预设旁的 **?** 提供当前方案说明，不确定时保留默认值；具体版本变化见 [更新日志](CHANGELOG.zh-CN.md)。

## 开源与致谢

Rust 管理器采用 [MIT 许可证](LICENSE)，提供 [构建说明](BUILDING.md) 与 [云端维护指南](docs/cloud-publishing.md)。游戏 DLL 保留各自许可，公开仓库不包含私有 DLL 补丁源码或 NVIDIA SDK 头文件；详见 [第三方说明](THIRD_PARTY_NOTICES.txt)。

感谢 [sdli1995 / dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86)、[pipotoufikxyz-lgtm / MFG](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version)、[dashdogy / RTX40MFG-Unlock](https://github.com/dashdogy/RTX40MFG-Unlock)、[SilyNoMeta / DLSSG-Transfusion](https://github.com/SilyNoMeta/DLSSG-Transfusion)、[GPUI](https://gpui.rs/)、[GPUI Kit](https://github.com/longbridge/gpui-kit)、[GPUI Fast](https://github.com/longbridge/gpui-fast) 与 [aria2](https://github.com/aria2/aria2)。

也感谢 [大大大怪将军阁下](https://space.bilibili.com/608531525)、[云外逸声](https://space.bilibili.com/256887068) 和提供兼容性反馈的用户。本项目与 NVIDIA、游戏厂商及上游项目无官方隶属关系。

[个人网站](https://lgxng.cn/) · [哔哩哔哩](https://b23.tv/5mHCHFn) · [GitHub](https://github.com/pandaligx/RTX-FG-Manager) · [Gitee](https://gitee.com/pandaligx/RTX-FG-Manager)
