# `x-upt` 独立全栈模拟上游组件实施计划

本计划结合 `x-loadgen` 目录结构、单二进制内嵌 Web 架构与自包含 Compose 部署方案，以及 `x-upt` 的 15 个多协议接入面代码基线制定，严格对齐 [IRON_RULES.md](../IRON_RULES.md) 与 [GATE_BOOTSTRAP.md](../GATE_BOOTSTRAP.md)。

---

## 阶段总览

| 阶段 | 核心目标 | 关键产出 |
|---|---|---|
| Phase 0 | Workspace 拆分、独立命名与门禁基建 | Cargo Workspace、`x-upt` 主二进制、`16000`–`16015` 端口体系、Python 守卫脚本 |
| Phase 1 | 管理面后端底座与 SPA 内嵌路由 | 独立管理端口 `18768`、`--data-dir` + `x-upt.sqlite`、安全守卫、`rust-embed` |
| Phase 2 | Vue 3 WebUI 工作台脚手架 | `web/` 工程、登录改密、15 面总览面板（`16000`–`16015`）、离线 OpenAPI 文档 |
| Phase 3 | 多子接口模拟数据扩充与规则引擎 | 基线锁定开关、数据集/图片库管理、Front×Path 条件规则、Custom Stubs |
| Phase 4 | gRPC 与多协议契约文件页面生成 | 内置 `.fds`/`.proto` 导出、单文件与多文件 `import` 编译生成 `.fds` + 动态 gRPC Mock |
| Phase 5 | 实时流量检查器、故障注入与实验室 | SSE 实时流水与信封对照、混沌故障注入、`job` 队列控制、信封加解密调试台 |
| Phase 6 | 自包含 Compose 工程与多阶段镜像 | `deploy/` 工程（`18768` + `16000-16015`）、`scripts/*.sh`、三阶段 `Dockerfile` |
| Phase 7 | G1–G3 全链路门禁闭环验收 | Playwright 冒烟、`ci.yml` 更新、`scripts/gate/all.sh` 输出 `ALL GATES PASS` |

---

## Phase 0：Workspace 拆分、独立规范与门禁基建落地

**目标**：完成独立工程规范化、单向依赖分层与静态质量基线，不破坏核心协议行为。

1. **Cargo Workspace 分层与独立命名**：
   - 根 [Cargo.toml](../Cargo.toml) 转为 `[workspace]`，拆出：
     - `x-upt-libs/x-upt-domain`：纯领域模型（`MockRule`、`FaultRule`、`CustomStub`、`AppError`、`DataPaths`、存储与契约端口 `trait`），零 `axum`/`tonic`/`rusqlite`/`rust-embed` 依赖。
     - `x-upt-libs/x-upt-crypto`：认证与信封加解密/签名原语（复用 `src/auth/*`、`src/envelope/{canon,crypt}`）。
     - `x-upt-libs/x-upt-store`：bundled `rusqlite`（SQLite WAL，`x-upt.sqlite`）、幂等建表/迁移 SQL 与检查逻辑。
     - `x-upt-libs/x-upt-core`：确定性业务核（`posts`、`photos`、`imgdef`）与规则匹配/模板渲染/故障注入引擎。
     - `x-upt-libs/x-upt-fronts`：15 个模拟接入面监听适配（`16000`–`16008` 核心 9 面 + `16010`–`16015` 信封 6 面，`[fronts.grpc]` 位于 `16005` 且默认启用），通过 `ArcSwap` 或 `RwLock` 读取热配置。
     - `x-upt-bin/x-upt`：主二进制 `x-upt`、CLI（`serve` / `reset-passwd` / `version` / `health`）、管理面路由（`18768`）与前端嵌入。
   - 同步对齐默认配置 `config/app.toml` 端口段为 `16000`–`16015`（`grpc` 默认 `enabled = true`），更新 [scripts/run.sh](../scripts/run.sh)、[test/acceptance.py](../test/acceptance.py)、[test/envelope_acceptance.py](../test/envelope_acceptance.py) 与 [docs/API.md](../docs/API.md) 中的端口与二进制名称。
2. **锁定 Lints 与复杂度阈值**：
   - 收紧 [clippy.toml](../clippy.toml) 至 [IRON_RULES.md](../IRON_RULES.md) 上限（`hyper::Response` / `tonic::Status` 等大错误类型以 `Box` 包装）。
   - 根 `Cargo.toml` 启用 `[workspace.lints.{rust,rustdoc,clippy}]` 全 `deny` + `unsafe_code = "forbid"` + `allow_attributes = "deny"`；所有成员声明 `[lints] workspace = true`；所有 `lib.rs` 顶部补齐 `#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。
3. **清理存量违例**：
   - 替换业务代码 `.unwrap()` / `.expect()` 为 `?` 或显式错误处理；
   - 替换 `println!` / `eprintln!` 为结构化 `tracing`；
   - 替换 `std::process::exit(1)` 为 `ExitCode` 返回；
   - 替换 `as` 数值强转为 `TryFrom` / `From`；
   - 拆分超长/高复杂度函数并补齐全部 `pub` API Rustdoc 与 `# Errors` 段落。
4. **落地 G1/G2 零依赖 Python 守卫脚本**：
   - 编写 `scripts/lint-guard.py`、`scripts/docs-and-plans-guard.py`、`scripts/check_routes.py`、`scripts/web-guard.py`、`scripts/embed-guard.py`（全部仅使用 Python >= 3.11 标准库）。

**阶段门禁**：`python3 scripts/lint-guard.py` 与 `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`、`cargo test --workspace --locked` 全部通过。

---

## Phase 1：管理面后端底座与单二进制内嵌 Web 架构

**目标**：搭建独立 `18768` 管理面 HTTP 服务、持久化目录模型与前端静态资源安全内嵌机制。

1. **运行目录与 CLI 模型（参照 `x-loadgen`）**：
   - 支持 `--admin-listen`（默认 `0.0.0.0:18768`）、`--data-dir`（或环境变量 `X_UPT_DATA_DIR`，容器内 `/data`）、`--no-auth`（`X_UPT_NO_AUTH`）。
   - 数据目录统一承载：`x-upt.sqlite`、自定义/扩展 `imgs/`、动态生成的 `keys/envelope/`、`admin.passwd`、过程日志 `logs/`。
   - 支持首次启动自动从默认配置或 `config/app.toml` 种子化写入 SQLite。
2. **管理面安全守卫**：
   - 复刻 `x-loadgen` `auth.rs` + `server.rs`：首次启动生成 64 位随机 `admin.passwd`（`600`），未改密前业务管理接口返回 `403 password_change_required`；支持 `Authorization: Bearer` 与 `HttpOnly; SameSite=Lax` 会话 Cookie。
   - 实现 `origin_and_host_guard` 中间件，阻止跨站状态修改请求与回环监听下的非回环 `Host` 头。
   - 日志与导出接口对 `client_secret`、信封密钥、`rsa_public_pem` 与图片 Base64 做统一脱敏。
3. **内嵌 SPA 路由与缓存头硬约束**：
   - 在 `x-upt-bin/x-upt/build.rs` 嵌入 `web/dist`（`rust-embed` + `#[cfg(frontend_embedded)]`），输出 `cargo:rerun-if-changed=web/dist`，release 下缺失 `web/dist/index.html` 直接退出报错（纯后端单测可用 `X_UPT_SKIP_FRONTEND_EMBED=1`）。
   - 在 `static_files.rs` 物理隔离 `/health`、`/api/`、`/metrics`：未命中必须返回 `404` JSON（`Content-Type: application/json`），严禁穿透到 `index.html`；缺失的 `/assets/*` 或带扩展名路径返回 `404`。
   - `/assets/*` 返回 `Cache-Control: public, max-age=31536000, immutable`；`index.html` 返回 `Cache-Control: no-cache` + 内容哈希 `ETag`（支持 `If-None-Match` → `304`）；所有静态响应带精确 `Content-Type` 与 `X-Content-Type-Options: nosniff`。
4. **管理面接口契约**：
   - 编写 `x-upt-bin/x-upt/openapi.json`，在 Rust 单元测试中强校验 Router 全路径与 `openapi.json` 一致。
5. **全端口优雅停机**：
   - 将管理面（`18768`）、`16000`–`16015` 全部已启用接入面、`JobStore` worker 与日志落盘协程统一接入 `CancellationToken`，收到 `SIGINT`/`SIGTERM` 后在 10 秒窗口内干净退出。

---

## Phase 2：Vue 3 WebUI 工作台脚手架

**目标**：复刻 `x-loadgen` 的设计体系与工程规范，建立离线自包含的前端项目 `x-upt-bin/x-upt/web`。

1. **工程初始化**：
   - 使用 Vue 3 + TypeScript（`"strict": true`、`"noUnusedLocals": true`、`"noUnusedParameters": true`、`"noUncheckedIndexedAccess": true`）+ Vite + Tailwind CSS + Reka UI + CodeMirror + ECharts + 本地字体包（零外部 CDN）。
   - 目录拆分：`src/api/{http,types,system,mocks,contracts,inspector,lab}.ts`、`src/components/ui/Ui*.vue`、`src/views/*.vue`、`src/router/index.ts`、`src/App.vue`。
2. **核心基础页面**：
   - **登录与首次改密页**（适配 `--no-auth` 自动跳过）。
   - **全局总览与接入面状态页**：展示 `16000`–`16008` 与 `16010`–`16015` 15 个接入面的启用状态、协议类型、TLS/Auth 策略、实时 QPS/耗时分布 ECharts 曲线（在 `onUnmounted` 销毁实例）。
   - **内置离线接口文档页（`/docs`）**：加载 `/api/v1/openapi.json`，支持检索管理面 API 与复制 `curl` 示例。

---

## Phase 3：众多子接口模拟数据扩充与规则引擎

**目标**：让所有业务面与信封面支持可视化配置多套模拟数据，同时确保默认基线 100% 兼容核心回归探针。

1. **基线锁定（Baseline Lock）机制**：
   - 提供全局开关（默认开启）：保证 `upupt.posts.search`（1–20 冻结帖 + 21–1020 确定性生成）、`upupt.photos.render`、`upupt.imgdef.compare` 及信封 6 面默认响应逻辑稳定；自定义规则按优先级命中，未命中则平滑回落到确定性基线。
2. **内置数据集管理页**：
   - **Posts 数据管理**：分页检索内置 1–1020 条帖子；支持按 `user_id` 新增/编辑/删除/批量导入 JSON 扩展帖子；支持响应字段动态模板（`{{uuid}}`、`{{timestamp_ms}}`、`{{random_int:1:100}}`、`{{req.body.*}}`）。
   - **图片素材库管理**：在线浏览 `imgs/` 缩略图与每张图的 aHash 指纹；支持上传自定义图片、或按宽高/格式/噪声参数在线生成对照图，用于 `imgdef.compare` 汉明距离阈值测试。
3. **子接口级 Mock 规则引擎（Mock Rules）**：
   - 规则数据模型：`id`、`front`（15 个面之一）、`matcher`（HTTP Method + Path / RPC Method / GraphQL Operation / 信封 Profile 名 + Header/Query/JSONPath 条件）、`priority`、`enabled`、`response_mode`（`static` / `template` / `weighted_pool`）。
   - **信封面增强**：支持在 WebUI 为 6 个 `envelope_*` 面的每个 Profile 配置业务返回结构体（模拟第三方支付、征信、短信等真实业务字段），响应生成后自动交由 `envelope/pipeline` 完成加密与签名封包。
   - **热加载**：规则变更写 SQLite 后立即刷新内存 `ArcSwap<RuleTable>`，无需重启任何端口。
4. **自定义虚拟子接口（Custom Stubs）**：
   - 在独立前缀或非冲突路径下，允许通过页面新建任意 REST 端点（自定义 Method、Path、认证策略、模板响应），并支持导入 OpenAPI JSON 一键批量生成 Mock 子接口。

---

## Phase 4：gRPC 与多协议契约文件页面生成及动态驱动

**目标**：在 WebUI 上直接提供单 `.proto` 与多文件 `import` 项目的在线编译、`.fds`/`.pb` 文件生成下载以及动态 gRPC Mock 挂载，同时覆盖其他协议契约与联调配置包导出。

1. **内置契约零依赖导出**：
   - 编译期通过 `include_bytes!(concat!(env!("OUT_DIR"), "/upupt_descriptor.bin"))` 把内置 `upupt.proto` 的 `FileDescriptorSet` 编入二进制。
   - 提供管理面下载 API（`GET /api/v1/contracts/grpc/descriptor`、`GET /api/v1/contracts/grpc/proto`）与 WebUI 按钮，直接下载 `upupt.fds` / `upupt.pb` 文件；默认启用 `[fronts.grpc]`（`:16005`，`h2c`）。
2. **单 `.proto` 在线编辑 + 多文件 `import` 包编译生成 `.fds` + 动态 gRPC Mock**：
   - 在 `x-upt-fronts` 引入纯 Rust proto 编译器（`protox` + `prost-reflect`），实现虚拟文件解析器（`protox::file::FileResolver`），无需容器内外部 `protoc` 二进制。
   - 支持两种输入模式：
     1. **单文件模式**：在 WebUI CodeMirror 编辑器直接粘贴或修改单个 `.proto` 文本，实时编译；
     2. **多文件 `import` 包模式**：上传包含目录结构的 `.zip` / `.tar.gz` 压缩包（或在页面多文件树中管理多个 `.proto` 与公共 `import` 依赖目录如 `google/protobuf/*`、`common/*.proto`），指定入口 `.proto` 与 include 根目录后在内存解析全部 `import` 依赖并编译成完整自包含 `FileDescriptorSet`（含 `--include_imports` 等价信息）。
   - 编译产出双向利用：
     1. 页面即时预览 Service/RPC/Message 结构树并一键下载生成的 `.fds` / `.pb` 文件；
     2. 后端提取 `DescriptorPool` 自动生成每个 RPC 的默认 JSON 请求/响应模板，并通过 `tonic` 通用路由 + `prost_reflect::DynamicMessage` 动态挂载至 `:16005`（支持与 Phase 3 规则引擎联动响应）。
3. **其他强契约协议与联调配置导出**：
   - 一键预览/下载 SOAP WSDL（`upupt.wsdl`）、GraphQL SDL（`schema.graphql`）与模拟面 OpenAPI 3.0 JSON。
   - **一键生成 `x-loadgen` 联调包**：导出标准 Collection + Environment JSON（自动填入 `16000`–`16015` 各面端口、示例 Body、认证凭据）。
   - **一键生成网关渠道接入片段**：按选定接入面生成 `upstream_url`、`auth_flow`、`egress` JSON 预览及对应 RSA 公钥/信封密钥导出包。

---

## Phase 5：实时流量检查器、故障注入与协议实验室

**目标**：将 `x-upt` 建设为全链路联调排错工作台。

1. **实时入站流量检查器（Traffic Inspector）**：
   - 将 `logger` 写入路径抽象为「文件落盘 + 内存环形缓冲 + tokio broadcast channel」，通过管理面 SSE 接口实时推送至 WebUI。
   - 支持按 `request_id`、`front`、`status`、`rpc`、`auth` 过滤；点击单条记录可查看原始请求头/Query/Body、**信封解密前后明密文对照**、验签待签串比对、返回响应体及耗时拆解（`duration_auth` / `duration_core` / `duration_latency`）。
2. **故障注入与混沌测试面板（Fault Injection）**：
   - 支持按接入面、子接口、触发概率（`0`–`100%`）配置动态策略：
     - 延迟尖刺与超时模拟（固定/区间分布延迟 `N` ms）；
     - 状态码与协议错误注入（HTTP `429`/`500`/`502`、gRPC `UNAVAILABLE`/`DEADLINE_EXCEEDED`、JSON-RPC `-32001`、SOAP Fault）；
     - 响应篡改（故意损坏响应 HMAC 签名或密文字节，验证下游验签/解密拦截）。
   - 提供快捷演练动作：**作废指定接入面全部 OAuth Token**、**主动断开指定 WS 活跃连接**，用于测试下游自动换票与 WS 断线重连。
3. **异步任务（`job` 面 `:16008`）队列控制台**：
   - WebUI 实时展示 `JobStore` 任务列表与状态，支持调整模拟执行时长或人工将 `pending`/`running` 任务置为 `succeeded`/`failed`/`expired`。
4. **应用凭据热管理与信封调试台**：
   - 在线增删改 `[[apps]]`（`client_id`、`client_secret`、`methods`、`rsa_public_pem`）及切换各面允许的 `auth` 集合，即时生效。
   - 提供「信封协议调试台」页面，直接调用 `x-upt-crypto` 的 `canon`/`crypt` 逻辑进行明文与密文/签名双向试算。

---

## Phase 6：Compose 自包含工程化与多阶段镜像

**目标**：全面对齐 `x-loadgen/deploy/` 部署体验，实现一条命令拉起带 WebUI 的容器化 `x-upt`。

1. **构建 `deploy/` 自包含 Compose 工程**：
   - `deploy/common/base.yml`、`deploy/lib/common.sh`、`deploy/config/compose.yml`、`deploy/config/.env.example`；
   - `deploy/scripts/{start,stop,down,status,remove,backup,restore,logs,reset-passwd}.sh`；
   - 映射管理面端口（`X_UPT_ADMIN_PORT=18768`）与接入面端口段（`16000-16015:16000-16015`），挂载 `deploy/data`（`/data`）、`deploy/logs`（`/data/logs`）、`deploy/backups`，沿用 `RUN_UID`/`RUN_GID` 属主守卫与 `FIX_OWNER=1` 修复机制。
2. **升级多阶段 [Dockerfile](../Dockerfile)**：
   - Stage 1（`node`）：`pnpm install --frozen-lockfile && pnpm build` 生成 `web/dist`；
   - Stage 2（`debian:trixie`）：安装指定版本 Rust 工具链，`cargo build --release --locked` 内嵌 `web/dist`；
   - Stage 3（`debian:trixie-slim`）：非 root 用户 `10001:10001`，内置 `tini`、`openssl`、默认 `imgs/` 与 `entrypoint`，暴露 `18768` 与 `16000-16015`，健康检查探测管理面 `/health`。

---

## Phase 7：三级门禁闭环与 CI/Release 落地

**目标**：确保前后端代码、契约、内嵌产物与单二进制交付通过自动化硬验证。

1. **完善 Playwright E2E 冒烟用例**：
   - 编写 `x-upt-bin/x-upt/web/e2e/smoke.spec.ts`：直连单二进制管理面（`18768`），验证登录/改密、页面渲染（`console.error` 与 `pageerror` 为 `0`）、新增一条子接口 Mock 规则并命中、在页面通过单 `.proto` 与多文件 `import` 包生成下载 `.fds` 文件及触发 `:16005` 动态 gRPC 调用闭环。
2. **更新 [.github/workflows/ci.yml](../.github/workflows/ci.yml) 与 [.github/workflows/release.yml](../.github/workflows/release.yml)**：
   - `scripts` job：`shellcheck`、`actionlint`、`release-test.sh` + 5 个 Python G1 守卫脚本；
   - `frontend` job：`pnpm` 全套检查 + 构建，上传 `web-dist` artifact；
   - `rust` job：下载 `web-dist`，执行 `cargo fmt --check`、`clippy -D warnings`、`RUSTDOCFLAGS='-D warnings' cargo doc`、`cargo test --workspace --locked`、`cargo test --doc`。
3. **串联 `scripts/gate/all.sh` 并完成独立签收**：
   - 按「G1 静态守卫 → G2 先 Web 后 Rust + `embed-guard.py` → G3 `cargo build --release --locked` → 拷贝单二进制到 `mktemp -d` 空目录拉起 → 黑盒断言（管理面 `/health`、`/`、深层路由、`/assets/*` MIME/哈希/`immutable`/`nosniff`、`/api/v1/*` JSON `404`、`/assets/__nonexistent__.js` `404`、`acceptance.py`、`envelope_acceptance.py`、gRPC `.fds` 导出与调用冒烟、Playwright 冒烟、`SIGTERM` 退出码 `0`）」执行全量验收，确认末行输出 `ALL GATES PASS`。
