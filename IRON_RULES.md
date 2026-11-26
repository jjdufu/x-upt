# x-upt 工程铁律与三级门禁规范（MANDATORY）

> **本文件定义 `x-upt` 仓库的架构边界、有状态开关门控模型、编码红线、接口文档标准与三级发布门禁，与 [AGENTS.md](./AGENTS.md) 配合生效；配套施工指引见 [GATE_BOOTSTRAP.md](./GATE_BOOTSTRAP.md)。所有开发者、主控代理与子代理在新建模块、修改代码、变更表结构、更新契约文档或提交验收前，必须遵守本规范。**

---

## 0. 仓库基准约定

| 规范项 | 工程标准 |
|---|---|
| **组件定位** | 独立、确定性的多协议模拟上游服务，内嵌 WebUI 控制台与在线接口文档，采用 `SQLite` 持久化配置、素材与接口开关状态 |
| **Rust 工作区** | 根 [Cargo.toml](./Cargo.toml)（Cargo Workspace）：`crates/x-upt-core`、`crates/x-upt-crypto`、`crates/x-upt-store`、`crates/x-upt-fronts`、`crates/x-upt`（主二进制 `x-upt`） |
| **前端目录** | `web/`（Vue 3 + TypeScript + Vite + pnpm + Tailwind CSS），生产构建产物 `web/dist` 编译期嵌入 `crates/x-upt` |
| **WebUI 控制台监听** | 独立监听（默认 `0.0.0.0:18768`，由 `--admin-listen` / `X_UPT_ADMIN_LISTEN` 控制），默认免登录直连，支持通过 `--admin-token` / `X_UPT_ADMIN_TOKEN` 开启管理口令；API 前缀 `/api/v1`，健康检查 `/health` |
| **模拟上游监听** | 9 个核心业务面（默认端口分配 `16000`–`16008`）+ 6 个信封协议面（默认端口分配 `16010`–`16015`）；初始全部处于关闭状态（不监听端口），通过 WebUI 开关动态启停 |
| **零预置数据原则** | 仓库与初始数据库不预置任何测试图片、帖子数据、应用凭据或信封密钥；所有业务素材与凭据均由 WebUI 或管理 API 导入后持久化 |
| **契约与文档真源** | [docs/API.md](./docs/API.md)、[docs/SPEC.md](./docs/SPEC.md)、`docs/ENVELOPE.md`、`proto/x_upt.proto`、`spec/x_upt.wsdl`，以及运行期同源生成的 `.fds`、GraphQL SDL 与 OpenAPI 3.0 JSON |
| **运行数据目录** | `--data-dir` / `X_UPT_DATA_DIR`（默认 `./data`，容器内 `/data`），统一存放 `x-upt.db`（SQLite WAL）、导入的图片文件与运行日志 |
| **容器与编排** | 多阶段 [Dockerfile](./Dockerfile)（非 root 用户 `xupt` `10001:10001`）与 `deploy/compose.yml` 自包含编排配置 |

---

## 一、架构边界与有状态开关门控铁律

### 1. 中立上游视角与零外部耦合
1. **中立命名与零下游绑定**：
   - 全仓源码、数据库表设计、协议契约、测试脚本与文档保持纯粹的模拟上游视角，统一使用 `x-upt` 命名体系。
   - **严禁在代码、前端界面、数据库或文档中掺入任何特定下游网关、压测平台或外部项目的名称、私有适配器概念、专属配置导出格式及个人账号标识。**
2. **数据确定性保证**：
   - 基于当前 `SQLite` 中已导入的数据集与素材，相同请求入参始终返回确定一致的结果：`posts.search` 遵循确定性排序与过滤规则返回结果，`photos.render` 按入参哈希取模从对应场景已导入图片中稳定选图，`imgdef.compare` 基于 64 位感知哈希（aHash）计算精确汉明距离，信封 6 面固定返回 `{ok, reason, hit, fields, digest}` 契约结构。
3. **动态端口管理与零硬编码**：
   - 所有接入面的监听 `host`、`port`、`tls`、`auth` 均存储在 `SQLite` 并支持在 WebUI 修改；`16000`–`16015` 仅作为新建初始化表记录时的默认建议端口号。
   - **严禁在业务处理逻辑、图片选图逻辑、前端页面、示例生成或验收脚本中写死固定端口号或写死特定样本文件名。**

### 2. 零预置启动与接口开关前置门控（Precondition Guard）
1. **初始全关与零预置素材**：
   - 仓库内严禁存放预生成样本图目录或硬编码预置账号/密钥；全新 `--data-dir` 首次启动时，仅启动 WebUI 控制台端口（如 `18768`），15 个模拟接入面与 3 个核心业务接口默认全部为 `enabled = false`。
2. **业务子接口开启前置硬校验（不满足条件严禁开启）**：
   - **`photos.render`（图片渲染接口）**：未导入图片（或 `article_cover`、`avatar`、`banner` 任一场景下可用图片数为 `0`）时，**禁止开启**；后端开关接口必须拦截并返回 `409 Conflict` 结构化错误，前端开关置为禁用态并标注缺失条件。
   - **`imgdef.compare`（图片比对接口）**：图片库为空（已导入图片数为 `0`）时，**禁止开启**。
   - **`posts.search`（帖子检索接口）**：帖子表为空（已导入或新建帖子数为 `0`）时，**禁止开启**。
3. **协议接入面开启前置硬校验与热启停**：
   - **9 个核心业务面**：开启前必须满足：① 监听 `host:port` 未与控制台端口或其他已启用面冲突；② 开启 TLS 时已上传有效证书与私钥；③ 至少已开启 `1` 个核心业务子接口；④ 除显式允许 `none` 免鉴权外，至少存在 `1` 个有效应用凭据（`apps`）且已授权至少 `1` 个当前开启的业务子接口（启用 JWT RS 算法时须具备有效 RSA 公钥）。
   - **6 个信封协议面**：开启前必须满足：① 该面下至少配置 `1` 条有效画像（换票类信封面须同时具备 `issue` 与 `call` 画像）；② 画像引用的全部签名/加解密密钥与凭据均已录入并通过算法长度及格式校验。
   - **运行期依赖保护与动态端口绑定**：任一接口或接入面处于开启状态时，禁止删除其依赖的最后一条前置资源（如最后一张图片、最后一条帖子或正在使用的密钥/凭据），须先关闭对应开关方可清空；开启接入面时通过 `FrontSupervisor` 动态 `bind` 端口并派发独立子 `CancellationToken`，关闭时立即优雅释放该端口监听，全程无需重启主进程。

### 3. Rust Workspace 单向依赖边界
1. **`crates/x-upt-core`（纯领域、门控规则与业务核）**：
   - 定义领域模型、开关前置校验纯函数、确定性业务计算（`posts`、`photos`、`imgdef`）、协议文档元数据与统一错误类型 `AppError`。
   - **严禁引入 `axum`、`tonic`、`hyper`、`rusqlite`、`rust-embed` 等网络、数据库或嵌入层依赖。**
2. **`crates/x-upt-crypto`（鉴权与信封密码学原语）**：
   - 封装 HTTP 鉴权算法及信封规范化拼串（`canon`）、闭集加解密/签名（`crypt`）与密钥合法性校验函数。
3. **`crates/x-upt-store`（SQLite 持久化与迁移层）**：
   - 基于 bundled `rusqlite`（开启 WAL 模式与外键约束）管理 `x-upt.db`，实现只追加不篡改的幂等版本迁移，在数据库事务内原子执行资源增删与开关前置条件校验。
4. **`crates/x-upt-fronts`（15 个接入面动态生命周期管理）**：
   - 实现 `FrontSupervisor`，按需热启停 15 个模拟接入面的 `TcpListener`，按面物理隔离内存 Token 桶与 Nonce 表，并基于内存快照（如 `ArcSwap` / `RwLock`）读取启用中的数据集与凭据，读路径严禁跨 `.await` 持锁。
5. **`crates/x-upt`（主二进制、控制台 API 与内嵌前端）**：
   - 唯一负责 CLI 解析、初始化 `x-upt-store` 与 `FrontSupervisor`、挂载 `/api/v1/*` 控制台路由（含可选 `X_UPT_ADMIN_TOKEN` 守卫与 `origin_and_host_guard`）及内嵌 `web/dist` 的入口 Crate。

### 4. 内嵌二进制 Web 资源（Embedded SPA）与同源文档硬约束
1. **门户与模拟业务端口物理隔离**：
   - WebUI 与 `/api/v1/*` 控制台接口仅监听管理端口（默认 `18768`），**严禁挂载到任何模拟上游端口上**，防止污染上游协议契约、鉴权语义与 `access_log`。
2. **构建时序锁死与 404 防穿透**：
   - `crates/x-upt/build.rs` 通过 `rust-embed` 读取 `web/dist`，输出 `cargo:rerun-if-changed=../../web/dist` 与 `cargo:rerun-if-changed=../../web/dist/index.html`；`--release` 构建、容器构建与 CI 中缺失 `web/dist/index.html` 必须报错退出（仅纯后端快速单测允许 `X_UPT_SKIP_FRONTEND_EMBED=1`）。
   - 管理端口上 `/health` 与 `/api/*` 未命中路径**必须返回 `404` JSON（`Content-Type: application/json`）**，严禁穿透返回 `index.html`；缺失的 `/assets/*` 或带扩展名静态路径返回真实 `404`。
3. **MIME、防嗅探与双轨缓存头**：
   - 静态响应精确设置 `Content-Type` 并附带 `X-Content-Type-Options: nosniff`；`/assets/*` 返回 `Cache-Control: public, max-age=31536000, immutable`；`index.html` 返回 `Cache-Control: no-cache` 与内容哈希 `ETag`（支持 `If-None-Match` → `304`）；严禁将 `*.map`、`.env*`、`node_modules` 打入 `web/dist`。
4. **在线接口文档与同源契约导出**：
   - 无论某个接入面当前是否开启，WebUI 与契约下载接口均支持查看 15 个接入面的完整调用文档，并同源导出 `x_upt.proto`、`FileDescriptorSet`（`.fds`）、SOAP `WSDL`、GraphQL `SDL` 与 OpenAPI 3.0 JSON，页面示例自动代入当前数据库中配置的监听端口。
5. **全端口优雅停机**：
   - 管理端口、当前所有已开启的模拟接入面及 `JobStore` 后台协程统一受根 `CancellationToken` 管控，收到 `SIGINT`/`SIGTERM` 后在 10 秒内释放全部端口与数据库连接并以状态码 `0` 退出。

---

## 二、代码与文档质量零容忍红线

### 1. Rust 零绕过红线
- **工具链与锁文件锁死**：[rust-toolchain.toml](./rust-toolchain.toml) 固定 Rust `1.100.0`（含 `clippy`、`rustfmt`）；必须提交 [Cargo.lock](./Cargo.lock)，CI 与门禁全程开启 `--locked`。
- **Lints 统一管控**：
  - 根 [Cargo.toml](./Cargo.toml) 统一定义 `[workspace.lints.{rust,rustdoc,clippy}]`，所有成员声明 `[lints] workspace = true`，所有库 Crate 的 `src/lib.rs` 顶部声明 `#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`，严禁局部放宽规则或提交 `.cargo/config.toml`。
- **零豁免与零 `unsafe`**：
  - 全仓 `.rs` 源码严禁出现 `unsafe`、`#[allow(...)]`、`#![allow(...)]`、`#[expect(...)]`、`#![expect(...)]`、`#[warn(...)]`、`cfg_attr(..., allow/expect/warn)`、`#[doc(hidden)]`、`#[rustfmt::skip]`、`cfg(false)`、`cfg(any())` 与动态拼接的 `include!`。
- **严禁崩溃、越界、隐式强转与裸标准输出**：
  - 全仓 `deny`：`unwrap_used`、`expect_used`、`get_unwrap`、`panic`、`todo`、`unimplemented`、`unreachable`、`indexing_slicing`、`as_conversions`、`as_underscore`、`trivial_casts`、`lossy_float_literal`、`let_underscore_must_use`、`unused_result_ok`、`mem_forget`、`dbg_macro`、`print_stdout`、`print_stderr`、`exit`、`rc_buffer`、`rc_mutex`、`infinite_loop`、`significant_drop_in_scrutinee`、`non_send_fields_in_send_ty`。
- **复杂度阈值与全量 Rustdoc**：
  - [clippy.toml](./clippy.toml) 阈值上限：`too-many-lines-threshold = 100`、`cognitive-complexity-threshold = 25`、`too-many-arguments-threshold = 7`、`type-complexity-threshold = 250`、`max-fn-params-bools = 3`、`max-struct-bools = 3`、`enum-variant-size-threshold = 200`。
  - `missing_docs`、`missing_errors_doc`、`missing_panics_doc`、`rustdoc::all` 全部 `deny`；所有返回 `Result` 的 `pub` 函数必须包含 `/// # Errors` 段落。

### 2. TypeScript / Web 前端红线
- **严格类型零逃逸**：开启 `"strict": true`、`"noUnusedLocals": true`、`"noUnusedParameters": true`、`"noUncheckedIndexedAccess": true`；严禁 `any`、`@ts-ignore`、`@ts-expect-error`、`@ts-nocheck`、`eslint-disable`。
- **组件收口、离线自包含与精简微文案**：
  - 零外部 CDN 依赖；通用表单、开关、弹窗原语收口于 `src/components/ui/Ui*.vue`；轮询或流连接在 `onUnmounted` 清理；所有接口开关、导入按钮、表单与状态徽标提供稳定的 `data-testid`。
  - 按钮、标签页与状态文案遵循极简产品标准（如 `接入面`、`数据与素材`、`接口文档`、`协议契约`、`导入`、`已启用`、`未就绪`），严禁冗余引导横幅或交互动作赘述。

### 3. 接口文档质量红线
- `docs/` 文档与 WebUI 在线文档完整覆盖 15 个模拟接入面的协议路径、认证方式、签名待签串拼装规则、信封加解密参数、出入参约束、前置开启条件、错误码表及可运行调用示例；日志与导出响应对密钥及图片 Base64 严格脱敏。

---

## 三、三级强制门禁矩阵（G1 → G2 → G3）

提交审查、PR 合入或宣告任务完成前，必须按顺序跑通以下三级门禁；任一步骤返回非 `0` 退出码或产生告警即视为阻断（BLOCK）。

### G1 · 毫秒级静态、路由契约与零预置守卫
1. **Shell 与工作流检查（与 [ci.yml](./.github/workflows/ci.yml) 对齐）**：
   - `git ls-files '*.sh' | xargs shellcheck`
   - `actionlint -color`
   - `./.github/scripts/release-test.sh`
2. `python3 scripts/lint-guard.py`：核验工具链、Lints、[clippy.toml](./clippy.toml)、全仓 `.rs` 零豁免与零 `unsafe`、外部耦合词清零，并断言仓库内不存在预置样本图目录或预置明文密钥文件。
3. `python3 scripts/web-guard.py`：核验 `web/tsconfig.json` Strict 模式、零 `any`/注释豁免、零外部 CDN 引用、`Ui*.vue` 收口及 `onUnmounted` 资源释放。
4. `python3 scripts/check_routes.py`：双向比对控制台 `axum::Router` 与 `admin-openapi.json`，并核验 15 个模拟上游接入面未混入管理路由。
5. `python3 scripts/docs-and-plans-guard.py`：核验全仓 Markdown 相对链接 100% 可达、核心规则文件 `< 24,000` 字节、`.gitignore` 屏蔽 `target/`、`node_modules/`、`web/dist/`、`.env*`、`logs/`、`*.pid`、`data/`。

### G2 · 分栈构建、内嵌产物审计与单元验证
> **构建顺序铁律**：必须**先完成前端构建、再执行内嵌产物审计、最后执行 Rust 全工作区验证**。

```bash
# Step 2.1 前端静态检查、单测与生产打包
(cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test && pnpm build)

# Step 2.2 内嵌产物纯净度与体积审计
python3 scripts/embed-guard.py

# Step 2.3 Rust 工作区零警告全套验证
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings && \
  RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked && \
  cargo test --workspace --locked --no-fail-fast && \
  cargo test --workspace --doc --all-features --locked
```

### G3 · 单二进制空目录黑盒交付与状态机全链路终验（`scripts/gate/all.sh`）
1. 执行 `cargo build --release --locked` 生成单二进制 `target/release/x-upt`（并验证 `docker build` 镜像构建）。
2. 创建物理隔离的空目录 `TMP_RUN_DIR="$(mktemp -d)"`，仅放入 `x-upt` 二进制并指定空 `--data-dir "$TMP_RUN_DIR/data"` 拉起服务。
3. 自动执行黑盒硬断言：
   - **初始零预置与全关状态断言**：`GET /health` 返回 `200` JSON；`GET /api/v1/status` 断言 15 个接入面与 3 个业务子接口全部为 `enabled: false`，图片、帖子、凭据数量均为 `0`，且模拟上游端口均未监听。
   - **前置门控拦截断言**：在未导入图片、帖子与凭据前，调用开启 `photos.render`、`imgdef.compare`、`posts.search` 及任一模拟接入面的接口，必须全部返回 `409 Conflict` JSON。
   - **在线导入、热开启与 15 面全协议回归**：通过 `/api/v1/*` 接口导入测试图片、帖子数据、应用凭据与信封画像密钥，依次开启 3 个业务子接口与 15 个模拟接入面（零重启热绑定），运行 [test/acceptance.py](./test/acceptance.py) 与 [test/envelope_acceptance.py](./test/envelope_acceptance.py)，100% 通过；同时验证启用态下删除最后一张依赖图片被 `409` 拦截，关闭接入面开关后对应端口立即停止监听。
   - **SQLite 重启持久化断言**：重启 `x-upt` 进程（复用同一 `--data-dir`），断言已导入素材、配置及处于开启状态的接入面自动恢复并正常响应。
   - **内嵌前端、同源契约与 404 防穿透断言**：验证 `/` 与深层前端路由、`/assets/*` 哈希与 `immutable`/`nosniff` 头、5 类同源协议契约下载、`/api/v1/__nonexistent_probe__` JSON `404` 及 `/assets/__nonexistent__.js` `404`。
   - **真实浏览器冒烟 E2E**：Playwright 跑通 `web/e2e/smoke.spec.ts`（验证未导入图片时图片接口开关禁用 → 页面导入图片 → 开启图片接口开关 → 查看在线接口文档与契约下载，全程 `console.error` 与 `pageerror` 为 `0`）。
   - **优雅停机断言**：发送 `SIGTERM`，进程在 10 秒内以退出码 `0` 干净退出。
4. 门禁脚本末行必须输出固定口令：`ALL GATES PASS`（单步输出 `GATE stepNN PASS`）。

---

## 四、契约先行与独立签收纪律

1. **存储 Schema 与契约先行**：跨前后端或跨协议改动先锁定 `SQLite` 幂等迁移脚本、领域模型、`admin-openapi.json`、上游协议契约文件与 `docs/` 文档，再并行开发后端与前端。
2. **编写与审查分离**：编码完成后，由独立验证上下文完整运行 `bash scripts/gate/all.sh` 并核验 `git status` 干净无残留，凭真实终端全绿输出签收。
