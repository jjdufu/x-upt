# `x-upt` 有状态模拟上游服务、内嵌 WebUI 与容器交付实施计划

本计划围绕 `x-upt` 作为「独立、确定性、由 `SQLite` 持久化驱动的多协议模拟上游服务」制定：初始零预置业务数据与图片、所有模拟接口默认关闭，由内嵌 WebUI 统一完成素材导入、凭据配置、前置条件门控开关与热端口启停，并配套清晰的对外接口文档与同源协议契约导出，严格遵循 [AGENTS.md](../AGENTS.md)、[IRON_RULES.md](../IRON_RULES.md) 与 [GATE_BOOTSTRAP.md](../GATE_BOOTSTRAP.md)。

---

## 阶段总览

| 阶段 | 核心目标 | 关键产出 |
|---|---|---|
| Phase 0 | Workspace 五层拆分、清理预置素材与静态门禁 | 5 个成员 Crate、中立 `x-upt` 命名、移除预置图片/密钥、Python 静态守卫 |
| Phase 1 | SQLite 存储层、开关前置门控与热端口监管器 | `x-upt.db` 幂等迁移、前置条件 `409` 门控状态机、`FrontSupervisor` 动态启停 |
| Phase 2 | 控制台管理 API、同源契约引擎与内嵌静态路由 | `/api/v1/*` 素材导入与开关 API、可选口令守卫、5 类同源契约生成、`rust-embed` |
| Phase 3 | 对外模拟上游接口文档体系建设 | 中立完整的 [docs/API.md](../docs/API.md)、[docs/SPEC.md](../docs/SPEC.md)、`docs/ENVELOPE.md` |
| Phase 4 | Vue 3 内嵌 WebUI 控制台与在线文档中心 | 接入面与子接口门控开关页、图片/帖子/凭据导入页、在线接口文档与契约下载页 |
| Phase 5 | 多阶段镜像、Compose 编排与 G1–G3 全链路终验 | 三阶段 [Dockerfile](../Dockerfile)、`deploy/compose.yml`、Playwright E2E、`scripts/gate/all.sh` |

---

## Phase 0：Workspace 五层拆分、清理预置素材与静态门禁

**目标**：建立单向依赖的 Cargo Workspace 与中立命名体系，彻底移除仓库预置图片与硬编码账号密钥，清零存量静态违例。

1. **Cargo Workspace 分层设计**：
   - 根 [Cargo.toml](../Cargo.toml) 拆分为 5 个成员 Crate：
     - `crates/x-upt-core`：领域模型、开关前置条件纯校验规则、协议文档元数据模型、统一错误类型 `AppError`，以及面向内存切片/特征的 `posts`、`photos`（按入参哈希在已导入场景图片中确定性选图）、`imgdef`（64 位 aHash 汉明距离计算）业务核，零网络、数据库与嵌入依赖。
     - `crates/x-upt-crypto`：HTTP 鉴权原语（Basic、Bearer、OAuth、JWT HS/RS、Digest、Hawk、AWS SigV4、HMAC 表单签名）与信封规范化拼串（`canon`）、闭集加解密/签名（`crypt`）及密钥合法性校验。
     - `crates/x-upt-store`：基于 bundled `rusqlite` 的 `SQLite` 存储层（WAL 模式，`x-upt.db`），负责版本化幂等迁移与事务级门控持久化。
     - `crates/x-upt-fronts`：15 个模拟接入面的动态监听器与路由适配，按面独立隔离内存 Token 桶、Nonce 表与 `JobStore`。
     - `crates/x-upt`：主二进制 `x-upt`、CLI 入口、`/api/v1/*` 控制台服务与前端静态资源嵌入。
2. **全仓中立化与移除预置文件**：
   - 统一二进制名、包名、协议契约（`proto/x_upt.proto`、`spec/x_upt.wsdl`）与方法名前缀为 `x-upt`，清除所有外部下游项目术语与个人标识。
   - 移除仓内预生成的 `imgs/` 样本图目录、`src/bin/gen_imgs.rs` 以及预置公钥文件；改造 `photos.render` 选图逻辑，脱离 `img_001.png`–`img_100.png` 固定文件名假设，改为从已导入对应场景的图片集合中按哈希稳定选取。
   - 同步适配 [scripts/run.sh](../scripts/run.sh)、`.github/scripts/release.sh` 与 [release-test.sh](../.github/scripts/release-test.sh)。
3. **收紧 Lints 与清零存量违例**：
   - 按 [IRON_RULES.md](../IRON_RULES.md) 配置 `[workspace.lints]` 与 [clippy.toml](../clippy.toml)，所有库 Crate 顶部声明 `#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。
   - 清理全部业务 `.unwrap()` / `.expect()`、`println!` / `eprintln!`（改为 `tracing`）、`std::process::exit`（改为 `ExitCode`）、`as` 隐式强转（改为 `TryFrom` / `From`），拆分超长函数并补齐 `pub` API Rustdoc 与 `# Errors` 文档。
4. **落地首批 G1 静态守卫**：
   - 编写零外部依赖的 `scripts/lint-guard.py` 与 `scripts/docs-and-plans-guard.py`。

---

## Phase 1：SQLite 存储层、开关前置门控与热端口监管器

**目标**：实现“默认全关、零预置数据、不满足前置条件禁止开启开关、开启后热绑定端口”的有状态核心底座。

1. **`SQLite` 表结构与幂等迁移（`crates/x-upt-store`）**：
   - 在 `--data-dir`（默认 `./data`，容器 `/data`）下初始化 `x-upt.db`，包含以下核心表：
     - `schema_migrations`：记录已执行的只追加迁移版本；
     - `global_settings`：全局模拟延迟区间（`latency_min_ms` / `latency_max_ms`）、Token 过期时间、签名时间窗与异步任务过期秒数；
     - `business_methods`：`posts.search`、`photos.render`、`imgdef.compare` 三条记录，初始 `enabled = 0`；
     - `fronts`：15 个接入面配置（`name`、`enabled = 0`、`host`、默认建议端口 `16000`–`16008` 与 `16010`–`16015`、`http2`、`h2c`、`auth_methods_json`、TLS 证书/私钥及 `ws`/`job` 专属参数）；
     - `images`：由 WebUI 导入的图片元数据（`id`、`filename`、`scene` 归属、`mime`、`width`、`height`、`size`、预计算的 `ahash_u64`、落盘于 `<data-dir>/images/` 的安全存储路径）；
     - `posts`：导入或新建的帖子数据集（`id`、`user_id`、`title`、`body`）；
     - `apps`：调用方应用凭据（`client_id`、`client_secret`、`name`、`rsa_public_pem`、允许调用的 `methods_json`）；
     - `envelope_profiles` 与 `envelope_secrets`：6 个信封面的画像配置及对应的签名/加解密密钥材料。
2. **开关前置条件门控状态机（事务级强校验）**：
   - **图片接口门控**：未导入图片时，`photos.render`（要求 `article_cover`、`avatar`、`banner` 三个场景下均至少有 `1` 张已导入图片）与 `imgdef.compare`（要求 `images` 总数 `>= 1`）严禁开启，请求开启一律返回 `409 Conflict` 并附带具体缺失项列表。
   - **帖子接口门控**：`posts` 表记录数为 `0` 时，严禁开启 `posts.search`（返回 `409 Conflict`）。
   - **9 个核心接入面门控**：目标端口冲突、TLS 缺证书、当前没有任何开启的业务子接口、或非免鉴权模式下缺少匹配的 `apps` 凭据（及 JWT RS 公钥）时，严禁开启接入面。
   - **6 个信封接入面门控**：缺少有效画像（换票面须含 `issue` 与 `call`）或画像引用的密钥缺失、长度/格式不合法时，严禁开启接入面。
   - **运行中依赖锁定**：当某业务子接口或接入面处于 `enabled = true` 时，删除其依赖的最后一张场景图片、最后一条帖子或正在引用的凭据/密钥将被 `409 Conflict` 阻止，要求先关闭对应开关再清理资源；关闭最后一个已启用的业务子接口前，须先关闭依赖它的核心接入面（或由事务联动提示）。
3. **`FrontSupervisor` 动态端口热启停与内存快照（`crates/x-upt-fronts`）**：
   - 进程启动时读取 `SQLite`，初始空库下 `0` 个模拟面监听，仅运行控制台端口；若重启前已有开启的接入面，启动时自动校验并恢复绑定。
   - 在 WebUI 开启某接入面时，`FrontSupervisor` 动态执行 `TcpListener::bind`，分配独立子 `CancellationToken` 拉起对应协议服务；关闭接入面时取消该子 Token，排空连接并立即释放端口。
   - 导入图片、帖子或修改凭据后，同步刷新内存只读快照（`ArcSwap`），保证高频模拟请求零跨 `.await` 锁竞争、零数据库读瓶颈。

---

## Phase 2：控制台管理 API、同源契约引擎与内嵌静态路由

**目标**：提供完整的素材导入、凭据管理、开关控制与同源契约下载 API，并封死静态资源路由边界。

1. **CLI 与访问安全模型（`crates/x-upt`）**：
   - 支持 `--admin-listen`（默认 `0.0.0.0:18768`）、`--data-dir`（默认 `./data`）、可选管理口令 `--admin-token`（或 `X_UPT_ADMIN_TOKEN`，默认不设口令免登录直连，配置后校验 `Authorization: Bearer` 或会话 Cookie），并启用 `origin_and_host_guard` 防护跨站修改与非法回环 `Host` 头。
2. **控制台 `/api/v1/*` 核心接口（配套 `admin-openapi.json` 路由强校验）**：
   - **总览与就绪度**：`GET /health`、`GET /api/v1/status`（返回 15 个接入面与 3 个业务子接口的开启状态、实时监听端口、`can_enable` 布尔值及 `missing_preconditions` 原因列表）。
   - **素材与数据导入管理**：
     - 图片库：`GET /api/v1/images`、`POST /api/v1/images`（支持 `multipart/form-data` 单张/批量上传图片并指定或批量分配 `scene`，自动解码校验尺寸并计算 `aHash`）、`GET /api/v1/images/{id}/preview`、`DELETE /api/v1/images/{id}`；
     - 帖子库：`GET /api/v1/posts`、`POST /api/v1/posts`、`POST /api/v1/posts/import`（JSON 批量导入）、`PUT /api/v1/posts/{id}`、`DELETE /api/v1/posts/{id}`；
     - 应用凭据：`GET/POST/PUT/DELETE /api/v1/apps`；
     - 信封画像与密钥：`GET/POST/PUT/DELETE /api/v1/envelopes/{front}/profiles` 与 `/api/v1/envelopes/secrets`（支持上传密钥或一键生成符合算法字节长度的随机密钥，响应仅回显指纹与长度，脱敏密钥原文）。
   - **开关与监听配置控制**：
     - 业务子接口开关：`POST /api/v1/methods/{method}/enable`、`POST /api/v1/methods/{method}/disable`；
     - 接入面配置与热启停：`PUT /api/v1/fronts/{front}`（修改端口、TLS、鉴权列表等）、`POST /api/v1/fronts/{front}/enable`、`POST /api/v1/fronts/{front}/disable`；
     - 全局参数与日志：`GET/PUT /api/v1/settings`、`GET /api/v1/logs/recent`（内存环形缓冲最近脱敏请求流水）与 `GET /api/v1/jobs`（`job` 面队列查看）。
   - **在线文档数据与同源契约导出**：
     - `GET /api/v1/docs/fronts`：返回 15 个接入面的完整接口文档模型与代入当前配置端口的调用示例；
     - `GET /api/v1/contracts/{grpc.proto,grpc.fds,soap.wsdl,graphql.sdl,upstream-openapi.json}`：与运行期服务代码同源生成并下载契约文件。
3. **内嵌 SPA 静态路由守卫**：
   - `crates/x-upt/build.rs` 通过 `rust-embed` 内嵌 `web/dist`，release 下缺失 `web/dist/index.html` 立即报错；`/health` 与 `/api/*` 未命中一律返回 `404` JSON，缺失 `/assets/*` 返回 `404`，`index.html` 返回 `no-cache` + `ETag`，指纹文件返回 `immutable` + `nosniff`。

---

## Phase 3：对外模拟上游接口文档体系建设（`docs/`）

**目标**：站在纯粹的上游服务视角，把零预置开启流程、15 个接入面的协议细节、鉴权与信封加解密规则写清楚。

1. **重写 [docs/API.md](../docs/API.md)（模拟上游调用与前置配置手册）**：
   - 开篇写明服务工作流（启动控制台 → 导入所需素材与凭据 → 开启业务子接口与目标协议接入面 → 发起调用）及 `X-Request-Id` 规则。
   - 完整列出 `posts.search`、`photos.render`、`imgdef.compare` 三个业务接口的前置数据要求、入参校验、确定性返回规则与状态码。
   - 逐一写清 10 种认证方式（`none`、`api_key`、`bearer`、`basic`、`oauth_cc`、`gettoken`、`jwt`、`digest`、`hawk`、`aws_sigv4`）与 `sign_gateway` HMAC-SHA256 待签串拼装规则，以及 9 个核心接入面的报文格式与可运行 `curl` / `grpcurl` / WebSocket 示例。
2. **编写 `docs/ENVELOPE.md`（信封协议与画像规范手册）**：
   - 完整说明 6 类信封接入面的 4 种槽位（`header` / `query` / `form` / `json`）、3 种待签串规则（`sorted_kv` / `concat` / `raw`）、闭集签名与加解密算法密钥长度/参数要求、`order` 与 `fail_in` 语义，以及 `{ok, reason, hit, fields, digest}` 确定性响应契约。
3. **更新 [docs/SPEC.md](../docs/SPEC.md)（架构、存储与运维规范）**：
   - 详细记录 CLI 参数、环境变量、`x-upt.db` 存储目录结构、前置门控规则矩阵、31 列结构化 `access_log` 规范与启停运维流程。

---

## Phase 4：Vue 3 内嵌 WebUI 控制台与在线文档中心（`web/`）

**目标**：构建离线零 CDN、交互清晰的前端工作台，让用户直观看到每个接口缺什么前置条件、导入图片与数据后一键开启并查阅接口文档。

1. **前端工程与组件规范**：
   - 初始化 `web/`（`pnpm` + Vue 3 + TypeScript Strict + Vite + Tailwind CSS + Reka UI），本地打包全部图标与字体，落地 `scripts/web-guard.py`、`scripts/embed-guard.py` 与 `scripts/check_routes.py`。
   - API 模块拆分为 `src/api/{http,types,status,fronts,assets,envelopes,docs,logs}.ts`，通用交互控件统一收口于 `src/components/ui/Ui*.vue`。
2. **五个核心功能视图（遵循精简微文案规范）**：
   - **接入面与接口开关（核心控制台）**：
     - 顶部展示 `posts.search`、`photos.render`、`imgdef.compare` 三个业务子接口卡片与独立开关；未导入图片时，`photos.render` 与 `imgdef.compare` 开关置为禁用态，直接显示 `未就绪` 状态标签与缺失条件（如 `需先导入三类场景图片`），附带跳转至图片导入页的快捷入口；导入满足条件后开关自动解锁可开启。
     - 下方展示 9 个核心接入面与 6 个信封接入面的状态、端口配置与启停开关，实时显示每个面是否具备开启条件及当前监听地址。
   - **数据与素材（导入中心）**：
     - **图片素材**：拖拽或选择多张 PNG/JPEG/GIF 图片上传、设置场景分类（`article_cover` / `avatar` / `banner`）、预览缩略图与 `aHash` 指纹、单张或批量删除；
     - **帖子数据**：支持 JSON 文件/文本批量导入及单条增删改查；
     - **凭据与信封**：管理 `apps` 调用方凭据（含上传 RSA 公钥）以及 6 个信封面的画像与密钥（支持一键生成合规随机密钥）。
   - **接口文档**：按 15 个接入面分类检索的完整在线调用文档，所有请求地址自动填入当前该面配置的实际端口，支持代码片段一键 `复制`。
   - **协议契约**：在线预览并一键 `下载` 同源生成的 `x_upt.proto`、`x_upt.fds`、`x_upt.wsdl`、`schema.graphql` 与 `upstream-openapi.json`。
   - **调用日志**：查看最近脱敏 `access_log` 流水（`request_id`、`front`、`status`、`code`、`auth`、耗时拆解与报文摘要），组件卸载时自动清理定时轮询。

---

## Phase 5：多阶段镜像、Compose 编排与 G1–G3 全链路终验

**目标**：完成容器化持久化编排，并用自动化门禁完整验证“空库全关 → 未导入图片禁止开接口 → 导入后热开启 → 15 面全协议回归 → 重启恢复”闭环。

1. **升级多阶段 [Dockerfile](../Dockerfile) 与 `deploy/compose.yml`**：
   - Stage 1（`node`）：`pnpm install --frozen-lockfile && pnpm build` 生成 `web/dist`；
   - Stage 2（`debian:trixie`）：安装 [rust-toolchain.toml](../rust-toolchain.toml) 指定工具链，执行 `cargo build --release --locked` 生成单二进制 `x-upt`；
   - Stage 3（`debian:trixie-slim`）：创建非 root 用户 `xupt`（`10001:10001`），内置 `tini`，挂载 `/data`（承载 `x-upt.db`、导入图片与日志），暴露控制台端口 `18768` 与默认上游端口段 `16000-16008`、`16010-16015`，`HEALTHCHECK` 探测控制台 `/health`。
   - 提供自包含 `deploy/compose.yml` 与 `deploy/.env.example`，支持通过环境变量自定义端口映射、数据卷路径与可选 `X_UPT_ADMIN_TOKEN`。
2. **改造回归脚本与新增 Playwright E2E**：
   - 更新 [test/acceptance.py](../test/acceptance.py) 与 [test/envelope_acceptance.py](../test/envelope_acceptance.py)：先调用 `/api/v1/*` 验证空库拦截，再自动通过 API 导入测试图片、帖子、应用凭据与信封画像密钥并热开启全部 3 个子接口与 15 个接入面，随后按实际配置端口跑完全部协议与鉴权断言。
   - 编写 `web/e2e/smoke.spec.ts`：在真实浏览器中验证初始状态下 `photos.render` 与 `imgdef.compare` 开关不可开启 → 上传导入样本图片 → 成功开启图片接口与 REST 接入面 → 浏览在线接口文档并下载 `.fds` 契约文件，全程 `console.error` 与 `pageerror` 为 `0`。
3. **对齐 [ci.yml](../.github/workflows/ci.yml) 并跑通 `scripts/gate/all.sh`**：
   - 更新 CI 流水线（`scripts` → `frontend` → `rust`），串联 `scripts/gate/all.sh` 完成 G1 静态守卫、G2 分栈构建审计、G3 空目录单二进制黑盒全链路验收（含重启持久化与 `SIGTERM` 退出码 `0` 校验），确认末行输出 `ALL GATES PASS`。
