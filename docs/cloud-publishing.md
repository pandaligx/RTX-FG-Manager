# DLL 云端维护

日常只维护 GitHub 上的 [`cloud/schemes.json`](../cloud/schemes.json)。发布助手从本机上传两站 DLL ZIP，GitHub Actions 负责双站回下载核验、自动生成摘要索引和 `catalog.json` 并同步配置。软件默认访问国内 Gitee，失败后切换 GitHub；手动选择 GitHub 优先仍有效。

配置和源码在两站的 `pandaligx/RTX-FG-Manager` 主仓库；GitHub DLL 放主仓库的 `payloads` 预发行，Gitee DLL 放独立的 `pandaligx/RTX-FG-Manager-payloads` 资源仓库。这由同一工作流管理，不需要维护第二份配置。实际预检发现 Gitee 的资源预发行仍会抢占同仓库 `/releases/latest`，因此必须隔离资源库，保护旧管理器的软件升级。

此流程管理 DLL 资源，不会自动发布管理器 EXE。EXE 继续先签名，再上传、回下载验证，最后发布 `update.json`。

Gitee 的 DLL ZIP、管理器和 aria2 等工具 EXE 均优先由发布助手从本机上传，避免海外运行器向国内上传较慢或超时；用户负责签名，无须例行手动上传。Actions 复用已上传的同名同内容 ZIP，继续执行完整回下载与元数据发布门槛。已有脚本的缺包上传能力保留用于兼容，日常发布应先在本机补齐附件再触发工作流。不得把 EXE 塞进 ZIP 规避发布分工。

## 管理器更新清单（4.2.7 起）

客户端默认读取 `https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/update.json`，失败后读取 GitHub 对应固定文件，再尝试旧 GitHub Release 清单；不再访问 Gitee Release API。下载正文仍使用内置 aria2，程序文件仍需通过摘要、版本和签名检查。

仓库根目录 `update.json` **不是候选版配置**，只代表已经完成发布核验的正式版。签名前保持它指向上一正式版。不要因为修改 Cargo 版本或更新 README 就手动提升该文件。

发行镜像工作流完成已签名 EXE 与小附件的双站核验后，由 `promote_static_update` 将发行附件的同一份 JSON 提交到 GitHub、快进同步到 Gitee，并核对两站固定 URL。普通源码镜像不能提升、删除或回退该文件；同版本摘要不同会停止。工作流需要本仓库 `contents: write`，Gitee 令牌仍只用于发布端，绝不能内置到客户端。

网络中断时继续相同版本的未完成步骤；如果只是 raw 缓存尚未刷新，只需重新回读固定 URL，不能重建或覆盖已签名 EXE。较老版本的发行任务不能回退较新的固定清单。旧 Release 附件 `update.json` 继续保留，供旧管理器使用。

## 更新一次 DLL

1. 对新 DLL 完成相应验证和签名，明确记录是否做过真实游戏测试，不将离线验证等同于游戏验证。旧协议的每个 ZIP 仍严格只有两个成员：一个代理 DLL 和匹配配置；INI 方案使用 `dlssg_sm86.ini`，RTX40MFG 使用 `RTXMFG-Universal.json`，旧 Transfusion 使用 `DLSSG-Transfusion.json`。Encore 专用协议严格要求下节说明的三个成员，额外携带上游许可。所有文件放在 ZIP 根目录，不打包日志、缓存、原游戏 DLL 或子文件夹。
2. 给 ZIP 新名称，例如 `upstream-0.3.6-version-r1.zip`。由助手从本机上传到 GitHub 主仓库和 Gitee 资源仓库各自的 **`payloads` 固定预发行**；保留其他附件。同名不同内容不允许覆盖；修改 INI 或重新签名，也需要新 ZIP 名称。
3. 编辑 `cloud/schemes.json`，修改对应方案的 `version`、`name` 和 `archives`。已有方案的 `id` 保持不变，以保留用户参数记忆。参数协议没有变化时保留 `profile`。
4. 提交后查看 **Publish verified DLL resources** 工作流。它核验 ZIP 内容和两站已有附件，匿名回下载核对，再发布索引，最后发布正式 catalog。若海外传输失败，保留旧清单，从本机补齐缺少的 ZIP 后重跑；不得跳过摘要核验。
5. 工作流成功后，在管理器中刷新方案并试装。旧 ZIP 先保留；不能仅因新版本发布就删除仍被旧清单或客户端缓存引用的文件。

只调整方案名称、排序或默认参数时，跳过上传 ZIP，直接编辑方案清单。不要手改自动生成的 `cloud/catalog.json` 或 `cloud/indexes/`。

```json
{
  "schema": 1,
  "default_scheme": "example-stable",
  "schemes": [
    {
      "id": "example-stable",
      "name": "示例稳定方案",
      "profile": "upstream035",
      "version": "0.3.5",
      "defaults": { "max_generated_frames": "3", "logging_level": "1" },
      "archives": ["example-0.3.5-version-r1.zip"]
    }
  ]
}
```

上例用于说明格式，不要直接覆盖生产清单。`max_generated_frames=3` 表示最多生成 3 帧，即 4X 上限，游戏实际请求与最终帧率仍取决于游戏。

## 字段和协议

| 字段 | 作用 |
| --- | --- |
| `default_scheme` | 新用户默认方案，必须对应一个方案 ID |
| `id` | 稳定身份；改名时不要修改它 |
| `name` / 可选 `names` | 中文名称 / 按语言代码指定其他语言名称 |
| `version` | ZIP 内 DLL 方案版本，三段数字 |
| `upstream_version` | 可选完整上游版本；Encore 候选填写 `1.0.0-beta.2`，保留预发行后缀，数字主版本须与 `version` 一致 |
| `profile` | 管理器已支持的参数协议（INI、JSON 或 JSONC），不是展示名称 |
| `defaults` | 新配置默认值；留空对象表示使用该协议内置默认值 |
| `archives` | 固定资源 Release 的 ZIP 文件名，不需要手填 URL 或 SHA-256 |
| `min_manager_version` | 可选最低管理器版本；较老的新客户端会跳过该方案并提示升级 |
| `capabilities` | 仅专项构建使用；不要给普通上游 DLL 添加三角洲能力标记 |

当前协议为 `upstream035`、`upstream031`、`native026`、`initial`、`mfg_vulkan_sm86_7`、`rtxmfg_universal_133`、`transfusion_json_v3`，4.2.9 候选另支持 `rtx_encore_json_v4`。新 DLL 若改变参数字段或功能含义，需要先适配管理器和发布工具，不能只改版本号。

4.2.9 候选的 **RTX40 MFG · 1.4.2** 保留方案 ID `rtx40mfg-1.3.3-hf2` 与 `profile: "rtxmfg_universal_133"`，`version` 更新为 `1.4.2`。参数协议兼容，`min_manager_version` 继续为 `4.2.6`，不要随本次管理器候选版本上调。只上传一个 ZIP：将已签的通用 DLL 原样命名为 `version.dll`，与 `RTXMFG-Universal.json` 放在根目录。管理器按用户所选入口改名，不修改签名字节。不要按 19 个支持名称复制上传，也不要加入 `dbghelp.dll`。默认参数可留空；可写 `rtx_mode`（`follow`、`1`—`6` 或 `dynamic`）、`rtx_target`（`0`—`1000`，0 跟随刷新率）、`rtx_preset`（`0`、`1`、`2`）。这些键仅用于云端预设，不是 JSON 原始字段名。

包含内嵌后端的方案，先签内部后端，再嵌回外层、更新对应资源摘要，最后签外层 DLL。嵌入操作会使旧外层签名失效。签名有效不等于所有游戏或反作弊允许加载。

`initial` 方案的两条 GPU 路由使用 `{ "file": "文件.zip", "gpu": "rtx20" }` 和 `rtx30`。兼容既有包身份时允许可选 `id`。其他方案通常直接写文件名即可。所有整数和开关默认值使用字符串，例如 `"1"`。

## 4.2.9 候选：Encore 通用包协议

此节描述尚未发布的候选格式，不表示正式资源和管理器已经上线。Encore 接替原 Transfusion 时保留稳定方案 ID `dlssg-transfusion-1.4.5.3`，但必须切换到独立 `profile: "rtx_encore_json_v4"`，并明确设置 `min_manager_version: "4.2.9"`。旧客户端跳过新协议，不能把 Encore 包当成旧 Transfusion 配置使用。方案条目示例如下，不要用它单独覆盖完整生产清单：

```json
{
  "id": "dlssg-transfusion-1.4.5.3",
  "name": "RTX Encore · 1.0.0-beta.2",
  "profile": "rtx_encore_json_v4",
  "min_manager_version": "4.2.9",
  "version": "1.0.0",
  "upstream_version": "1.0.0-beta.2",
  "source_url": "https://github.com/SilyNoMeta/rtx-encore",
  "defaults": {},
  "archives": ["rtx-encore-1.0.0-beta.2-v429-universal.zip"]
}
```

`version` 继续使用客户端兼容的三段数字；`upstream_version` 保留完整上游版本并贯通资源索引、准备结果和部署记录，不能把 beta.2 显示为正式 1.0.0。索引中的 backend 为 `encore`，对应 `encore_json` 策略，允许 SM75 / SM86 / SM89 选择；这些路由不代表各显卡或游戏已经实测。

每个 Encore 方案恰好一个通用 ZIP，根目录严格只有以下三个成员：

| 成员 | 来源与约束 |
| --- | --- |
| `version.dll` | 已签名 `rtx-encore.dll` 的同字节副本，只改文件名，不修改代码、导出表、资源或签名 |
| `rtx-encore.jsonc` | 真实 schema 4 默认配置，`configVersion` 为整数 `4`，已知字段必须使用完整正确的嵌套路径、类型和范围 |
| `rtx-encore-THIRD-PARTY-NOTICES.md` | 上游 `THIRD-PARTY-NOTICES.md` 原文，仅改名；部署时随 DLL 保留，不省略或改写许可内容 |

包内初始配置保持 `frameGeneration.mode="game"`、`general.gpuSeries="auto"`、`smoothMotion.smoothMotionEnabled=false`、`neuralRendering.core.nrEnabled=false`。使用真实模板和共享字段元数据校验，不能把上游设置文档的摘要表当作完整 JSON 层级，也不能凭猜测生成高级默认值。包、成员大小和 SHA-256 均由工具生成并校验，不手填摘要。旧协议仍只接受两个成员，不能为了带许可而随意给旧包增加第三个文件。

客户端下载这一份包后，把唯一 canonical `version.dll` 原样重映射为所选入口，每次只允许一个：`version.dll`、`dinput8.dll`、`winmm.dll`、`dxgi.dll`、`d3d9.dll`、`d3d10.dll`、`d3d11.dll`、`d3d12.dll`、`dsound.dll`、`wininet.dll`、`winhttp.dll`、`binkw64.dll`、`bink2w64.dll`、`xinput1_1.dll`、`xinput1_2.dll`、`xinput1_3.dll`、`xinput1_4.dll`、`xinput9_1_0.dll`、`xinputuap.dll`。不要打包或上传十九份副本，也不要混入 ASI、独立 `alternative-proxies` 构建、NVIDIA NR DLL 或游戏原文件。Bink 原件须由用户预先在旁边保留为相应 `binkw64Hooked.dll` / `bink2w64Hooked.dll`，不属于云包成员。改名保持同一签名内容，但入口名称不增加图形 API 支持。

发布顺序仍为：本地生成候选并核验签名与内容 → 两站不可变 ZIP 上传及完整回下载校验 → 两站索引发布及回读 → catalog 提升及客户端固定 URL 回读 → 已签管理器 EXE 双站上传与核验 → 最后发布 `update.json`。资源验证失败时保留原正式 catalog/index；本地候选生成不等于发布。正式资源尚未提升时，旧缓存或在线刷新可能仍返回 Transfusion，不应为掩盖候选状态绕过正式清单门槛。原历史 ZIP 和不可变索引继续保留。

## 安全顺序和失败恢复

- GitHub 的 `payloads`、`build-tools` 使用预发行并设置不成为 latest。Gitee 使用独立资源库，工具在创建前后核对原管理器仓库 latest 不变，避免旧管理器把资源当软件更新。
- 不重建、不重签 ZIP 内 DLL。发布器检查 x64 DLL 格式、成员路径、大小和摘要；这不能代替 Windows 签名验证或真实游戏测试。
- 附件分页读取；同名同内容可重复验证，同名不同内容立即停止，不强制覆盖文件或标签。
- 最多并行处理 4 个不同 ZIP，每个 ZIP 分别完成两站上传和回读；不会并发写同名附件。等待时持续输出阶段与耗时，任一失败停止安排新附件，已完成上传可在重跑时复用，清单不提前发布。上传请求最多等待 15 分钟，其他 API 请求 90 秒；首次完整迁移任务最多 180 分钟。
- 每次发布先分页读取两站附件快照，已有附件不再逐包重复查询 API；新增附件上传前再次确认名称。Gitee 正文读取最多 2 路并发，429/临时网络错误或返回 HTML 挑战页最多尝试 3 次并输出安全诊断。真实文件的摘要冲突立即停止，不将冲突当成网络故障重试。
- 两站 ZIP 全部回下载一致后才写不可变索引。索引在两站发布并回读后，才提交 catalog；镜像中断不会把缺失资源写入新清单。
- 两站 Git 提交无法跨服务同时生效。若最后一个 catalog 镜像步骤失败，暂时可能一站新、一站旧，两份清单的资源均保留；修复网络后手动重跑工作流即可。
- 新文件推送成功后，Gitee raw 可能短暂返回旧的 404 缓存。只针对刚推送的精确索引/catalog 地址，发布器最多回读 5 次、每次 20 秒，中间等待 3/8/15/30 秒；返回内容与预期不一致时立即停止，不靠等待放宽摘要校验。
- 回退版本时恢复 `schemes.json` 中上一组 ZIP 与默认值并运行同一工作流。不要删除新 ZIP，也不要覆盖旧索引。
- 不再需要日常维护私人网盘。迁移验收之前仍保留旧云端，保证旧客户端先通过原来的软件更新通道升级。

## 本地检查与首次迁移

仅本地生成，不上传：

```text
python tools/cloud_release.py self-test
python tools/cloud_release.py prepare --archives "已签名ZIP目录" --out cloud-candidate
```

`--archives` 可以重复指定。`cloud-candidate` 是候选输出，不应直接复制到正在服务的仓库代替双站验证。

首次迁移可手动运行资源工作流，在 `seed_release` 填已有 GitHub 资源标签。工具复用经过验证的原 ZIP，不重打包。Gitee 附件能力须先通过 **Verify Gitee resource attachments** 工作流：它只复制一个指定摘要的公开资源并匿名回下载，不触碰生效清单。

如果首次创建 Gitee 独立资源库的 API 返回 403，使用已登录网页创建一次公开的 `pandaligx/RTX-FG-Manager-payloads` 并初始化 README，再重跑工作流。后续无需重复建库；Gitee 附件由助手本机上传。已有仓库的所有者或公开状态不符时，脚本会停止，不擅自修改权限。

GitHub 自动令牌来自工作流；Gitee 令牌只放仓库 Secret `GITEE_TOKEN`。不要放入 URL、JSON、日志或源码。日常发布不需要在本机保存令牌。

## 传输边界

管理器的清单、索引、DLL ZIP 和 EXE 正文均由内置 aria2 下载。元数据单连接、有大小限制，文件校验后才使用；已有 ZIP 缓存校验通过可离线复用。下载器先检查 HTTPS 响应头与跳转，再运行 aria2，TLS 证书检查始终开启。响应头预检与后续正文请求是两次请求，不能当作 aria2 内每次跳转都具备可编程校验的保证。

国内优先是站点顺序，不会关闭系统 VPN，也不保证所有运营商速度一致。迁移是否可用，以匿名下载、摘要、Range 与实际无 VPN 网络测试结果为准。

## 4.2.6 起：方案协议与更新日志维护

新增 Transfusion 时使用 `profile: "transfusion_json_v3"`、`min_manager_version: "4.2.6"`。每个ZIP只放一个匹配的精确导出DLL与 `DLSSG-Transfusion.json`（JSONC v3），四个入口分包、单选。不要把通用备用构建作为同一入口的第二个资源，也不要混用RTX40 JSON。云端默认保留GPU自动识别，Smooth Motion关闭；新字段需要管理器协议支持后再使用。

RTX40 1.4.1 Hotfix1继续使用兼容的 `rtxmfg_universal_133` 协议；方案ID保持稳定以保留每游戏记忆。展示版本和ZIP文件名更新，旧不可变附件继续保留。`source_url` 可选字段用于记录上游GitHub项目，不能用它改变下载线路；新方案的显卡支持范围由对应管理器协议验证。

每次更新同时维护中英文CHANGELOG、README候选/正式状态、`rust/assets/release-notes.json`的五语言当前版说明、五语言使用说明和参数帮助。未发布的候选写入Unreleased，不添加不可下载的正式EXE链接；发布验证完成后再改正式状态。

软件内的历史说明保存在 `rust/assets/release-history.json`，与 EXE 一同打包，离线可按版本查看。结构为倒序数组：`[{"version":"4.2.5","notes":{"zh-CN":"纯文本说明","en":"Release notes","ru":"…","ja":"…","ko":"…"}}]`。当前候选版由 `release-notes.json` 提供，不能同时重复出现在历史数组中。

开始下一版时，先核对上一版已正式发布，再把上一版的最终当前说明滚入历史，随后编写新候选说明；尚未发布的候选不得伪装成正式历史。每版只能出现一次，保留五语言，中文和英文需完整记载实质变化，另外三语可以准确概括。以发布当时的功能、名称、验证范围和限制为准，不因今天的方案改名、功能调整或后续修复而重写旧事实。若需订正已发现的历史错误，应明确附加订正说明及依据，不能静默改成未经验证的结论。

4.2.6 首次内置的已发布历史为 4.2.0–4.2.5：4.2.3/4.2.4/4.2.5 的中英文取自两份 CHANGELOG；4.2.0/4.2.1/4.2.2 取自本地正式发布说明，并已核对对应 GitHub 正式 Release。更早版本未补齐不表示它们未发布；后续只能根据真实发行说明补充，不从开发计划推测完成情况。每次构建前校验 JSON、版本去重、倒序和五语言；软件内历史与在线发布页入口同时保留。

软件Release中的 `update.json`仍用schema 1，可选加入 `notes` 语言映射。下面仅是新增字段示例，应合并到已由最终签名EXE生成的清单中；不要改变文件名、大小或摘要：

```json
{
  "notes": {
    "zh-CN": "新增自定义部署文件夹。\n更新适配方案与预设参数。",
    "en": "Added custom deployment folders.\nUpdated schemes and presets.",
    "ru": "Добавлены свои папки установки.\nОбновлены схемы и настройки.",
    "ja": "カスタム配置先を追加しました。\n方式とプリセットを更新しました。",
    "ko": "사용자 지정 배포 폴더를 추가했습니다.\n방식과 사전 설정을 업데이트했습니다."
  }
}
```

新版按当前语言显示说明，缺少对应语言时回退英文、再回退中文；不带 `notes` 的旧清单继续兼容。说明是纯文本，不执行HTML或脚本，最多8个语言条目、每项16 KiB。保持两站同版说明一致，并沿原流程最后发布 `update.json`：先核验已签EXE与云端资源，再发布说明和更新入口。更新日志不得把静态或离线检查写成实体GPU或真实游戏验收。
