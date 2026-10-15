# 配置社区服务

Workers + D1 + Turnstile，免费额度起步。客户端离线功能不依赖此服务。

1. 安装 Node.js 24 和 pnpm，运行 `pnpm install --frozen-lockfile`。
2. 将 `wrangler.example.jsonc` 复制为本机 `wrangler.jsonc`，填写自己创建的 D1 ID、Worker 域名与 Turnstile 公钥。实际账户配置不提交。
3. 通过 Wrangler 登录，运行 D1 migrations apply，再通过 `wrangler secret put TURNSTILE_SECRET` 设置服务端密钥。
4. 运行 `pnpm test`，然后 `pnpm exec wrangler deploy`。
5. 在 `/admin` 自行创建审核身份、下载加密备份，把页面显示的公开身份映射加入 `MODERATOR_KEYS` 后重新部署。私钥和备份密码不进入服务端。无需 Zero Trust 或付款信息。

浏览必须指定游戏 ID；可选补丁方案与协议筛选供旧客户端兼容。客户端只在同一游戏内推荐不同方案，不使用跨游戏热度；分享的加载入口限定为已审核的 DLL / ASI 名称，不接受本机路径。写入使用 P-256 签名、时间戳、一次性随机数和已验证身份。服务端限制每身份每游戏五份配置、过滤本机信息、隔离配置修订，并记录审核操作。三个不同已验证身份举报触发隐藏；这不等同于证明三个不同自然人。

非 Steam 游戏可先选择目录中已有游戏，未收录时由玩家确认正式名称。已验证身份首次分享时以 `gameName` 和名称摘要标识原子登记，不以 EXE 名称自动合并游戏；区服、重制版需分别命名。管理员仍可管理目录、审核配置、限制身份和授予官方标识。游戏目录与社区允许为空，不预置伪造玩家推荐方案。

身份备份使用 PBKDF2/AES-GCM；密码由管理员保管。浏览器中的身份会自动锁定，下载问题可再次点击下载按钮，无需重建身份。

`node --test tests/*.test.mjs` 使用内存 SQLite 和测试密钥，不修改线上数据。部署测试与游戏或 GPU 验证是不同范围。

网页帮助与客户端使用同一份静态产物：在仓库根运行 `python tools/build_help.py --output <新产物目录>`，将产物中的 `web` 四个文件复制到 `community/public/help/` 后部署，即可访问 `/help/`。该目录是生成物；原始帮助源码在根目录 `help/`。帮助 ZIP 和独立清单先完成分发验证，再发布 `cloud/help.json`；修改说明不需要重签客户端。
