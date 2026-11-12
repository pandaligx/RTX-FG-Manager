# RTX Encore 1.1.0 与神经渲染

适用于 RTX-FG-Manager 4.3.2。管理器升级不会自动替换游戏中的补丁。

## 开始使用

1. 退出游戏，在游戏库选中真正的游戏 EXE，选择 **RTX Encore · 1.1.0** 并安装。
2. 在游戏右侧栏找到 **DLSS神经渲染**。游戏目录中检测到 DLSS 超分组件后，选择版本并点击 **安装**。
3. **310.8.0** 为上游兼容选择，只有一个网络；**310.9.16** 为实验性双网络版本，可以选择 A（较大网络）或 B（较小网络，默认）。
4. 启动游戏并开启 DLSS 超分或 DLAA，按 **Insert** 打开 Encore 菜单，启用 Neural Rendering。默认 **Ctrl+Alt+N** 切换效果；自定义快捷键或关闭菜单后以自己的配置为准。

右侧显示名称与状态，版本下拉框独占一行，安装/卸载按钮放在下一行，避免长翻译挤压版本文字。完整安装位置和说明放在悬浮提示中，游戏列表显示 NR 标记。文件缺失和外部安装会单独提示；“已安装”表示运行库在磁盘上，不代表游戏内效果已开启。预设参数页只保留效果设置，底部应用按钮始终可见。

预设参数底栏同时固定显示恢复默认按钮；有修改时显示醒目的 **已修改，待应用**。关闭窗口会提醒选择返回应用或暂不应用并关闭。有效修改作为草稿保留，只有应用成功才清除待应用状态；恢复默认同样需要应用。

组件检测覆盖游戏根目录、Unreal Engine 深层插件/ThirdParty、Unity `_Data/Plugins`、常见 bin/x64 和自定义插件目录，不按游戏名称设白名单。检测限定在当前安装范围内，不跟随目录链接，不读取其他游戏或备份；超出扫描预算会提示检测未完成。单独发现 Streamline 包装器、FG、RR 或 NR 文件不会误判已有 SR 运行库。

NR 不包含在 Encore 主包中，按需下载约 108 / 180 MiB。运行库安装到当前游戏的补丁目录，完整缓存可复用；GitCode 优先，失败回退 GitHub。安装不会自动开启 NR，不会替换游戏的其他 NVIDIA 或 Streamline 运行库。

> 检测到 `nvngx_dlss.dll` 只是安装线索，并不证明实际运行兼容。游戏还须提供可用的 DLSS 输出与深度输入；RR 路线需要相应去噪输出。RTX 30/40 为主要目标，RTX 20 和 310.9.16 为实验性支持。NR 会增加 GPU 开销，请实际对比画质与帧率。

## 1.1.0 参数变化

| 参数 | 默认值 | 生效方式 | 说明 |
| :--- | :--- | :--- | :--- |
| `dlssSuperResolution.fastSuperResolution` | `false` | 重启游戏 | RTX 30、DX12、DLSS 310.9.x 的 M/L 模型；不适用于 RR 或 DLAA。不是逐像素相同的替代实现。 |
| `dlssPresets.dlssNrPreset` | `b` | 即时 | `a` / `b` / `auto`；310.8.0 只有一个模型，Open 引擎忽略此项。 |
| `keyboardShortcuts.hotkeyToggleNr` | `Ctrl+Alt+N` | 快捷键设置 | 切换 NR，便于比较效果。 |

旧 `neuralRendering.advanced.nrPreset` 在受管理配置迁移/保存时转换到新位置：0/3 → `auto`，1 → `a`，2 → `b`。已有新字段优先。保留 JSONC 注释、未知字段和菜单状态；不因增加功能而开启 NR、Fast SR 或其他实验性设置。

上游还更新了 310.9.16 运行库识别、重复加载保护、RTX 30 驱动适配和会话状态文件。管理器不自动安装驱动、不修改 NVIDIA 驱动配置。1.1.0 更新日志列出了 591.86 支持，Smooth Motion 指南仍有旧版本文字；以对应发行版及实际游戏测试为准。

## 备份与卸载

- 安装前将原 `nvngx_dlssnr.dll` 保存在同目录 `.rtx-fg-nr` 中，并记录该游戏与文件摘要。
- 正常卸载补丁时恢复原 NR 文件；原来没有则移除管理器安装的 NR。切换 NR 版本仍保留最初原件。
- 右侧 **卸载** 可单独移除受管理的 NR 或恢复原文件，保留 Encore 和其配置；未经管理器安装的 NR 不会被直接删除。
- 文件或备份被其他程序修改时停止覆盖并保留，提示处理冲突；不会直接删除其他 MOD。
- 清理受管理 Encore 的 `rtx-encore-logs/rtx-encore-status.json`，以及已有严格命名规则识别的日志。未知文件、手动抓取数据保留。
- 清理未完成时，右侧提供 **查看原因** 和 **重新清理**，只处理当前游戏的受管理文件。旧记录缺少原因时，重新清理会重新检测并保存原因。
- 游戏仍在后台运行时，显示进程名、PID 和路径。可以手动退出后重新检测，或确认 **结束并重试**；强制结束可能丢失未保存进度。无法核对路径或启动时间的进程须手动处理。

## 游戏库修复

- 游戏显示名称统一隐藏末尾 `.exe`，真实文件名和路径不变；Steam 标题仍优先。
- 《巫师 3》优先使用 `bin/x64_dx12/witcher3.exe` 作为名称、图标和部署目标；排除 D3D12StateObjectCompiler 等编译工具。已保存的错误目标在加载时修复，曾受管理的旧目录保留清理入口。

## 依据与验证范围

[安装](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/INSTALLATION.md) · [帧生成](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/FRAME-GENERATION.md) · [超分](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/SUPER-RESOLUTION.md) · [Smooth Motion](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/SMOOTH-MOTION.md) · [神经渲染](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/NEURAL-RENDERING.md) · [菜单](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/MENU-AND-OVERLAY.md) · [设置](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/SETTINGS.md) · [兼容性](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/COMPATIBILITY.md) · [排错](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/docs/TROUBLESHOOTING.md) · [更新日志](https://github.com/SilyNoMeta/rtx-encore/blob/v1.1.0/CHANGELOG.md)

本次检查覆盖配置、扫描、下载及文件归属恢复；查看了本机 Hub 的游戏详情与运行库选择界面，没有通过 Hub 修改真实游戏。管理器离线测试不代表游戏、GPU、画质或性能验证。
