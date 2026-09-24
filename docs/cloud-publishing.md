# DLL 云端维护

日常只维护 GitHub 上的 [`cloud/schemes.json`](../cloud/schemes.json)。GitHub Actions 自动生成摘要索引和 `catalog.json`，将 DLL ZIP 与配置同步到 Gitee。软件默认访问国内 Gitee，失败后切换 GitHub；手动选择 GitHub 优先仍有效。

配置和源码在两站的 `pandaligx/RTX-FG-Manager` 主仓库；GitHub DLL 放主仓库的 `payloads` 预发行，Gitee DLL 放独立的 `pandaligx/RTX-FG-Manager-payloads` 资源仓库。这由同一工作流管理，不需要维护第二份配置。实际预检发现 Gitee 的资源预发行仍会抢占同仓库 `/releases/latest`，因此必须隔离资源库，保护旧管理器的软件升级。

此流程管理 DLL 资源，不会自动发布管理器 EXE。EXE 继续先签名，再上传、回下载验证，最后发布 `update.json`。

Actions 的资源探测和自动同步只上传 DLL ZIP。管理器和 aria2 等工具 EXE 均从发布者本机上传 Gitee；不得把 EXE 塞进 ZIP 规避这条发布分工。

## 更新一次 DLL

1. 对新 DLL 完成游戏测试、签名。每个 ZIP 只放一个代理 DLL 和匹配的 `dlssg_sm86.ini`，文件放在 ZIP 根目录。不要打包日志、缓存、原游戏 DLL 或子文件夹。
2. 给 ZIP 新名称，例如 `upstream-0.3.6-version-r1.zip`。上传到 GitHub **`payloads` 固定预发行**。同名不同内容不允许覆盖；修改 INI 或重新签名，也需要新 ZIP 名称。
3. 编辑 `cloud/schemes.json`，修改对应方案的 `version`、`name` 和 `archives`。已有方案的 `id` 保持不变，以保留用户参数记忆。参数协议没有变化时保留 `profile`。
4. 提交后查看 **Publish verified DLL resources** 工作流。它核验 ZIP 内容，上传到 Gitee，两站匿名回下载核对，再发布索引，最后发布正式 catalog。
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
| `profile` | 管理器已支持的 INI 参数协议，不是展示名称 |
| `defaults` | 新配置默认值；留空对象表示使用该协议内置默认值 |
| `archives` | 固定资源 Release 的 ZIP 文件名，不需要手填 URL 或 SHA-256 |
| `min_manager_version` | 可选最低管理器版本；较老的新客户端会跳过该方案并提示升级 |
| `capabilities` | 仅专项构建使用；不要给普通上游 DLL 添加三角洲能力标记 |

当前协议为 `upstream035`、`upstream031`、`native026`、`initial`、`mfg_vulkan_sm86_7`。新 DLL 若改变 INI 字段或功能含义，需要先适配管理器和发布工具，不能只改版本号。

`initial` 方案的两条 GPU 路由使用 `{ "file": "文件.zip", "gpu": "rtx20" }` 和 `rtx30`。兼容既有包身份时允许可选 `id`。其他方案通常直接写文件名即可。所有整数和开关默认值使用字符串，例如 `"1"`。

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

如果首次创建 Gitee 独立资源库的 API 返回 403，使用已登录网页创建一次公开的 `pandaligx/RTX-FG-Manager-payloads` 并初始化 README，再重跑工作流。后续无需重复建库或手动上传 Gitee；已有仓库的所有者或公开状态不符时，脚本会停止，不擅自修改权限。

GitHub 自动令牌来自工作流；Gitee 令牌只放仓库 Secret `GITEE_TOKEN`。不要放入 URL、JSON、日志或源码。日常发布不需要在本机保存令牌。

## 传输边界

管理器的清单、索引、DLL ZIP 和 EXE 正文均由内置 aria2 下载。元数据单连接、有大小限制，文件校验后才使用；已有 ZIP 缓存校验通过可离线复用。下载器先检查 HTTPS 响应头与跳转，再运行 aria2，TLS 证书检查始终开启。响应头预检与后续正文请求是两次请求，不能当作 aria2 内每次跳转都具备可编程校验的保证。

国内优先是站点顺序，不会关闭系统 VPN，也不保证所有运营商速度一致。迁移是否可用，以匿名下载、摘要、Range 与实际无 VPN 网络测试结果为准。
