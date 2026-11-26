# SQLite 状态化存储、接口开关前置门控与内嵌 WebUI 控制台

## 背景

`x-upt` 定位为独立、确定性的多协议模拟上游服务。本次落地五层 Cargo Workspace、基于 `SQLite` WAL 模式的持久化状态层、空库零预置默认全关机制、事务级接口开关前置条件门控，以及内嵌于单二进制的离线 Vue 3 WebUI 管理控制台与在线协议文档中心。

## 代码

- **`crates/x-upt-core`**：定义 3 个确定性业务接口（`x_upt.posts.search`、`x_upt.photos.render`、`x_upt.imgdef.compare`）、15 个模拟接入面元数据、开关前置校验与运行期依赖锁定规则、31 列结构化访问日志脱敏模型，以及同源在线接口文档与上游 OpenAPI 3.0 生成器。
- **`crates/x-upt-crypto`**：提供 10 种核心 HTTP 鉴权校验器（`none`、`api_key`、`bearer`、`basic`、`oauth_cc`、`gettoken`、`jwt`、`digest`、`hawk`、`aws_sigv4`）、6 个信封接入面的词法 JSON 解析器、3 类规范化待签串拼装器、闭集签名/加解密原语与随机合规密钥生成器。
- **`crates/x-upt-store`**：基于 bundled `rusqlite` 管理 `<data-dir>/x-upt.db` 与 `<data-dir>/images/`；在无对应图片、帖子、应用凭据或信封画像密钥时以 `409 Conflict` 阻断开启开关，并在接口启用期间拦截删除最后一个被依赖资源的请求。
- **`crates/x-upt-fronts`**：通过 `FrontSupervisor` 与子 `CancellationToken` 动态热绑定、热重绑与热释放 9 个核心接入面及 6 个信封接入面监听端口；通过 `SharedRuntime` 原子快照读取业务数据集与编译期 `FileDescriptorSet` 同源契约。
- **`crates/x-upt`**：提供 `serve`、`check`、`version` 子命令、Origin/Host 防护与可选管理口令鉴权、35 个管理端 API 操作（与 `crates/x-upt/admin-openapi.json` 双向校验），以及通过 `rust-embed` 内嵌 `web/dist` 的静态资源服务。
- **`web/`**：构建零外部 CDN 的 Vue 3 + TypeScript Strict + Vite + Tailwind CSS + Reka UI 控制台，涵盖接入面与接口开关、素材与凭据导入、实时端口在线文档、同源契约预览下载与最近访问流水查看，配套 Vitest 单元测试与 Playwright 端到端冒烟测试。
- **静态守卫与门禁脚本**：新增 `scripts/lint-guard.py`、`scripts/web-guard.py`、`scripts/embed-guard.py`、`scripts/docs-and-plans-guard.py`、`scripts/check_routes.py` 与 `scripts/gate/all.sh`，升级参数化回归套件 `test/acceptance.py` 与 `test/envelope_acceptance.py`。

## 数据库

新增 `<data-dir>/x-upt.db`（`SQLite` WAL 模式）及幂等版本迁移 `version = 1`（`initial_stateful_schema`），包含 `schema_migrations`、`global_settings`、`business_methods`、`fronts`、`images`、`posts`、`apps`、`envelope_secrets`、`envelope_profiles` 共 9 张表。首次启动时仅写入全局默认模拟参数与 3 个业务子接口、15 个接入面的关闭状态配置（`enabled = 0`），业务图片、帖子、应用凭据及信封密钥表均为空。

## 配置与契约文档

- 协议契约真源：`proto/x_upt.proto`（`package x_upt.v1`，`service XUpt`）、`spec/x_upt.wsdl`（`urn:x-upt`）及管理端契约 `crates/x-upt/admin-openapi.json`。
- 接口与运维文档：`docs/API.md`、`docs/ENVELOPE.md`、`docs/SPEC.md`。
- 构建与容器编排：三阶段 `Dockerfile`（Node 构建前端 → Rust `--release --locked` 内嵌编译 → `debian:trixie-slim` 非 root `10001:10001` 运行）、`deploy/compose.yml`、`deploy/.env.example`、`scripts/run.sh`、`scripts/docker-entrypoint.sh` 及 `.github/workflows/ci.yml`。

## 验证

执行 `bash scripts/gate/all.sh` 跑通三级门禁：
1. **G1 静态守卫**：`lint-guard.py`、`web-guard.py`、`docs-and-plans-guard.py`、`check_routes.py`（35 个管理接口操作全对齐）与 `.github/scripts/release-test.sh`（26 个用例全绿）。
2. **G2 分栈审计与编译**：`pnpm`（ESLint `--max-warnings 0`、`vue-tsc --noEmit`、Vitest 6 项单测、`vite build`）、`embed-guard.py`、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` 与 `cargo build --release --locked`。
3. **G3 隔离目录单二进制端到端验收**：空目录单二进制启动、SPA 路由缓存与 `/api/*` 404 隔离校验、Playwright 真实浏览器前置门控与上传解锁验收、15 个模拟接入面与 10 种认证方式全链路断言、热停用端口即时释放、`SIGTERM` 10 秒内退出码 `0` 排空及 `SQLite` 重启状态恢复校验，末行输出 `ALL GATES PASS`，并经独立复核代理二次运行确认。

## 部署说明

- 单机直接运行：执行 `./x-upt serve --admin-listen 0.0.0.0:18768 --data-dir ./data`（或 `scripts/run.sh start`），浏览器访问管理端导入所需基准图片、帖子与凭据后按需开启业务子接口与协议接入面。
- 容器运行：在 `deploy/` 目录复制 `.env.example` 为 `.env`，执行 `docker compose up -d --build`，持久化卷挂载至容器 `/data`。

## 文件清单

- `Cargo.toml`、`Cargo.lock`、`clippy.toml`、`.gitignore`、`.dockerignore`
- `crates/x-upt-core/Cargo.toml`、`crates/x-upt-core/src/{lib.rs,domain.rs,error.rs,imgs.rs,services.rs,preconditions.rs,logger_types.rs,docs_model.rs}`
- `crates/x-upt-crypto/Cargo.toml`、`crates/x-upt-crypto/src/{lib.rs,nonce.rs,jwt_digest.rs,hawk_aws.rs,http_auth.rs,table.rs,profile.rs,slots.rs,canon.rs,crypt.rs,crypt_asym.rs,pipeline.rs}`
- `crates/x-upt-store/Cargo.toml`、`crates/x-upt-store/src/{lib.rs,schema.rs,dto.rs,queries.rs,ops_assets.rs,ops_envelopes.rs,ops_switches.rs}`
- `crates/x-upt-fronts/Cargo.toml`、`crates/x-upt-fronts/build.rs`、`crates/x-upt-fronts/src/{lib.rs,snapshot.rs,logger.rs,token.rs,http_guard.rs,jobstore.rs,fronts_gateway.rs,fronts_rest_rpc.rs,fronts_graphql.rs,fronts_grpc.rs,fronts_soap.rs,fronts_ws_job_env.rs,contracts.rs,supervisor.rs}`
- `crates/x-upt/Cargo.toml`、`crates/x-upt/build.rs`、`crates/x-upt/admin-openapi.json`、`crates/x-upt/src/{main.rs,cli.rs,state.rs,security.rs,static_files.rs,api_control.rs,api_assets.rs,router.rs}`
- `web/`（Vue 3 + TypeScript Strict 控制台源码、Vitest 单测与 Playwright `e2e/smoke.spec.ts`）
- `proto/x_upt.proto`、`spec/x_upt.wsdl`
- `docs/API.md`、`docs/ENVELOPE.md`、`docs/SPEC.md`
- `Dockerfile`、`deploy/compose.yml`、`deploy/.env.example`、`scripts/run.sh`、`scripts/docker-entrypoint.sh`
- `.github/workflows/ci.yml`、`.github/scripts/release.sh`、`.github/scripts/release-test.sh`
- `scripts/{lint-guard.py,web-guard.py,embed-guard.py,docs-and-plans-guard.py,check_routes.py}`、`scripts/gate/all.sh`
- `test/acceptance.py`、`test/envelope_acceptance.py`
- `AGENTS.md`、`IRON_RULES.md`、`GATE_BOOTSTRAP.md`、`plan/x-upt-webui-compose-plan.md`
