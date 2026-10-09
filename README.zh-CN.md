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

### 4.2.9 更新：RTX Encore 与 RTX MFG 1.4.2

4.2.9 适配 **[RTX40 MFG · 1.4.2](https://github.com/dashdogy/RTX40MFG-Unlock/releases/tag/v1.4.2)**，并将原 **DLSSG-Transfusion 1.4.5.3** 方案升级为 **[RTX Encore 1.0.0-beta.2](https://github.com/SilyNoMeta/rtx-encore)**，保留每游戏的方案选择。

- **RTX MFG 1.4.2 改善《巫师 3》实验性路径追踪毛发的兼容检测**，解决游戏更新后选项不可用的问题。该游戏使用 `winmm.dll`，放在 `witcher3.exe` 旁。管理器仍仅为 RTX40 提供此方案；Vulkan 仍属实验且无动态 MFG，菜单按 **Backspace**。
- **RTX MFG 升级可保留设置**：退出游戏后点 **安装并应用**，适用于有合法管理器记录、同方案、同 DLL 入口的旧版升级新版。原安装须通过归属校验；遇到未知改动时保护现有文件。更新管理器本身不会自动替换游戏 DLL。

- **Encore 菜单按 Insert** 开关，首次启动自动打开一次，可在菜单改键。内置菜单支持 DX11/DX12/Vulkan，无需 ReShade；RTX40 MFG 的菜单键仍是 Backspace。
- 管理器提供完整 JSONC v4 高级参数，保留注释、未知键和菜单状态；只保存明确修改的字段，其他值以最新磁盘配置为准。退出游戏后，对有合法管理器记录的旧 Transfusion 使用 **安装并应用** 即可事务升级，不要先卸载而丢失设置。原 DLL、配置和记录先备份，失败恢复原件；未知改动和未完成的恢复资料会保留。
- 一个已签的 `rtx-encore.dll` 原样复制改成所选名称即可，签名不变，无需每个别名重复签名。这不适用于另编译的 `alternative-proxies` 或 ASI 文件；本版只部署 19 个 DLL 入口，不部署 ASI。改名只决定加载方式，不增加图形 API 支持。
- **NR 默认关闭**，需要用户自行将 NVIDIA `nvngx_dlssnr.dll` **310.8.0** 放在补丁旁并开启游戏 DLSS SR/DLAA；管理器不分发该 NVIDIA 文件。NVIDIA/Open 引擎、多遍、精度和实验选项可能增加 GPU 耗时或显存、改变画面，按参数提示重启。RTX20 路径仍为实验，不承诺性能收益。
- **Smooth Motion 默认关闭**，此解锁路径仅支持 RTX30，驱动须为 **617.42、617.14、616.92 或 616.64**。上游只报告 **617.14** 的实际游戏验证，其余三个不能视为已游戏验证。选择游戏实际 API 后重启；“Driver prepared”和估算 FPS 不等于插值帧已实际显示。

五语言离线帮助精简为八节，按上手、选方案、升级和排障组织，保留目录、滚动与复制。

上游能力与限制：[安装](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/INSTALLATION.zh-CN.md) · [菜单](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/MENU-AND-OVERLAY.zh-CN.md) · [设置](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/SETTINGS.zh-CN.md) · [NR](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/NEURAL-RENDERING.zh-CN.md) · [Smooth Motion](https://github.com/SilyNoMeta/rtx-encore/blob/main/docs/SMOOTH-MOTION.zh-CN.md)。加载检查与离线测试不等于目标游戏或 GPU 验收。

### 4.2.8 更新内容

相比 **4.2.7**，本版修复六项管理器问题：

- 自定义插件目录检查同一游戏的全部关联 EXE 进程，覆盖安装、参数修改和卸载。
- DLL 准备与软件更新共享缓存时可等待、取消；等待超时后提示重试。
- 扫描、文件选择或部署期间保留最新云端目录，操作结束后应用，不改变执行中的配置。
- 相同数量重扫造成游戏顺序或名称变化时，立即刷新搜索结果。
- 卸载仍有待核验临时文件或待清理缓存时，报告未完成并显示警告，保留记录供重试。
- 排除带 `Win64-Shipping` 等构建后缀的 Unreal 服务端，避免误列为游戏。

仅为 **0.3.5 · 三角洲专用** 新增 **RTX40（SM89）** 选择，沿用已有签名 DLL，不重建 DLL、不修改设备 ID。Github 上游原版仍为默认，其他方案的显卡范围不变。五语言离线使用说明整理为七节 Markdown 指南，提供目录、步骤、提示、复制和窄窗口布局。

<p align="center">
  <img src="https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/docs/screenshot-home-zh.png" alt="4.2.8 管理器浅色主题，使用演示游戏库" width="49%">
  <img src="https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/docs/screenshot-home-themes.png" alt="4.2.8 管理器深色主题，使用演示游戏库" width="49%">
</p>
<p align="center">4.2.8 实际界面，使用演示游戏库展示浅色与深色主题。</p>

## 选择方案

**首次默认使用 0.3.5 · Github-sdli1995，也就是上游原版。** 先选择显卡系列，管理器会保留兼容方案或选择可用方案，并禁用不支持的选项。下表列出 **4.2.9** 可选方案，包含 Encore 与 RTX MFG 1.4.2。显卡范围表示管理器允许的选择，不代表所有游戏都已验证兼容。

| 方案 | 管理器可选显卡 | 游戏／接口要求与用途 | DLL 入口 | 配置格式 |
| --- | --- | --- | --- | --- |
| **0.3.5 · Github-sdli1995（默认）** | RTX20 / 30 | 兼容的 D3D12 DLSS 帧生成游戏；保留上游原版与 310.9 模型 | 六入口 | INI |
| **0.3.5 · 三角洲专用** | RTX20 / 30 / 40 | D3D12/Vulkan 扩展；为识别到的三角洲游戏提供额外倍率控制 | 六入口 | INI |
| **Dlssg-MFG-Vulkan** | RTX20 / 30 | 兼容的 DLSS 帧生成路径；上游 DX12/Vulkan 多帧方案。RTX20 待实测，动态模式需兼容 DX12 | `version.dll` | 独立 INI 协议 |
| **0.2.6 · DX12/Vulkan** | RTX20 / 30 | 面向受支持 DLSS 帧生成游戏的保留兼容方案，使用 310.1 模型 | 五入口 | INI |
| **初始方案 · Github 第一版** | RTX20 / 30 | 用于兼容游戏的早期基础方案，供回退选择，参数较少 | `version.dll` | INI |
| **RTX40 MFG · 1.4.2** | 仅 RTX40 | 游戏须已有 Streamline DLSS 帧生成；Vulkan 属实验支持，动态模式仅限 DX12 | 19 个名称中单选，由管理器给通用 DLL 改名 | `RTXMFG-Universal.json` |
| **RTX Encore · 1.0.0-beta.2** | RTX20 / 30 / 40 | FG、DLSS SR、NR 与 Smooth Motion 各有独立要求；菜单支持不等于全部功能支持 | 19 个名称中单选，由同一已签 DLL 改名 | `rtx-encore.jsonc`（v4） |

- **六入口方案**：`version.dll`、`winmm.dll`、`dinput8.dll`、`dbghelp.dll`、`dxgi.dll`、`d3d12.dll`。建议先选一个，通常从 `version.dll` 开始；`dxgi.dll` 与 `d3d12.dll` 不能同时选择。0.2.6 的五入口为 version / winmm / dinput8 / winhttp / dxgi。
- **Encore 的 19 个名称**：`version.dll`、`dinput8.dll`、`winmm.dll`、`dxgi.dll`、`d3d9.dll`、`d3d10.dll`、`d3d11.dll`、`d3d12.dll`、`dsound.dll`、`wininet.dll`、`winhttp.dll`、`binkw64.dll`、`bink2w64.dll`、`xinput1_1.dll`、`xinput1_2.dll`、`xinput1_3.dll`、`xinput1_4.dll`、`xinput9_1_0.dll`、`xinputuap.dll`。只选一个。Bink 原文件分别保留为旁边的 `binkw64Hooked.dll` / `bink2w64Hooked.dll`，这两个是原件名称，不是新入口；管理器不会自动改名、覆盖或删除它们。固定 5X/6X 仍为实验。
- **各方案保持独立**：INI、RTX40 JSON、旧 Transfusion JSONC 与 Encore JSONC v4 不能混用。旧 Transfusion 的事务升级按上节操作；切换其他方案或入口前，退出游戏并卸载旧补丁，不叠加帧生成补丁。

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

**下载与更新。** 检查更新直接读取固定清单，不依赖容易限流的 Gitee Release API。默认先用 Gitee，失败时回退 GitHub；手动选择 GitHub 优先后会记住。下载显示速度与进度，已校验的 DLL 缓存可离线复用。软件更新会检查大小、SHA-256 和发布者签名，替换前仍需确认。管理器升级保留游戏库与偏好，但不会自动替换游戏中的 DLL；设置页也能离线阅读当前与历史更新日志。

**安全卸载。** 退出游戏后点击 **卸载补丁**，仅清理已确认属于该部署的文件，保留原游戏文件、未知文件和其他 MOD。文件被占用时会提示重试，关闭相关程序后再操作。从游戏库移除条目不等于卸载补丁；**清理缓存** 不会删除游戏库与偏好。

## 常见问题与限制

- **扫描到游戏不等于确认兼容。** 可用性取决于游戏的帧生成接入、图形接口、驱动和显卡。请求倍率不代表同等倍数的 FPS 提升，也不保证延迟不变；本项目不承诺性能收益。
- **RTX3060 6GB 的已反馈显存问题在 4.2.9 中仍未解决。** 新增 RTX40 选择不代表全部硬件已验证兼容或性能。
- **RTX40 MFG 的 Backspace 游戏内菜单仍为英文。** 可在管理器的中文预设中调整倍率、动态目标、UI 预设、垂直同步与固定模式 Reflex 限帧。Bink 入口需要按上游说明保留原始 Hooked 文件。
- **数字签名不代表反作弊系统允许加载。** 请遵守游戏规则，并确认是否允许第三方补丁。管理员权限只提供文件操作权限，不改变这些规则。
- **不同方案有不同参数。** 预设旁的 **?** 提供当前方案说明，不确定时保留默认值；具体版本变化见 [更新日志](CHANGELOG.zh-CN.md)。

## 开源与致谢

Rust 管理器采用 [MIT 许可证](LICENSE)，提供 [构建说明](BUILDING.md) 与 [云端维护指南](docs/cloud-publishing.md)。游戏 DLL 保留各自许可，公开仓库不包含私有 DLL 补丁源码或 NVIDIA SDK 头文件；详见 [第三方说明](THIRD_PARTY_NOTICES.txt)。

感谢 [sdli1995 / dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86)、[pipotoufikxyz-lgtm / MFG](https://github.com/pipotoufikxyz-lgtm/dlssg_for_sm86-MFG-version)、[dashdogy / RTX40MFG-Unlock](https://github.com/dashdogy/RTX40MFG-Unlock)、[SilyNoMeta / RTX Encore（原 DLSSG-Transfusion）](https://github.com/SilyNoMeta/rtx-encore)、[GPUI](https://gpui.rs/)、[GPUI Kit](https://github.com/longbridge/gpui-kit)、[GPUI Fast](https://github.com/longbridge/gpui-fast) 与 [aria2](https://github.com/aria2/aria2)。

也感谢 [大大大怪将军阁下](https://space.bilibili.com/608531525)、[云外逸声](https://space.bilibili.com/256887068) 和提供兼容性反馈的用户。本项目与 NVIDIA、游戏厂商及上游项目无官方隶属关系。

[个人网站](https://lgxng.cn/) · [哔哩哔哩](https://b23.tv/5mHCHFn) · [GitHub](https://github.com/pandaligx/RTX-FG-Manager) · [Gitee](https://gitee.com/pandaligx/RTX-FG-Manager)
