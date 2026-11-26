# x-upt 工程铁律与三级门禁规范（MANDATORY）

> **本文件定义 `x-upt` 仓库的架构边界、编码红线、接口文档标准与三级发布门禁，与 [AGENTS.md](./AGENTS.md) 配合生效；配套施工指引见 [GATE_BOOTSTRAP.md](./GATE_BOOTSTRAP.md)。所有开发者、主控代理与子代理在新建模块、修改代码、更新契约文档或提交验收前，必须遵守本规范。**

---

## 0. 仓库基准约定

| 规范项 | 工程标准 |
|---|---|
| **组件定位** | 独立、无状态、确定性的多协议模拟上游服务，附带只读在线接口文档与状态门户 |
| **Rust 工作区** | 根 [Cargo.toml](./Cargo.toml)（Cargo Workspace）：`crates/x-upt-core`、`crates/x-upt-crypto`、`crates/x-upt-fronts`、`crates/x-upt`（主二进制 `x-upt`） |
| **前端目录** | `web/`（Vue 3 + TypeScript + Vite + pnpm + Tailwind CSS），生产构建产物 `web/dist` 编译期嵌入 `crates/x-upt` |
| **文档门户监听** | 独立只读监听（默认 `0.0.0.0:18768`，由配置与环境变量驱动），只读 API 前缀 `/api/v1`，健康检查 `/health` |
| **模拟上游监听** | 9 个核心业务面（默认 `16000`–`16008`，含默认启用的 gRPC）+ 6 个信封协议面（默认 `16010`–`16015`），各面监听地址与端口完全由配置驱动 |
| **契约与文档真源** | [docs/API.md](./docs/API.md)、[docs/SPEC.md](./docs/SPEC.md)、`docs/ENVELOPE.md`、`proto/x_upt.proto`、`spec/x_upt.wsdl`，以及运行期同源生成的 `.fds`、GraphQL SDL 与 OpenAPI 3.0 JSON |
| **运行配置与目录** | `--config` / `X_UPT_CONFIG`（缺省加载内置默认配置）与 `--data-dir` / `X_UPT_DATA_DIR`（容器 `/data`，承载自动生成的信封测试密钥与运行日志） |
| **容器与编排** | 多阶段 [Dockerfile](./Dockerfile)（非 root 用户 `xupt` `10001:10001`）与 `deploy/compose.yml` 自包含编排配置 |

---

## 一、架构解耦与模拟上游边界铁律

### 1. 纯粹模拟上游定位（零外部耦合与零有状态膨胀）
1. **中立视角与零外部绑定**：
   - 全仓源码、默认配置、协议契约、测试脚本与文档保持纯粹的模拟上游视角，统一使用中立包名（`x-upt`）与公共演示凭据（如 `demo_app`）。
   - **严禁在代码、配置、前端或文档中掺入任何特定下游网关、压测平台或外部项目的名称、内部适配器概念、私有配置导出格式及个人账号标识。**
2. **无状态确定性运行**：
   - 服务运行完全由静态配置驱动，**严禁引入业务数据库、后台登录状态墙、动态任意路由注入或破坏基线一致性的有状态改写接口**。
   - 相同配置与相同请求入参必须始终返回确定一致的结果：`posts.search`（1–20 冻结帖 + 21–1020 确定性生成）、`photos.render`（哈希取模选图）、`imgdef.compare`（感知哈希汉明距离）及 6 个信封面的 `{ok, reason, hit, fields, digest}` 契约保持 100% 可复现。
3. **配置真源与零端口硬编码**：
   - 所有接入面与文档门户的 `host`、`port`、`enabled`、`tls`、`auth` 均以运行期加载的 `AppConfig` 为唯一真源；默认端口号仅存在于默认配置定义中。
   - **严禁在业务处理逻辑、前端页面、示例生成、契约导出或验收脚本中硬编码固定端口号与可执行文件同级路径。**

### 2. Rust Workspace 单向依赖边界
1. **`crates/x-upt-core`（纯领域与业务核）**：
   - 定义领域模型、配置结构、统一错误类型、协议文档元数据模型以及 `posts`、`photos`、`imgdef` 确定性业务核。
   - **严禁引入 `axum`、`tonic`、`hyper`、`rust-embed` 等网络或静态资源嵌入依赖。**
2. **`crates/x-upt-crypto`（鉴权与信封密码学原语）**：
   - 封装无网络副作用的 HTTP 鉴权算法（Basic、Bearer、OAuth Token 校验、JWT HS/RS、Digest、Hawk、AWS SigV4、HMAC 表单签名）及信封规范化拼串（`canon`）与闭集加解密/签名（`crypt`）。
3. **`crates/x-upt-fronts`（15 个多协议模拟接入面）**：
   - 按协议隔离 9 个核心接入面与 6 个信封接入面；各启用面独立绑定 `TcpListener`、独立维护内存 Token 桶与 Nonce 表，互不共享 Router、互不串票。
4. **`crates/x-upt`（主二进制与只读文档门户）**：
   - 唯一负责 CLI 解析、配置与默认素材初始化、组装拉起 15 个模拟接入面、提供只读文档门户 API 并内嵌 `web/dist` 的入口 Crate。
   - `src/main.rs` 仅负责启动引导、信号监听与退出码返回，严禁堆砌 HTTP Handler。

### 3. 只读文档门户与内嵌前端（Embedded SPA）硬约束
1. **门户与模拟业务端口物理隔离**：
   - 只读文档门户独立监听自有端口（默认 `18768`），**严禁将 `/health`、`/api/v1/*` 或前端静态页面挂载到任何模拟上游业务端口上**，防止污染上游协议契约、鉴权返回码与 `access_log`。
   - 门户仅提供只读接口（`GET` / `HEAD`）：展示运行期真实生效的 15 个接入面状态与端口、完整在线调用文档、同源协议契约下载，以及已脱敏的内存最近访问日志环形缓冲快照。
2. **构建新鲜度锁死与脱离源码自包含运行**：
   - `crates/x-upt/build.rs` 通过 `rust-embed` 内嵌 `web/dist`，声明 `cargo:rerun-if-changed=../../web/dist` 与 `cargo:rerun-if-changed=../../web/dist/index.html`；`--release` 构建、容器构建与 CI 模式下缺失 `web/dist/index.html` 必须直接报错阻断（仅纯后端快速单测允许 `X_UPT_SKIP_FRONTEND_EMBED=1`）。
   - 默认 100 张基准样本图片与演示公钥内置于后端二进制或启动时按需释放，缺失信封测试密钥时在 `--data-dir` 下自动初始化，保证单一二进制放入任意空目录可直接拉起全部 15 个接入面。
3. **API 路由与 SPA Fallback 严格防穿透**：
   - 门户端口上所有 `/health` 与 `/api/*` 未命中路径**必须返回结构化 JSON `404 Not Found`（`Content-Type: application/json`）**，严禁回退返回 `index.html`。
   - 缺失的 `/assets/*` 或带文件扩展名的路径必须返回真实 `404`；仅非保留前缀、无扩展名的 `GET`/`HEAD` 前端路由允许回退至 `index.html`。
4. **精确 MIME、防嗅探与双轨缓存头**：
   - 内嵌静态资源精确设置 `Content-Type`（`.html`、带 `charset=utf-8` 的 `.js`/`.mjs`/`.css`、`.wasm -> application/wasm`、`.svg -> image/svg+xml`、`.woff2`）并统一附带 `X-Content-Type-Options: nosniff`。
   - 指纹静态资源 `/assets/*` 返回 `Cache-Control: public, max-age=31536000, immutable`；`index.html` 返回 `Cache-Control: no-cache` 并配对内容哈希 `ETag`（支持 `If-None-Match` 返回 `304`）。
   - 严禁将 `*.map`、`.env*`、`node_modules` 混入 `web/dist`。
5. **同源契约生成与全端口优雅停机**：
   - 门户与业务面暴露的 `.proto`、`FileDescriptorSet`（`.fds`）、SOAP `WSDL`、GraphQL `SDL`、OpenAPI 3.0 JSON 及页面调用示例，必须与运行期实际服务定义和监听端口同源，严禁手工维护分叉副本。
   - 文档门户、15 个模拟接入面与 `JobStore` 后台协程统一接入 `CancellationToken` 响应 `SIGINT`/`SIGTERM`（容器内由 `tini` 转发），在 10 秒窗口内干净释放监听并以状态码 `0` 退出。

---

## 二、代码与文档质量零容忍红线

### 1. Rust 零绕过红线
- **工具链与锁文件锁死**：[rust-toolchain.toml](./rust-toolchain.toml) 固定 Rust `1.100.0` 并显式包含 `clippy`、`rustfmt`；仓库必须提交 [Cargo.lock](./Cargo.lock)，CI 与门禁全程加 `--locked`。
- **Lints 统一管控**：
  - 根 [Cargo.toml](./Cargo.toml) 统一定义 `[workspace.lints.{rust,rustdoc,clippy}]`，成员 Crate 统一声明 `[lints] workspace = true`，严禁局部覆写放宽规则或提交篡改编译参数的 `.cargo/config.toml`。
  - 所有库 Crate 的 `src/lib.rs` 顶部必须声明：`#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。
- **零豁免与零 `unsafe`**：
  - 全仓 `.rs` 源码严禁出现 `unsafe`、`#[allow(...)]`、`#![allow(...)]`、`#[expect(...)]`、`#![expect(...)]`、`#[warn(...)]`、`cfg_attr(..., allow/expect/warn)`、`#[doc(hidden)]`、`#[rustfmt::skip]`、`cfg(false)`、`cfg(any())` 与动态拼接代码的 `include!`。
- **严禁崩溃、越界、隐式强转与裸标准输出**：
  - 全仓 `deny`：`unwrap_used`、`expect_used`、`get_unwrap`、`panic`、`todo`、`unimplemented`、`unreachable`、`indexing_slicing`（切片统一用 `.get()` / `.first()` / 迭代器）。
  - 全仓 `deny`：`as_conversions`、`as_underscore`、`trivial_casts`、`lossy_float_literal`（统一用 `TryFrom` / `From`）；`let_underscore_must_use`、`unused_result_ok`、`mem_forget`；`dbg_macro`、`print_stdout`、`print_stderr`、`exit`（统一使用结构化 `tracing` 与 `ExitCode`）。
  - 全仓 `deny`：`rc_buffer`、`rc_mutex`、`infinite_loop`、`significant_drop_in_scrutinee`、`non_send_fields_in_send_ty`。
- **复杂度阈值与全量 Rustdoc**：
  - [clippy.toml](./clippy.toml) 阈值上限：`too-many-lines-threshold = 100`、`cognitive-complexity-threshold = 25`、`too-many-arguments-threshold = 7`、`type-complexity-threshold = 250`、`max-fn-params-bools = 3`、`max-struct-bools = 3`、`enum-variant-size-threshold = 200`。
  - `missing_docs`、`missing_errors_doc`、`missing_panics_doc`、`rustdoc::all` 全部 `deny`；所有返回 `Result` 的 `pub` 函数必须包含 `/// # Errors` 段落。

### 2. TypeScript / Web 前端红线
- **严格类型零逃逸**：开启 `"strict": true`、`"noUnusedLocals": true`、`"noUnusedParameters": true`、`"noUncheckedIndexedAccess": true`；严禁 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`@ts-nocheck`、`eslint-disable`，外部 JSON 响应按 `unknown` 接收并经类型守卫校验。
- **离线自包含、资源释放与精简微文案**：
  - 零外部 CDN 依赖（字体、图标、样式全部本地打包）；定时器或流连接在 `onUnmounted` 销毁清理；关键导航、搜索框与下载控件具备稳定的 `data-testid`。
  - 界面控件与文档文案遵循极简事实准则：按钮与导航仅用简短领域名词或动作词（如 `概览`、`接口文档`、`协议契约`、`复制`、`下载`），不写交互提示语、不写引导横幅、不留历史迭代痕迹。

### 3. 接口文档质量红线
- `docs/` 文档与门户在线文档必须完整、准确地写清全部 15 个模拟接入面的协议入口、鉴权方式、签名待签串拼法、加解密算法参数、请求/响应字段约束、状态码/错误码及可直接执行的调用示例。
- 所有示例凭据仅使用公共演示值，信封密钥只引用数据目录相对路径，日志与接口返回严格脱敏密钥与图片 Base64。

---

## 三、三级强制门禁矩阵（G1 → G2 → G3）

提交审查、PR 合入或宣告任务完成前，必须按顺序跑通以下三级门禁；任一步骤退出码非 `0` 或产生告警即视为阻断（BLOCK）。

### G1 · 毫秒级静态、契约与去耦守卫
1. **Shell 与工作流静态检查（与 [ci.yml](./.github/workflows/ci.yml) 一致）**：
   - `git ls-files '*.sh' | xargs shellcheck`
   - `actionlint -color`
   - `./.github/scripts/release-test.sh`
2. `python3 scripts/lint-guard.py`：核验 [rust-toolchain.toml](./rust-toolchain.toml)、[Cargo.lock](./Cargo.lock)、`[workspace.lints]`、[clippy.toml](./clippy.toml)、全量 `.rs` 零豁免与零 `unsafe`，并拦截外部项目耦合词汇与硬编码端口常量。
3. `python3 scripts/web-guard.py`：核验 `web/tsconfig.json` Strict 模式、零 `any`/注释豁免、零外部 CDN 引用及 `onUnmounted` 清理。
4. `python3 scripts/check_routes.py`：双向比对门户 `axum::Router` 与 `portal-openapi.json`，并校验 15 个模拟上游接入面未混入任何门户路径。
5. `python3 scripts/docs-and-plans-guard.py`：核验全仓 Markdown 相对链接 100% 可达、规则文件单文件 `< 24,000` 字节、`.gitignore` 完整屏蔽构建产物、运行日志与动态密钥目录。

### G2 · 分栈构建、内嵌产物审计与单元验证
> **构建顺序铁律**：Rust 编译需内嵌 `web/dist`，必须**先完成前端生产构建、再执行内嵌产物审计、最后执行 Rust 全工作区验证**。

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

### G3 · 单二进制脱离源码黑盒交付与全协议终验（`scripts/gate/all.sh`）
1. 执行 `cargo build --release --locked` 生成单二进制 `target/release/x-upt`（并验证 `docker build` 多阶段镜像构建）。
2. 创建与源码仓物理隔离的临时空目录 `TMP_RUN_DIR="$(mktemp -d)"`，仅放入 `x-upt` 二进制并传入独立 `--data-dir`（不拷贝 `config/`、`imgs/`、`proto/`、`spec/`、`web/`），拉起服务。
3. 自动执行黑盒硬断言：
   - **15 个模拟接入面全协议回归**：通过配置/环境变量注入实际监听端口，运行 [test/acceptance.py](./test/acceptance.py)（9 个核心面 + gRPC 调用）与 [test/envelope_acceptance.py](./test/envelope_acceptance.py)（6 个信封面加解密与验签），100% 通过。
   - **门户健康与状态同源断言**：`GET /health` 返回 `200` JSON；`GET /api/v1/status` 返回的各面启用状态与监听端口同实际运行配置完全一致。
   - **同源契约下载断言**：请求门户 `.proto`、`.fds`、`WSDL`、`SDL`、`openapi.json` 下载接口，断言 `200`、精确 `Content-Type` 且契约内容可被各自解析器正确加载。
   - **内嵌前端路由与防穿透断言**：`GET /` 与深层前端路由返回 `200` `text/html`（`Cache-Control: no-cache` + `ETag`）；`/assets/*` 指纹文件返回 `200`、`immutable`、`nosniff` 且字节哈希与 `web/dist` 一致；`GET /api/v1/__nonexistent_probe__` 返回 `404` `application/json`；`GET /assets/__nonexistent__.js` 返回 `404`。
   - **浏览器冒烟 E2E**：Playwright 直连独立单二进制门户跑通 `web/e2e/smoke.spec.ts`（首屏概览、15 面接口文档检索、契约预览与下载闭环，`console.error` 与 `pageerror` 为 `0`）。
   - **优雅停机断言**：向进程发送 `SIGTERM`，断言进程在 10 秒内以退出码 `0` 干净退出。
4. 门禁脚本末行必须输出固定口令：`ALL GATES PASS`（单步输出 `GATE stepNN PASS`）。

---

## 四、契约先行与独立签收纪律

1. **契约与文档先行**：涉及协议行为、配置项或门户 API 的改动，先更新领域模型、协议契约文件与 `docs/` 文档定义，再并行落地代码与测试。
2. **编写与审查分离**：编码完成后，必须由独立验证上下文重新跑通 `bash scripts/gate/all.sh` 并检查 `git status` 无多余缓存或密钥残留，凭真实全绿输出签发验收结论。
