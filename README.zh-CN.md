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
  <img alt="开发语言" src="https://img.shields.io/badge/language-Python-3776AB?logo=python&logoColor=white">
  <a href="LICENSE"><img alt="管理器许可" src="https://img.shields.io/badge/manager_license-MIT-blue"></a>
</p>

管理 RTX 20/30 系列显卡帧生成适配补丁的 Windows 工具。本仓库是**软件发行仓库**，仅提供程序、文档、截图、更新清单和发布同步配置，不公开管理器源码。第三方组件保留各自许可。

## 下载

- [GitHub Releases](https://github.com/pandaligx/RTX-FG-Manager/releases/latest)
- [Gitee Releases · 国内镜像](https://gitee.com/pandaligx/RTX-FG-Manager/releases)

下载正式 Release 中已签名的 `RTXManager-v<版本>-x64.exe`，直接运行，无需另装 Python 或 aria2。每个版本的变化和已知限制放在 Release 更新说明中。只有 EXE 和更新清单上传完整后才会提供更新，开发待签名包不属于正式发行版。

## 主要功能

- 中文、英文、俄文、日文、韩语；首次跟随 Windows 语言，手动选择后自动记忆。点击左下角地球图标或进入设置即可切换。
- 左侧图标导航，悬停显示名称；浅色、深色和跟随系统主题，适配窗口尺寸和系统缩放。
- 添加游戏 EXE，或选择目录、磁盘、全盘扫描；主页直接显示游戏库和操作日志。
- 提供 Native 0.2.4 Fix1 与原有 R2/SM86 方案。Native 支持 `version.dll`、`winmm.dll`、`dinput8.dll`、`winhttp.dll`、`dxgi.dll` 五种入口；默认测试方案和 `version.dll`。
- 可选独立 Fix1＋5X/6X 实验方案。6X 是倍率上限，实际倍率由游戏请求，不会自动增加游戏菜单；具体验证范围见 Release 说明。
- 安装及卸载本项目可识别的文件；修改 INI 后仍可正常清理，保留游戏自带和无法确认归属的其他文件。未完成操作会弹窗提醒，并显示在日志中。
- 后台检查更新，可选自动下载，内置 aria2。安装前需要确认，下载后校验完整性和管理器发布者签名，更新时保留旧 EXE 备份。

## 简单使用

1. 完全退出游戏，点击“添加游戏”选择游戏本体 EXE，或使用“扫描”旁的箭头选择范围。
2. 选中游戏，选择显卡系列和适配方案，点击安装补丁。
3. 启动游戏，在游戏中开启 DLSS 帧生成。关闭管理器不影响已安装补丁。
4. 切换方案或 DLL 入口前，先退出游戏、卸载补丁，再重新安装。建议逐个入口测试；多选属于试验功能，可能冲突。
5. 点击“卸载补丁”移除适配。如果提示游戏仍在运行，代表**尚未卸载成功**，请退出后重试。主页“使用说明”中有更详细介绍。

效果取决于游戏、显卡及驱动，扫描到游戏不代表已经验证兼容。显卡名称功能只修改 Windows 显示名称字段，不改变真实硬件能力，也不能保证游戏或任务管理器采用新名称；可重新打开相关窗口，必要时重启 Windows 检查。

## 更新和设置

默认启动后自动检查更新，软件运行期间每六小时检查一次；自动下载默认关闭，可在设置中启用。自动线路按 **Windows 系统地区**选择：中国优先 Gitee，其余地区优先 GitHub，也可手动指定。镜像不可用或版本落后时，会尝试另一站点的有效版本；这不是根据 IP 定位判断地区。

设置和游戏列表保存在 `%LOCALAPPDATA%\RTXFGManager`。更新管理器会保留偏好，不会自动替换游戏内已部署的补丁，也不会擅自关闭游戏或重启电脑。

## 致谢与联系

- [dlssg_for_sm86 · sdli1995](https://github.com/sdli1995/dlssg_for_sm86)
- [aria2](https://github.com/aria2/aria2)：独立下载工具，GPL-2.0-or-later
- [个人网站](https://lgxng.cn/) · [GitHub](https://github.com/pandaligx) · [哔哩哔哩](https://b23.tv/5mHCHFn)

组件许可及归属见 [第三方声明](THIRD_PARTY_NOTICES.txt)。管理器的 MIT 许可不会更改 NVIDIA 及其他第三方材料的许可；本项目与 NVIDIA、上述上游项目没有官方隶属关系。
