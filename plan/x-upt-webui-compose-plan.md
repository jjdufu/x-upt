# `x-upt` 模拟上游服务、在线接口文档门户与容器交付实施计划

本计划围绕 `x-upt` 作为「独立、无状态、确定性的多协议模拟上游服务」的核心定位制定，聚焦跑稳 15 个模拟协议接入面、写清楚对外调用文档与同源机器契约、提供轻量只读的在线文档与状态门户，并通过单二进制与容器编排实现开箱即用交付，严格遵循 [AGENTS.md](../AGENTS.md)、[IRON_RULES.md](../IRON_RULES.md) 与 [GATE_BOOTSTRAP.md](../GATE_BOOTSTRAP.md)。

---

## 阶段总览

| 阶段 | 核心目标 | 关键产出 |
|---|---|---|
| Phase 0 | 中立命名统一、Workspace 拆分与静态门禁基线 | 4 个 Cargo 成员 Crate、全仓中立命名、零告警 Rust 代码库、Python 静态守卫脚本 |
| Phase 1 | 无状态自包含运行底座与同源契约引擎 | `--config`/`--data-dir` CLI、内置基准图片与演示密钥、5 类协议同源契约生成、内存日志环 |
| Phase 2 | 对外模拟上游接口文档体系建设 | 中立完整的 [docs/API.md](../docs/API.md)、[docs/SPEC.md](../docs/SPEC.md)、`docs/ENVELOPE.md` |
| Phase 3 | 只读文档门户后端与内嵌静态资源路由 | 独立只读门户服务、实时状态/契约下载/脱敏日志查看 API、`rust-embed` 与 404 防穿透 |
| Phase 4 | Vue 3 离线在线接口文档与状态门户 | `web/` 前端工程、15 面运行概览、可检索协议文档与动态端口示例、契约在线预览下载 |
| Phase 5 | 多阶段镜像、Compose 编排与 G1–G3 终验 | 三阶段 [Dockerfile](../Dockerfile)、`deploy/compose.yml`、Playwright 冒烟、`scripts/gate/all.sh` |

---

## Phase 0：中立命名统一、Workspace 拆分与静态门禁基线

**目标**：建立独立中立的工程命名与单向依赖边界，消除硬编码耦合与存量静态违例，保持 15 个模拟接入面核心行为不变。

1. **Cargo Workspace 单向分层**：
   - 根 [Cargo.toml](../Cargo.toml) 拆分为 4 个职责清晰的成员 Crate：
     - `crates/x-upt-core`：领域模型、配置结构（`AppConfig`）、统一错误类型（`AppError`）、协议文档元数据模型，以及 `posts`（1–20 冻结 + 21–1020 确定性生成）、`photos`（哈希取模选图）、`imgdef`（aHash 汉明距离）确定性业务核，零网络与嵌入依赖。
     - `crates/x-upt-crypto`：HTTP 鉴权校验原语（Basic、Bearer、OAuth、JWT HS/RS、Digest、Hawk、AWS SigV4、HMAC 表单签名）与信封规范化拼串（`canon`）、闭集加解密及签名原语（`crypt`）。
     - `crates/x-upt-fronts`：9 个核心业务面与 6 个信封协议面的独立监听与路由适配，每面独立维护内存 Token 桶与 Nonce 表。
     - `crates/x-upt`：主二进制 `x-upt`、CLI 引导、默认素材初始化与只读文档门户装配。
2. **全仓中立命名与动态端口参数化**：
   - 统一二进制名、包名、日志默认文件名（`x-upt.log`）、协议契约包名（`proto/x_upt.proto`、`spec/x_upt.wsdl`）与方法前缀，将默认配置中的演示凭据替换为公共中立标识（`demo_app` 与 `demo_rsa_pub.pem`），清除所有外部下游系统术语与个人标识。
   - 默认配置文件将文档门户监听设为 `0.0.0.0:18768`，模拟上游默认端口段设为 `16000`–`16008` 与 `16010`–`16015`（`[fronts.grpc]` 默认 `enabled = true`）。
   - 改造 [test/acceptance.py](../test/acceptance.py) 与 [test/envelope_acceptance.py](../test/envelope_acceptance.py)，改为从配置文件或环境变量动态读取各接入面地址与端口，移除写死端口号与过时的源码关键词扫描逻辑。
   - 同步适配 [scripts/run.sh](../scripts/run.sh)、`.github/scripts/release.sh` 与 [release-test.sh](../.github/scripts/release-test.sh)，兼容 Cargo Workspace 结构与 `x-upt` 包名。
3. **收紧 Lints 并清零存量违例**：
   - 按 [IRON_RULES.md](../IRON_RULES.md) 配置根 `[workspace.lints]` 与 [clippy.toml](../clippy.toml) 阈值，所有库 Crate 顶部声明 `#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。
   - 逐项将业务代码中的 `.unwrap()` / `.expect()` 改为显式错误传播，将 `println!` / `eprintln!` 改为结构化 `tracing`，将 `std::process::exit` 改为 `ExitCode`，将 `as` 数值强转改为 `TryFrom` / `From`，拆分超长函数并补齐全部 `pub` API Rustdoc 与 `# Errors` 说明。
4. **落地首批 G1 静态守卫脚本**：
   - 使用 Python >= 3.11 标准库编写 `scripts/lint-guard.py` 与 `scripts/docs-and-plans-guard.py`。

**阶段验证**：`./.github/scripts/release-test.sh`、`python3 scripts/lint-guard.py`、`python3 scripts/docs-and-plans-guard.py`、`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` 与 `cargo test --workspace --locked` 全部通过。

---

## Phase 1：无状态自包含运行底座与同源契约引擎

**目标**：让单一二进制无需外部源码目录即可独立拉起全部模拟接入面，并实现所有协议契约与运行期配置的同源生成。

1. **CLI 与无状态自包含目录模型**：
   - 支持 `--config`（或 `X_UPT_CONFIG`，未指定时若当前数据目录无配置文件则直接采用内置默认配置）、`--data-dir`（或 `X_UPT_DATA_DIR`，承载运行期自动生成的测试密钥与日志）、`--portal-listen`（或 `X_UPT_PORTAL_LISTEN`，覆盖文档门户监听地址），以及 `serve`（默认）、`check-config`、`print-default-config`、`version` 子命令。
   - 将 100 张确定性测试样本图（`img_001.png`–`img_100.png`）与公共演示 RSA 公钥编入二进制资源或支持运行期内存直读；启动时若检测到信封画像引用的测试密钥目录为空，自动在 `--data-dir` 下生成对应长度的随机测试密钥（保留 [scripts/gen_envelope_keys.sh](../scripts/gen_envelope_keys.sh) 供离线手动生成）。
2. **同源协议契约与接口元数据生成**：
   - **gRPC**：`crates/x-upt-fronts/build.rs` 编译 `proto/x_upt.proto` 生成 `tonic` 服务端代码与 `FileDescriptorSet` 二进制（`x_upt_descriptor.bin`），运行期直接提供 `.proto` 原文与 `.fds` 字节流下载，与 gRPC 接入面严格同源。
   - **SOAP**：`spec/x_upt.wsdl` 作为唯一模板，在 SOAP 接入面 `GET /soap` 与门户契约下载接口中按当前实际监听地址动态渲染 `<soap:address>`。
   - **GraphQL**：GraphQL 接入面 `GET /sdl` 与门户契约下载接口统一调用同一个 `async_graphql::dynamic::Schema::sdl()` 输出。
   - **HTTP 接入面 OpenAPI 与结构化文档模型**：在 `x-upt-core` 中维护 9 个核心接入面与 6 个信封接入面的结构化接口元数据（路径、方法、槽位、鉴权要求、加解密规则、请求/响应 Schema、错误码表），按实际生效的 `AppConfig` 动态生成模拟上游 OpenAPI 3.0 JSON 与示例命令。
3. **脱敏访问日志内存环与全端口优雅停机**：
   - 保持 31 列结构化 `access_log` 文件落盘格式不变，同时在内存中维护固定容量（如最近 500 条）的只读环形缓冲区（复用统一的参数、密钥与图片 Base64 脱敏逻辑），供调用方排错核对。
   - 将文档门户、15 个模拟接入面及 `JobStore` 异步 worker 全部接入同一个 `CancellationToken`，收到 `SIGINT`/`SIGTERM` 后在 10 秒内干净退出。

---

## Phase 2：对外模拟上游接口文档体系建设（`docs/`）

**目标**：站在纯粹的上游服务提供方视角，把 15 个模拟接入面怎么调、怎么验签加解密、返回什么结果写得一清二楚。

1. **重写 [docs/API.md](../docs/API.md)（模拟上游调用参考手册）**：
   - 开篇直接列出 9 个核心接入面与 6 个信封接入面的职责划分、默认端口表、公共演示凭据与 `X-Request-Id` 透传规则。
   - 逐一写清 3 个确定性业务方法（`posts.search`、`photos.render`、`imgdef.compare`）的字段类型、取值范围、确定性命中规律（如探针用例固定命中结果）与边界返回码。
   - 逐一写清 10 种认证方式（`none`、`api_key`、`bearer`、`basic`、`oauth_cc`、`gettoken`、`jwt`、`digest`、`hawk`、`aws_sigv4`）及 `sign_gateway` HMAC-SHA256 表单待签串的完整拼装规则与请求头格式。
   - 按协议给出 `oauth_gateway`、`sign_gateway`、`rest`、`jsonrpc`、`graphql`、`grpc`、`soap`、`ws`、`job` 的标准请求报文、成功响应、错误结构及可直接运行的 `curl` / `grpcurl` 示例。
2. **编写 `docs/ENVELOPE.md`（信封协议与画像规范手册）**：
   - 整合信封六面的完整外部契约说明：4 类参数槽位（`header` / `query` / `form` / `json`）提取与优先级、3 类待签串拼法（`sorted_kv` / `concat` / `raw`）、闭集签名与加解密算法参数要求、`sign_then_encrypt` / `encrypt_then_sign` 执行顺序、`fail_in`（`http` / `envelope`）行为，以及固定结构的 `{ok, reason, hit, fields, digest}` 响应规范与预置画像速查表。
3. **更新 [docs/SPEC.md](../docs/SPEC.md)（架构、配置与运维规范）**：
   - 完整记录 CLI 参数、环境变量、`app.toml` 全字段说明、启动校验规则、31 列 `access_log` 字段定义及单二进制/容器运行方式。

---

## Phase 3：只读文档门户后端与内嵌静态资源路由

**目标**：在独立端口提供零状态污染风险的只读状态、在线文档数据与同源契约下载服务，并严格封装静态资源路由。

1. **只读门户 API（挂载于独立门户端口，默认 `18768`）**：
   - `GET /health`：返回服务存活状态、版本号与已启用接入面数量。
   - `GET /api/v1/status`：返回服务运行摘要（版本、运行时长）以及全部 15 个模拟接入面的实时监听地址、实际端口、启用状态、HTTP2/TLS 模式与生效鉴权/画像摘要。
   - `GET /api/v1/fronts` 与 `GET /api/v1/fronts/{name}`：返回各模拟接入面的结构化接口说明、入参/出参表、鉴权或信封加解密步骤、错误码定义，以及代入当前真实监听端口的可运行调用示例。
   - `GET /api/v1/contracts/{name}`：提供 `grpc.proto`、`grpc.fds`、`soap.wsdl`、`graphql.sdl`、`upstream-openapi.json` 及公共演示验证公钥的同源预览与文件下载。
   - `GET /api/v1/logs/recent`：只读查询内存环形缓冲区中已脱敏的最近入站请求记录（支持按 `front`、`status`、`request_id` 过滤），辅助联调方排查验签或解密报错。
   - `GET /api/v1/portal-openapi.json`：门户自身只读接口的 OpenAPI 描述，配合 `scripts/check_routes.py` 与 Rust 单测做双向路由一致性强校验，确认仅注册 `GET`/`HEAD` 方法且模拟业务端口零门户路径混入。
2. **内嵌 SPA 静态资源与防穿透守卫**：
   - `crates/x-upt/build.rs` 通过 `rust-embed` 内嵌 `web/dist`，配置 `cargo:rerun-if-changed` 并在 release 构建缺失 `web/dist/index.html` 时报错阻断。
   - 门户 `static_files.rs` 严格隔离 `/health` 与 `/api/*`（未命中一律返回 `404` `application/json`）；缺失 `/assets/*` 或带扩展名路径返回真实 `404`；前端路由返回 `index.html`（`Cache-Control: no-cache` + `ETag` / `304`）；指纹静态文件返回精确 MIME、`Cache-Control: public, max-age=31536000, immutable` 与 `X-Content-Type-Options: nosniff`。

---

## Phase 4：Vue 3 离线在线接口文档与状态门户（`web/`）

**目标**：构建干净、零外部 CDN、开箱可读的前端在线文档门户，让调用方打开浏览器就能看清所有模拟接口并拿到契约文件。

1. **前端工程初始化与守卫**：
   - 在 `web/` 初始化 `pnpm` + Vue 3 + TypeScript（开启 `strict`、`noUnusedLocals`、`noUnusedParameters`、`noUncheckedIndexedAccess`）+ Vite + Tailwind CSS，所有字体与图标本地离线打包，落地 `scripts/web-guard.py` 与 `scripts/embed-guard.py`。
   - API 层按 `src/api/{http,types,status,fronts,contracts,logs}.ts` 拆分，所有外部响应经类型守卫窄化。
2. **四个只读核心视图（严守精简微文案规范）**：
   - **概览**：展示 9 个核心接入面与 6 个信封接入面的运行状态卡片（协议标识、实际监听 `host:port`、TLS/HTTP2 状态、鉴权或信封画像概览），数据全部取自 `/api/v1/status`。
   - **接口文档**：按接入面与方法分类检索的完整在线接口手册，直接呈现请求路径、认证头/签名拼串说明、信封加解密参数、字段约束表、响应示例与状态码表，并提供自动填入当前实际端口的 `curl` / `grpcurl` 示例一键 `复制`。
   - **协议契约**：在线代码高亮预览并一键 `下载` 同源生成的 `x_upt.proto`、`x_upt.fds`、`x_upt.wsdl`、`schema.graphql`、`upstream-openapi.json` 与演示验证公钥。
   - **调用日志**：只读表格展示最近入站请求流水（`request_id`、`front`、`method`、`path`、`status`、`code`、`auth`、耗时拆解及脱敏后的请求/响应摘要），支持按 `front`、`status`、`request_id` 筛选与手动 `刷新`，组件卸载时清理定时轮询。

---

## Phase 5：多阶段镜像、Compose 编排与 G1–G3 全链路门禁终验

**目标**：打通多架构容器化交付与脱离源码树的单二进制自动化黑盒终验。

1. **多阶段 [Dockerfile](../Dockerfile) 与自包含 `deploy/compose.yml`**：
   - 升级三阶段 [Dockerfile](../Dockerfile)：Stage 1（`node`）执行 `pnpm install --frozen-lockfile && pnpm build` 产出 `web/dist`；Stage 2（`debian:trixie`）按 [rust-toolchain.toml](../rust-toolchain.toml) 安装指定版本 Rust 工具链并执行 `cargo build --release --locked` 内嵌前端与默认素材；Stage 3（`debian:trixie-slim`）创建非 root 用户 `xupt`（`10001:10001`），内置 `tini` 与 `openssl`，默认数据目录指向 `/data`，`HEALTHCHECK` 探测门户 `/health`。
   - 编写简洁自包含的 `deploy/compose.yml` 与 `deploy/.env.example`：支持通过环境变量配置门户端口（默认 `18768`）与模拟上游端口段（默认 `16000-16008`、`16010-16015`），按需挂载自定义 `app.toml` 与 `/data` 目录。
2. **Playwright 冒烟与 CI 流水线对齐**：
   - 编写 `web/e2e/smoke.spec.ts`：校验门户概览 15 个接入面渲染、接口文档切换与检索、契约预览与下载、调用日志列表查看正常，全程断言 `console.error` 与 `pageerror` 为 `0`。
   - 更新 [ci.yml](../.github/workflows/ci.yml)：`scripts` 任务运行 `shellcheck`、`actionlint`、[release-test.sh](../.github/scripts/release-test.sh) 与 Python G1 守卫；`frontend` 任务构建并上传 `web-dist`；`rust` 任务下载 `web-dist` 后执行 `embed-guard.py`、`cargo fmt --check`、`clippy -D warnings`、`cargo doc -D warnings` 与全套测试。
3. **串联 `scripts/gate/all.sh` 黑盒验收**：
   - 完整串联 G1 → G2 → G3（拷贝 `target/release/x-upt` 单二进制至 `mktemp -d` 空目录拉起，跑通 15 个模拟接入面全协议回归、门户健康与同源契约下载断言、静态资源缓存与 `404` 防穿透断言、Playwright E2E 冒烟及 `SIGTERM` 10 秒内退出码 `0` 断言），确认末行稳定输出 `ALL GATES PASS`。
