# x-upt 基建落地与三级门禁执行指引（GATE BOOTSTRAP）

> **用途**：本文件是 [IRON_RULES.md](./IRON_RULES.md) 的配套施工说明，供开发者与代理在拆分 `x-upt` Cargo Workspace、落地 `SQLite` 持久化与接口开关前置门控、建设内嵌 WebUI 控制台与在线接口文档、以及打通 G1–G3 门禁时按序执行。

---

## 0. 落地前核对清单

动手修改前，先核对仓内现有基线文件：
- 工程规范：[AGENTS.md](./AGENTS.md)、[IRON_RULES.md](./IRON_RULES.md)
- 构建与发版工作流：[Cargo.toml](./Cargo.toml)、[crates/x-upt/build.rs](./crates/x-upt/build.rs)、[clippy.toml](./clippy.toml)、[rust-toolchain.toml](./rust-toolchain.toml)、[Dockerfile](./Dockerfile)、[ci.yml](./.github/workflows/ci.yml)、[release.yml](./.github/workflows/release.yml)、[release-test.sh](./.github/scripts/release-test.sh)
- 模拟上游协议实现与验收脚本：[docs/SPEC.md](./docs/SPEC.md)、[docs/API.md](./docs/API.md)、[scripts/run.sh](./scripts/run.sh)、[test/acceptance.py](./test/acceptance.py)、[test/envelope_acceptance.py](./test/envelope_acceptance.py)

---

## 一、五步落地流程

### 1. 拆分 Cargo Workspace、移除预置素材与锁死 Lints
1. **Workspace 五层结构与零预置清理**：
   - 根 [Cargo.toml](./Cargo.toml) 配置为 `[workspace]`，按单向依赖拆分为：
     - `crates/x-upt-core`：纯领域模型、开关前置校验规则、`posts` / `photos` / `imgdef` 确定性业务核、协议文档元数据模型与 `AppError`（零网络、数据库、嵌入依赖）；
     - `crates/x-upt-crypto`：HTTP 鉴权原语与信封 `canon` / `crypt` 算法及密钥格式校验；
     - `crates/x-upt-store`：bundled `rusqlite`（WAL 模式，`x-upt.db`）、幂等版本迁移与事务级前置条件门控；
     - `crates/x-upt-fronts`：`FrontSupervisor` 15 个模拟接入面热绑定/热卸载生命周期管理与同源契约服务；
     - `crates/x-upt`：主二进制 `x-upt`、CLI、`/api/v1/*` 控制台路由（默认免登录，支持 `--admin-token` / `X_UPT_ADMIN_TOKEN`）与 `rust-embed` 静态资源服务。
   - 统一包名、二进制名、协议契约文件（`proto/x_upt.proto`、`spec/x_upt.wsdl`）为中立 `x-upt` 命名，清理仓内预置样本图目录、图片生成工具与预置演示密钥文件，同步更新 [scripts/run.sh](./scripts/run.sh)、`.github/scripts/release.sh` 与 [release-test.sh](./.github/scripts/release-test.sh)。
2. **工具链、复杂度阈值与 Workspace Lints**：
   - 保持 [rust-toolchain.toml](./rust-toolchain.toml) 固定 `1.100.0`（含 `clippy`、`rustfmt`）。
   - 收紧 [clippy.toml](./clippy.toml) 至：`too-many-lines-threshold = 100`、`cognitive-complexity-threshold = 25`、`too-many-arguments-threshold = 7`、`type-complexity-threshold = 250`、`max-fn-params-bools = 3`、`max-struct-bools = 3`、`enum-variant-size-threshold = 200`。
   - 根 [Cargo.toml](./Cargo.toml) 配置 `[workspace.lints.{rust,rustdoc,clippy}]`：`unsafe_code = "forbid"`、`allow_attributes = "deny"`、`missing_docs = "deny"`、`missing_errors_doc = "deny"`、`missing_panics_doc = "deny"`、`rustdoc::all = "deny"`，以及 `unwrap_used`、`expect_used`、`get_unwrap`、`panic`、`todo`、`unimplemented`、`unreachable`、`indexing_slicing`、`as_conversions`、`as_underscore`、`trivial_casts`、`lossy_float_literal`、`let_underscore_must_use`、`unused_result_ok`、`mem_forget`、`dbg_macro`、`print_stdout`、`print_stderr`、`exit`、`rc_buffer`、`rc_mutex`、`infinite_loop`、`significant_drop_in_scrutinee`、`non_send_fields_in_send_ty` 全设 `deny`。
   - 所有成员声明 `[lints] workspace = true`，所有库 Crate 的 `src/lib.rs` 顶部加 `#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。

### 2. 落地零外部依赖的静态守卫脚本（Python >= 3.11 标准库）
1. **`scripts/lint-guard.py`**：
   - 核验 [rust-toolchain.toml](./rust-toolchain.toml)、[Cargo.lock](./Cargo.lock)、`[workspace.lints]`、[clippy.toml](./clippy.toml) 及零 `.cargo/config*` 覆写。
   - 剥离注释与字符串后扫描全仓 `.rs` 拦截 `unsafe`、豁免属性与动态 `include!`；拦截外部耦合词汇与硬编码端口常量；断言仓库内不存在预置图片目录或明文密钥文件。
2. **`scripts/web-guard.py`**：
   - 核验 `web/tsconfig.json` 开启 `"strict": true`、`noUnusedLocals`、`noUnusedParameters`、`noUncheckedIndexedAccess`；拦截 `any`、注释豁免、外部 CDN URL，校验 `src/components/ui/Ui*.vue` 原语收口与 `onUnmounted` 清理。
3. **`scripts/embed-guard.py`**：
   - 校验 `web/dist/index.html` 存在，拦截 `*.map`、`.env*`、`node_modules` 混入 `web/dist`，校验内嵌体积预算及 `crates/x-upt/build.rs` 的 `rerun-if-changed` 与 release 缺失阻断逻辑。
4. **`scripts/check_routes.py`**：
   - 双向比对控制台 `axum::Router` 与 `admin-openapi.json`，同时校验 15 个模拟上游接入面未挂载任何控制台路径。
5. **`scripts/docs-and-plans-guard.py`**：
   - 校验全仓 Markdown 相对链接 100% 可达；校验 [AGENTS.md](./AGENTS.md)、[IRON_RULES.md](./IRON_RULES.md) 与本文件均 `< 24,000` 字节；核验 `.gitignore` 屏蔽 `target/`、`node_modules/`、`web/dist/`、`.env*`、`logs/`、`*.pid`、`data/`。

### 3. 落地 SQLite 前置门控引擎、控制台路由与在线文档
1. **默认全关与开关门控**：
   - 全新 `--data-dir` 初始化 `x-upt.db` 时仅监听控制台端口（默认 `18768`），15 个模拟接入面与 3 个核心业务接口初始全部为 `enabled = false`。
   - 未导入对应场景图片时拒绝开启 `photos.render`（返回 `409`），图片库为空时拒绝开启 `imgdef.compare`，帖子为空时拒绝开启 `posts.search`，缺少有效凭据/密钥或未开启任何业务接口时拒绝开启对应协议接入面；接口开启期间禁止删除其依赖的最后一条前置资源。
2. **端口物理隔离、404 防穿透与同源文档**：
   - `/health` 与 `/api/*` 未命中路径返回 `404` `application/json`，缺失 `/assets/*` 返回 `404`，前端路由返回 `index.html`（`no-cache` + `ETag`），指纹静态资源返回 `immutable` + `nosniff`。
   - 整理中立清晰的 [docs/API.md](./docs/API.md)、[docs/SPEC.md](./docs/SPEC.md) 与 `docs/ENVELOPE.md`，并在 WebUI 提供 15 个接入面的实时在线文档与 `.proto`、`.fds`、`WSDL`、`SDL`、`openapi.json` 同源导出。

### 4. 串联 `scripts/gate/all.sh` 与 CI / 容器交付
1. **`scripts/gate/all.sh`** 串联全链路并在末行输出 `ALL GATES PASS`：
   - **G1**：`shellcheck`、`actionlint`、[release-test.sh](./.github/scripts/release-test.sh) 及 5 个 Python 守卫脚本。
   - **G2**：`web/` 前端 `pnpm` 全套检查与构建 → `embed-guard.py` → Rust 全工作区 `fmt --check`、`clippy -D warnings`、`cargo doc -D warnings`、单元与文档测试。
   - **G3**：`cargo build --release --locked` → 拷贝单二进制至 `TMP_RUN_DIR="$(mktemp -d)"` 空目录并指定空 `--data-dir` 拉起 → 断言初始 0 开启、0 素材且前置不满足时开关请求全部返回 `409` → 通过 `/api/v1/*` 导入测试图片、帖子、凭据与信封密钥并热开启全部开关 → 跑通 [test/acceptance.py](./test/acceptance.py) 与 [test/envelope_acceptance.py](./test/envelope_acceptance.py) → 验证重启后 `SQLite` 状态恢复与关闭开关端口释放 → 验证静态缓存头、同源契约下载、`404` 防穿透与 Playwright `web/e2e/smoke.spec.ts` → 验证 `SIGTERM` 10 秒内 `0` 退出码。
2. **对齐 CI 与容器编排**：
   - 更新 [ci.yml](./.github/workflows/ci.yml)（`scripts` → `frontend` → `rust`）与三阶段 [Dockerfile](./Dockerfile)，配套 `deploy/compose.yml` 持久化挂载 `/data`。

### 5. 存量违例清理与独立签收
- 逐项清零存量 `.unwrap()` / `.expect()`、`println!` / `eprintln!`、`std::process::exit`、`as` 强转与超长函数，补齐全部 `pub` API Rustdoc。
- 由独立验证上下文运行 `bash scripts/gate/all.sh` 并检查 `git status`，确认输出 `ALL GATES PASS` 后完成签收。
