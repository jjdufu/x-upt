# x-upt 基建落地与三级门禁执行指引（GATE BOOTSTRAP）

> **用途**：本文件是 [IRON_RULES.md](./IRON_RULES.md) 的配套落地施工说明，供开发者与代理在拆分 `x-upt` Cargo Workspace、完善对外接口文档、嵌入只读在线文档门户及建设 G1–G3 自动化门禁时按序执行。

---

## 0. 落地前核对清单

动手修改前，先核对仓内现有基线文件：
- 工程规范：[AGENTS.md](./AGENTS.md)、[IRON_RULES.md](./IRON_RULES.md)
- 构建与发版工作流：[Cargo.toml](./Cargo.toml)、[build.rs](./build.rs)、[clippy.toml](./clippy.toml)、[rust-toolchain.toml](./rust-toolchain.toml)、[Dockerfile](./Dockerfile)、[ci.yml](./.github/workflows/ci.yml)、[release.yml](./.github/workflows/release.yml)、[release-test.sh](./.github/scripts/release-test.sh)
- 模拟上游契约与验收脚本：[docs/SPEC.md](./docs/SPEC.md)、[docs/API.md](./docs/API.md)、[scripts/run.sh](./scripts/run.sh)、[scripts/gen_envelope_keys.sh](./scripts/gen_envelope_keys.sh)、[test/acceptance.py](./test/acceptance.py)、[test/envelope_acceptance.py](./test/envelope_acceptance.py)

---

## 一、五步落地流程

### 1. 统一中立命名、拆分 Cargo Workspace 与锁死 Lints
1. **Workspace 分层与独立标识**：
   - 根 [Cargo.toml](./Cargo.toml) 配置为 `[workspace]`，按单向依赖拆分为：
     - `crates/x-upt-core`：领域模型、配置结构、统一错误类型、协议文档元数据与 `posts` / `photos` / `imgdef` 确定性业务核（零网络与嵌入依赖）；
     - `crates/x-upt-crypto`：HTTP 鉴权校验原语与信封 `canon` / `crypt` 算法实现；
     - `crates/x-upt-fronts`：15 个多协议模拟接入面监听与路由适配；
     - `crates/x-upt`：主二进制 `x-upt`、CLI 入口、内置默认素材释放与只读文档门户服务。
   - 统一包名、二进制名、默认日志名、协议契约文件与演示凭据为中立的 `x-upt` / `demo_app` 命名，同步适配 [scripts/run.sh](./scripts/run.sh)、`.github/scripts/release.sh` 与 [release-test.sh](./.github/scripts/release-test.sh)。
   - 默认配置将文档门户监听设为 `0.0.0.0:18768`，模拟上游监听设为 `16000`–`16008` 与 `16010`–`16015`（gRPC 默认启用），所有代码、文档示例与验收脚本一律从配置或环境变量读取地址与端口。
2. **工具链、复杂度阈值与 Workspace Lints**：
   - 保持 [rust-toolchain.toml](./rust-toolchain.toml) 固定 `1.100.0`（含 `clippy`、`rustfmt`）。
   - 收紧 [clippy.toml](./clippy.toml) 至：`too-many-lines-threshold = 100`、`cognitive-complexity-threshold = 25`、`too-many-arguments-threshold = 7`、`type-complexity-threshold = 250`、`max-fn-params-bools = 3`、`max-struct-bools = 3`、`enum-variant-size-threshold = 200`。
   - 根 [Cargo.toml](./Cargo.toml) 配置 `[workspace.lints.{rust,rustdoc,clippy}]`：开启 `unsafe_code = "forbid"`、`allow_attributes = "deny"`、`missing_docs = "deny"`、`missing_errors_doc = "deny"`、`missing_panics_doc = "deny"`、`rustdoc::all = "deny"`，并将 `unwrap_used`、`expect_used`、`get_unwrap`、`panic`、`todo`、`unimplemented`、`unreachable`、`indexing_slicing`、`as_conversions`、`as_underscore`、`trivial_casts`、`lossy_float_literal`、`let_underscore_must_use`、`unused_result_ok`、`mem_forget`、`dbg_macro`、`print_stdout`、`print_stderr`、`exit`、`rc_buffer`、`rc_mutex`、`infinite_loop`、`significant_drop_in_scrutinee`、`non_send_fields_in_send_ty` 全部设为 `deny`。
   - 所有成员 Crate 声明 `[lints] workspace = true`，所有库 Crate 的 `src/lib.rs` 顶部声明 `#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。

### 2. 落地零外部依赖的静态守卫脚本（Python >= 3.11 标准库）
1. **`scripts/lint-guard.py`**：
   - 校验 [rust-toolchain.toml](./rust-toolchain.toml)、[Cargo.lock](./Cargo.lock)、`[workspace.lints]`、`[lints] workspace = true` 与 [clippy.toml](./clippy.toml) 阈值完整有效，确认仓内不存在覆写规则的 `.cargo/config*`。
   - 剥离注释与字符串后扫描全仓 `.rs` 源码，拦截 `unsafe`、豁免属性与动态 `include!`；扫描源码与文档拦截外部项目耦合词汇与硬编码端口死值。
2. **`scripts/web-guard.py`**：
   - 校验 `web/tsconfig.json` 开启 `"strict": true`、`noUnusedLocals`、`noUnusedParameters`、`noUncheckedIndexedAccess`。
   - 扫描 `web/src` 拦截 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`@ts-nocheck`、`eslint-disable`、外部 CDN URL，以及未在 `onUnmounted` 中清理的定时器和流连接。
3. **`scripts/embed-guard.py`**：
   - 校验 `web/dist/index.html` 存在，拦截 `*.map`、`.env*`、`node_modules` 或业务样本图片混入 `web/dist`。
   - 校验单文件与总内嵌体积预算，核验 `crates/x-upt/build.rs` 声明 `cargo:rerun-if-changed` 且在 release 构建缺失 `web/dist/index.html` 时非零退出。
4. **`scripts/check_routes.py`**：
   - 提取只读文档门户 `axum::Router` 路由与 `portal-openapi.json` 双向比对，确认只暴露只读方法（`GET` / `HEAD`），同时核验 15 个模拟上游接入面未挂载任何门户路由。
5. **`scripts/docs-and-plans-guard.py`**：
   - 校验全仓 Markdown 相对链接 100% 指向真实文件；校验 [AGENTS.md](./AGENTS.md)、[IRON_RULES.md](./IRON_RULES.md) 与本文件均 `< 24,000` 字节；核验 `.gitignore` 屏蔽 `target/`、`node_modules/`、`web/dist/`、`.env*`、`logs/`、`*.pid`、`config/keys/envelope/`、`data/`。

### 3. 构建无状态自包含底座与只读文档门户
1. **脱离源码自包含启动**：
   - 支持 `--config`（缺省使用内置默认配置）、`--data-dir` 与 `--portal-listen`。
   - 内置 100 张确定性基准样本图与公共演示公钥；当 `--data-dir` 下缺少信封测试密钥时在启动阶段自动生成，保证单二进制放入空目录即可直接运行全部 15 个接入面。
2. **模拟上游接口文档与同源契约导出**：
   - 整理中立、清晰的 [docs/API.md](./docs/API.md)、[docs/SPEC.md](./docs/SPEC.md) 与 `docs/ENVELOPE.md`。
   - 只读门户端口挂载 `/health`、`/api/v1/status`、`/api/v1/fronts`、`/api/v1/contracts/*`（同源输出 `.proto`、`.fds`、`WSDL`、`GraphQL SDL`、`openapi.json`）、`/api/v1/logs/recent`（已脱敏的内存环形缓冲）以及内嵌 SPA 静态资源。
3. **路由防穿透、缓存头与优雅停机**：
   - `/health` 与 `/api/*` 未命中请求返回 `404` JSON，缺失 `/assets/*` 返回 `404`，前端路由返回 `index.html`（`no-cache` + `ETag`），指纹资源返回 `immutable` + `nosniff`。
   - 门户、15 个模拟接入面与 `JobStore` 统一通过 `CancellationToken` 响应 `SIGINT`/`SIGTERM` 优雅退出。

### 4. 串联 `scripts/gate/all.sh` 与 CI / 容器交付
1. **`scripts/gate/all.sh`** 按顺序串联全套门禁并在末尾打印 `ALL GATES PASS`：
   - **G1**：`shellcheck`、`actionlint`、[release-test.sh](./.github/scripts/release-test.sh) 及 5 个 Python 静态守卫脚本。
   - **G2**：`web/` 前端 `pnpm` 安装、lint、typecheck、单测、生产打包 → `embed-guard.py` → Rust 全工作区 `fmt --check`、`clippy -D warnings`、`cargo doc -D warnings`、单元与文档测试。
   - **G3**：`cargo build --release --locked` → 拷贝单二进制至 `TMP_RUN_DIR="$(mktemp -d)"` 空目录并指定独立 `--data-dir` 拉起 → 运行 [test/acceptance.py](./test/acceptance.py) 与 [test/envelope_acceptance.py](./test/envelope_acceptance.py) → 校验门户 `/health`、`/api/v1/status`、5 类同源契约下载、静态资源缓存头、API/静态 `404` 防穿透 → 运行 Playwright `web/e2e/smoke.spec.ts` → 验证 `SIGTERM` 10 秒内 `0` 状态码退出。
2. **对齐 CI 与容器编排**：
   - [ci.yml](./.github/workflows/ci.yml) 按 `scripts` → `frontend`（上传 `web-dist` 产物）→ `rust`（下载 `web-dist` 后执行全套 Rust 验证）组织流水线；
   - [Dockerfile](./Dockerfile) 采用 `node` 前端构建 + `debian:trixie` Rust 编译 + `debian:trixie-slim` 非 root 运行三阶段结构，与 `deploy/compose.yml` 环境变量保持一致。

### 5. 存量违例清理与独立签收
- 收紧 Lints 后逐项消除存量 `.unwrap()` / `.expect()`、`println!` / `eprintln!`、`std::process::exit`、`as` 强转与超长函数，补齐全部 `pub` API Rustdoc。
- 由独立验证上下文执行 `bash scripts/gate/all.sh` 并核对 `git status` 干净无残留，确认末行输出 `ALL GATES PASS` 后完成签收。
