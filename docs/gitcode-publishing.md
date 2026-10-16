# GitCode 国内源与发布顺序

GitHub 为主仓库，GitCode 为国内下载源，Gitee 保留过渡版与旧更新清单。所有客户端来源失败均可回退 GitHub；旧下载缓存不因迁移而失效。

- GitCode 主仓库：`gcw_qWIjmve7/RTX-FG-Manager`
- GitCode 补丁库：`gcw_qWIjmve7/RTX-FG-Manager-payloads`
- 4.3.1 完整包元数据：`cloud/bundles/catalog.json` 与不可变版本索引。4.3.0 的 `cloud/v3/catalog.json` 和更早的 `cloud/catalog.json` 保留供旧客户端使用。
- 国内元数据走 GitCode 官方匿名 raw API；无需把访问令牌放进客户端。

GitHub Actions 使用 `mirror-gitcode.yml`。维护者在 GitHub 仓库 Actions Secrets 设置 `GITCODE_TOKEN`，设置有效期，仅开启“项目”和“Repository”读写。GitCode 经典令牌不能限定单个仓库；脚本另外固定目标为上述两个仓库。凭据不写入源码、日志或客户端。工作流只接受主仓库已审核的公开源码，检查新增提交中的文件白名单，快进推送，不强推、不删除远端历史。源码、文档和资源单向从 GitHub 同步。

发布顺序：

1. `build_scheme_bundles.py --baseline <原展开清单> --output <新目录>` 将已审核本地资源合并为每方案一个完整 ZIP，保留原文件字节。0.4.0 包含所有组件；客户端只提取所选入口。`build_v430_resources.py` 仅用于历史 4.3.0 格式维护。
2. GitHub、GitCode、Gitee 均使用固定 `payloads` 标签，标题为 **Resources (not a manager update)**。用 `gitcode_publish.py assets --payloads --tag payloads --folder <资源目录>` 同步 GitCode。不要为每个组件或帮助版本创建新发行。附件名包含内容摘要，同名不同内容不可覆盖；上传后核对分发文件。Gitee 单附件上限 100 MB，Encore 完整包超过限制，仅使用 GitCode/GitHub，并从其包地址中省略 Gitee。
3. 资源齐全后提交 schema 3 清单和索引。源码镜像在提升清单前核对其新国内资源引用，历史 Gitee 包保留原地址。4.3.1 已移除客户端帮助窗口；`cloud/help.json` 和旧帮助 ZIP 仅保留供旧客户端兼容，当前说明在中英文 README。
4. 客户端完成统一验收，交用户签名一次。助手从本机上传同一份签名 EXE 到 GitHub、GitCode 和过渡 Gitee。Actions 永不上传 EXE。
5. 同源 EXE 大小与摘要核对后，上传小附件；`update.json` 最后上传并提升到仓库固定位置。签后不得重建。缺少附件时保持旧清单。

`gitcode_publish.py` 从环境变量读取发布凭据，拒绝带凭据的 URL，上传授权不跨重定向传递。自动化仍需首次配置凭据；工作流文件存在不等同于已成功运行。

已由本机完成匿名回下载的不可变补丁与帮助资源，可以通过维护者控制的 Actions 仓库变量 `RTXFG_RESOURCE_RECEIPTS` 复用回执。回执绑定元数据 SHA256、完整资源 URL、字节数与摘要，有效期七天；仅含公开分发信息。必须从实际成功的下载记录生成，不接受 PR 提供的回执。未命中、过期或摘要变化仍执行下载核验；EXE 和客户端更新清单不能使用此资源缓存。

单包测速只反映该时段和网络，不代表所有国内用户，也不代表 Cloudflare 社区速度。资源发布完整性检查不会加载 DLL，更不是游戏兼容或 GPU 性能测试。
