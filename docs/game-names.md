# 游戏名称 / Game names

中文界面按“安装记录 → 本机中文元数据 → 已核实组合规则”识别游戏名称。右侧“改名”可查找中文/英文候选、直接输入或恢复自动名称。手动名称始终优先，不被扫描、云端更新或重启覆盖；无法确认时保留原名。名称只用于显示和搜索，不改游戏文件、安装路径、补丁归属或启动方式。

Chinese naming uses launcher records, local localized metadata, then reviewed identification rules. **Rename** searches Chinese/English candidates or accepts a local custom name. Manual names always win. Uncertain matches retain the original name. Naming never changes game files, installation paths, deployment ownership or launch behavior.

## 读取范围 / Local evidence

| 来源 / Source | 读取内容 / Evidence |
|---|---|
| Steam | `appmanifest_*.acf`：核对 App ID、安装目录、标题；区分同一合集中的独立子游戏。 |
| Epic | `.item` 与 `LauncherInstalled.dat`：安装位置、namespace/item、名称和启动目标；过滤明确的 DLC。namespace/item 不猜成商店网址 slug。 |
| EA | 安装注册表与 `__Installer/installerdata.xml`：content ID、原名和已有中文标题。 |
| GOG / Ubisoft | GOG `.info` 与注册表，Ubisoft `Launcher/Installs` 和卸载记录：ID、目录、目标及可用名称。 |
| Xbox | `MicrosoftGame.config`：StoreId、声明的游戏目标和显示名称。读取身份不绕过受保护 EXE 的权限。 |
| Battle.net | 本地 `Agent/product.db`、`.build.info`：产品 ID 和安装路径；多产品冲突时不猜测。 |
| WeGame / 米哈游 / 独立客户端 | 可用安装注册表、Rail 目录标记、游戏配置与 Unity 元数据。主程序名必须再结合目录、文件或产品证据；包含绝区零、燕云十六声、终末地、永劫无间、三角洲行动、云顶之弈等规则。 |
| 方块 / 杉果 / 其他 | 使用通用卸载注册表、本机中文快捷方式和产品元数据；本版没有宣称支持其所有专有安装数据库。购买后经 Steam 安装的游戏按 Steam 记录识别。 |

读取到平台 ID **不等于**名称库已有该 ID 的中文映射。EA、Ubisoft、Epic 等记录可能只提供英文；缺少已核实映射时仍使用本机中文证据、组合规则或手动改名。本版不承诺所有平台、地区、版本和游戏的自动覆盖。

Registry records must match the current installation or executable. Chinese shortcuts are accepted only when their target is exactly that game's executable with no launcher arguments. Product names are read from version resources and Unity metadata. The adapters are bounded and read-only; they do not launch shortcuts, change permissions, read account databases or upload local paths. Identifying a platform ID does not imply that a Chinese mapping exists for it. Unknown editions and conflicting records remain unresolved.

## 名称库与独立更新 / Catalog and updates

- 内置约 5,300 条 [Wikidata CC0](https://www.wikidata.org/wiki/Wikidata:Data_access) 记录，保留 Q ID。它不是官方译名全集，也不代表游戏支持 DLSS；少量常用译名通过审核修正。
- 离线可用；启动读取缓存，每天最多自动尝试一次更新，设置中也可手动更新。GitCode 优先、GitHub 回退，并尊重用户线路选择。只请求公共名称库，不提交游戏列表、硬件、用户名或目录。
- 数据与规则共用版本化名称包，规则只描述 EXE、目录、文件、产品信息和 ID 条件，不包含可执行代码、脚本或任意路径。客户端校验摘要、大小、版本、格式及身份冲突后原子替换；失败保留旧库。
- GitHub Actions 每周自动刷新公开名称数据，并在同一流程同步 GitCode。无变化不提交；数据明显缺失、大批重命名或映射冲突时停止，保留旧库。人工审核的特殊规则和译名修正保留，不由网络文本自动生成。

The bundled CC0 catalog works offline. Clients check at most daily and retain the last valid cache on failure. GitHub's weekly workflow refreshes public names and explicitly mirrors to GitCode, since token-created commits do not trigger another push workflow. Curated rules remain reviewed source data. No-change runs create no new revision; incomplete feeds and suspicious bulk changes are rejected. Special launcher formats and new local identification rules can still require maintenance; there is no promise of a universal self-learning translation service.

**4.3.3 发布状态：**名称包和每周自动流程已上线。上游访问失败时保留现有名称库，后续任务会再次尝试；离线识别不受影响。

**4.3.3 release status:** the name package and weekly workflow are published. Failed upstream requests retain the existing catalog for later retry; offline identification remains available.

## 维护与参考 / Maintenance and references

`tools/build_game_names.py` 从 Wikidata 查询结果生成内置库、不可变 `cloud/names/names-<digest>.json` 和独立 `cloud/game-names.json`；`--fetch` 获取公开数据。`tools/game-name-overrides.json` 保存审核过的译名、ID 补充与规则。`.github/workflows/update-game-names.yml` 负责周期运行；签名 EXE 不通过此流程上传。初次发布应将数据与清单一同提交，后续保留旧不可变文件。

本机读取格式参考：[Windows 卸载记录](https://learn.microsoft.com/en-us/windows/win32/msi/uninstall-registry-key)、[Windows Shell Links](https://learn.microsoft.com/en-us/windows/win32/shell/links)、[Playnite 启动器适配器](https://github.com/JosefNemec/PlayniteExtensions/tree/master/source/Libraries)、[Lutris EA 适配器](https://github.com/lutris/lutris/blob/master/lutris/services/ea_app.py)。本项目自行实现必要的有界读取，不接入这些软件的账户或私人服务。
