# OptiScaler 0.9.97 简体中文菜单

此目录对应 ShyVortex/OptiScaler-NR-PreSR-MFG 的 `v0.9.97`，提交为 `b733e6deab371fda711622ab4bde46531af40894`。中文资源编入菜单主 DLL，八个加载入口共用同一份构建。

## 构建

1. 在上游源码根目录应用 `optiscaler-0.9.97.patch`，将本目录的 `MenuZh.h` 复制到 `OptiScaler/menu/`。
2. 使用 Python、fonttools 4.60.1 和 Noto Sans CJK SC Regular 运行 `python build_menu.py <NotoSansCJKsc-Regular.otf> <上游源码>/OptiScaler/menu`。生成器校验字体摘要、翻译字形和格式参数，生成字典和字体子集。
3. 使用 Visual Studio 2022、MSVC 14.44，在还原该版本的子模块后构建 OptiScaler 的 Release x64 目标。编译中文源码时使用 `/utf-8`。交付前保留编译原件以便核对签名后的 PE 内容。

## 验证

`verify_menu.cpp` 链接同一次构建的 ImGui、ImGui FreeType 对象和 FreeType 库。它会渲染中文字形、点击复选框、调整滑块，检查隐藏控件 ID 和含格式参数的文本。这项测试在独立进程中执行；游戏里的 Insert 菜单需要另外验证。

`tools/package_optiscaler_nr.py` 生成双站分发的中文补丁 ZIP。六份私有 Streamline DLL 复用两站已有的 `encore-added-fg-2.14.16-971b5212c874.zip`。管理器校验每份文件的摘要，并在 Windows 上验证原件的发行者签名。

## 来源

通用翻译采用 GPL-3.0 中文词表，项目在 `overlay.tsv` 中补充此版本的神经渲染等文案。字体子集沿用 SIL Open Font License；两份许可随补丁包附带。[翻译来源](https://github.com/A-XIANGQAQ/OptiScaler-DLSS5-MFG-RTX40/tree/v10.0.0-pre2) · [Noto CJK](https://github.com/notofonts/noto-cjk)
