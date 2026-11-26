# x-upt 配套基建与三级门禁落地执行规范（GATE BOOTSTRAP）

> **用途**：本文件是 [IRON_RULES.md](./IRON_RULES.md) 的配套落地施工指引，结合 `x-loadgen` 全栈布局与 `x-upt` 代码基线编写。供开发者或 AI 代理在为 `x-upt` 拆分 Cargo Workspace、接入 Vue 3 内嵌 WebUI 与上线 G1–G3 自动化门禁时按序执行。

---

## 0. 落地前必读与现状基线

执行任何改造前，先读取并核对：
- 行为与仓库基线：[AGENTS.md](./AGENTS.md)、[IRON_RULES.md](./IRON_RULES.md)
- 现有构建与 CI：[Cargo.toml](./Cargo.toml)、[build.rs](./build.rs)、[clippy.toml](./clippy.toml)、[rust-toolchain.toml](./rust-toolchain.toml)、[Dockerfile](./Dockerfile)、[.github/workflows/ci.yml](./.github/workflows/ci.yml)
- 现有模拟上游契约与验收：[docs/SPEC.md](./docs/SPEC.md)、[docs/API.md](./docs/API.md)、[proto/upupt.proto](./proto/upupt.proto)、[spec/upupt.wsdl](./spec/upupt.wsdl)、[test/acceptance.py](./test/acceptance.py)、[test/envelope_acceptance.py](./test/envelope_acceptance.py)
- 参照样板（`x-loadgen`）：Workspace 分层（`x-loadgen-bin/x-loadgen` + `x-loadgen-libs/*`）、`build.rs` + `rust-embed`（`static_files.rs`）、路由守护与安全中间件（`server.rs`）、Vue 3 + pnpm 前端（`web/`）、OpenAPI 路由一致性校验（`openapi.rs`）及自包含 `deploy/` Compose 工程。

**当前 `x-upt` 存量基线（改造起点）**：
- `x-upt` 为独立项目，当前以单体 Crate 形态存在，`src/` 下共 `57` 个 `.rs` 文件（`0` 处 `unsafe`、`0` 处 `#[allow]`、`76` 处 `.unwrap(`、`2` 处 `.expect(`、`11` 处 `println!`/`eprintln!`，另有 `main.rs` 的 `std::process::exit(1)` 与 `posts_gen.rs` 等处的 `as` 强转，尚无 `lib.rs` / Rustdoc 全覆盖）。
- 暂无 `web/` 前端目录、管理面 HTTP 服务、`openapi.json` 及 `scripts/*-guard.py` / `scripts/gate/all.sh` 门禁脚本。

---

## 一、五步落地清单

### 1. 锁死 Rust Workspace、独立命名与 Clippy 配置
1. **Workspace 拆分与独立标识**：
   - 根 `Cargo.toml` 转为 `[workspace]`，按依赖倒置拆分为 `x-upt-libs/x-upt-domain`（纯模型/错误/`trait`，零 HTTP/DB/嵌入依赖）、能力层 Crate（如 `x-upt-core`、`x-upt-crypto`、`x-upt-store`、`x-upt-fronts`）与入口 Crate `x-upt-bin/x-upt`（产出二进制 `x-upt`）。
   - 统一独立命名与默认端口：环境变量前缀 `X_UPT_*`、数据文件 `x-upt.sqlite`、管理面默认监听 `0.0.0.0:18768`、模拟上游接入面默认监听 `16000`–`16008` 与 `16010`–`16015`（其中 `[fronts.grpc]` 位于 `16005` 且默认启用）。
2. **工具链与复杂度阈值**：
   - 保持 [rust-toolchain.toml](./rust-toolchain.toml)（固定 `1.100.0`，显式包含 `clippy`、`rustfmt`）。
   - 收紧 [clippy.toml](./clippy.toml) 至：`too-many-lines-threshold = 100`、`cognitive-complexity-threshold = 25`、`too-many-arguments-threshold = 7`、`type-complexity-threshold = 250`、`max-fn-params-bools = 3`、`max-struct-bools = 3`、`enum-variant-size-threshold = 200`（`hyper::Response` / `tonic::Status` 等大错误体以 `Box` 包装）。
3. **Workspace Lints 全 `deny` / `forbid`**：
   - 在根 `Cargo.toml` 配置 `[workspace.lints.{rust,rustdoc,clippy}]`：`unsafe_code = "forbid"`、`allow_attributes = "deny"`、`missing_docs = "deny"`、`missing_errors_doc = "deny"`、`missing_panics_doc = "deny"`、`rustdoc::all = "deny"`，以及 `unwrap_used`、`expect_used`、`get_unwrap`、`panic`、`todo`、`unimplemented`、`unreachable`、`indexing_slicing`、`as_conversions`、`as_underscore`、`trivial_casts`、`lossy_float_literal`、`let_underscore_must_use`、`unused_result_ok`、`mem_forget`、`dbg_macro`、`print_stdout`、`print_stderr`、`exit`、`rc_buffer`、`rc_mutex`、`infinite_loop`、`significant_drop_in_scrutinee`、`non_send_fields_in_send_ty` 全设 `deny`。
   - 所有成员 Crate `Cargo.toml` 声明 `[lints] workspace = true`，所有 `src/lib.rs` 顶部加：`#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。

### 2. 编写零外部依赖的 G1/G2 静态守卫脚本（Python >= 3.11 标准库）
1. **`scripts/lint-guard.py`**：
   - 核验 `rust-toolchain.toml`（含 `clippy`、`rustfmt`）、`Cargo.lock` 已提交、`[workspace.lints]` 与 `[lints] workspace = true` 完整、`clippy.toml` 阈值严谨、仓库内不存在篡改规则的 `.cargo/config*`。
   - 剥离 Rust 注释与字符串字面量后，全仓拦截 `unsafe`、`#[allow]`/`#![allow]`/`#[expect]`/`#![expect]`/`#[warn]`/`cfg_attr(..., allow|expect|warn)`、`#[doc(hidden)]`、`#[rustfmt::skip]`、`cfg(false)`、`cfg(any())` 与动态 `include!`。
2. **`scripts/web-guard.py`**：
   - 校验 `x-upt-bin/x-upt/web/tsconfig.json` 开启 `"strict": true`、`noUnusedLocals`、`noUnusedParameters`、`noUncheckedIndexedAccess`。
   - 剥离 TS/Vue 注释与字符串后，全仓拦截 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`@ts-nocheck`、`eslint-disable`；检查基础 UI 原语收口（`src/components/ui/Ui*.vue`）与 `onUnmounted` 资源释放（ECharts / EventSource / WebSocket）。
3. **`scripts/embed-guard.py`**：
   - 校验 `x-upt-bin/x-upt/web/dist/index.html` 存在；拦截 `*.map`、`.env*`、`node_modules` 或业务测试图片（`imgs/`）混入 `web/dist`。
   - 校验单文件与总内嵌体积上限；核验 `x-upt-bin/x-upt/build.rs` 声明 `cargo:rerun-if-changed=web/dist` 且在 release 构建缺失 `web/dist/index.html` 时非零退出。
4. **`scripts/check_routes.py`**：
   - 静态提取管理面 `axum` Router 路由，与 `x-upt-bin/x-upt/openapi.json` 双向比对（对照 `x-loadgen` `openapi.rs` 策略），阻断未登记暗路由与文档空头路由；同时校验 `16000`–`16015` 业务接入面未混入管理面路径。
5. **`scripts/docs-and-plans-guard.py`**：
   - 校验仓内 Markdown 相对链接 100% 指向真实存在文件；校验 [AGENTS.md](./AGENTS.md)、[IRON_RULES.md](./IRON_RULES.md) 与本文件各自 `< 24,000` 字节；核验 `.gitignore` 屏蔽 `target/`、`node_modules/`、`web/dist/`、`.env*`、`logs/`、`*.pid`、`config/keys/envelope/`、`data/`、`backups/`。

### 3. 加固管理面 Web 服务端路由层（物理隔离业务接入面）
1. **独立监听端口**：管理面（`0.0.0.0:18768`）独立于 `16000`–`16015` 业务接入面运行，挂载 `/health`、`/api/v1/*` 与内嵌 SPA Fallback。
2. **404 防穿透与缓存头**：
   - 所有 `/health`、`/api/*`、`/metrics` 未命中路由**严格返回 `404` JSON（`Content-Type: application/json`）**，禁止穿透到 `index.html`。
   - 缺失的 `/assets/*` 或带扩展名静态路径返回真实 `404`。
   - `/assets/*` 返回 `Cache-Control: public, max-age=31536000, immutable`；`index.html` 返回 `Cache-Control: no-cache` + 内容哈希 `ETag`（支持 `If-None-Match` → `304`）。
   - 所有静态响应精确设置 `Content-Type`（`.js`/`.mjs`/`.css` 带 `charset=utf-8`，`.wasm` 为 `application/wasm`，`.svg` 为 `image/svg+xml`）并统一附带 `X-Content-Type-Options: nosniff`；开启预压缩/压缩层且跳过已压缩二进制。
3. **安全与停机**：复用 `x-loadgen` 的 `admin.passwd` 首登改密（支持 `--no-auth` 联调）与 `origin_and_host_guard`；管理面、全部业务端口及后台 worker 统一接入 `CancellationToken` 实现 `SIGINT`/`SIGTERM` 优雅退出。

### 4. 落地 G3 单二进制空目录黑盒终验 `scripts/gate/all.sh` 与 CI
1. **`scripts/gate/all.sh`** 串联全链路并在末尾输出 `ALL GATES PASS`（单步输出 `GATE stepNN PASS`）：
   - **G1**：`shellcheck`（`git ls-files '*.sh'`）、`actionlint`（读取 [.github/actionlint.yaml](./.github/actionlint.yaml)）、[release-test.sh](./.github/scripts/release-test.sh)，以及上述 5 个 Python 守卫脚本。
   - **G2**（严格先 Web 后 Rust）：`(cd x-upt-bin/x-upt/web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test && pnpm build)` → `python3 scripts/embed-guard.py` → `cargo fmt --all -- --check` → `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked` → `cargo test --workspace --locked` → `cargo test --workspace --doc --all-features --locked`。
   - **G3（脱离源码黑盒交付）**：`cargo build --release --locked` → 仅拷贝编译产物到 `TMP_RUN_DIR="$(mktemp -d)"`（使用独立 `--data-dir`，不拷贝 `web/` 或 `web/dist/`）→ 启动服务（管理面 `18768` + 业务面 `16000`–`16015`）并断言：
     - 管理面 `/health` 返回 `200` JSON；`/` 与 `/app/deep-route-check` 返回 `200` `text/html`（`no-cache` + 含 `/assets/`）；
     - 提取首页 `/assets/*.js` 与 `*.css` 验证 `200`、MIME、`immutable`、`nosniff` 及 SHA-256 与构建期 `web/dist` 一致；
     - `/api/v1/__nonexistent_probe__` 返回 JSON `404`，`/assets/__nonexistent__.js` 返回 `404`；
     - 运行模拟上游基线回归（`test/acceptance.py`、`test/envelope_acceptance.py`）及 gRPC `.fds`（单文件与多文件 `import` 打包）导出和 RPC 冒烟；
     - Playwright 直连单二进制管理面（`18768`）跑通 `e2e/smoke.spec.ts`（`console.error` 与 `pageerror` 为 `0`，含模拟规则与 gRPC 文件生成闭环）；
     - 发送 `SIGTERM`，进程在 `<= 10s` 内以退出码 `0` 退出。
2. **更新 [.github/workflows/ci.yml](./.github/workflows/ci.yml) 与 [Dockerfile](./Dockerfile)**：
   - CI 增设 `frontend` job（`pnpm` 构建 `web/dist` 并上传 artifact 供 `rust` job 真实嵌入）及 Python G1 守卫步骤；
   - `Dockerfile` 增设 `node` 前端构建阶段（参照 `x-loadgen/Dockerfile`）并与 `deploy/` 自包含 Compose 工程保持路径对齐。

### 5. 存量违例清零与签收
- 按第 1 步收紧 Lints 后，逐项清理存量代码中的业务 `.unwrap()`/`.expect()`、`println!`/`eprintln!`（切至 `tracing`）、`std::process::exit`（切至 `ExitCode`）、`as` 强转（切至 `TryFrom`/`From`）、超长/高复杂度函数及缺失的 Rustdoc 注释。
- 由独立 `Verifier` 上下文运行 `bash scripts/gate/all.sh` 与 `git status`，确认输出 `ALL GATES PASS` 后方可宣告完成。
