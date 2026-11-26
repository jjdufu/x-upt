# x-upt 全栈工程铁律与三级门禁规范（MANDATORY）

> **本文件为 `x-upt` 仓库的工程架构、代码质量与发布门禁规范，与 [AGENTS.md](./AGENTS.md) 配合生效；配套的基建与门禁落地施工指引见 [GATE_BOOTSTRAP.md](./GATE_BOOTSTRAP.md)。所有开发者、主控代理（Orchestrator）与子代理（Subagent）在新建模块、重构拆分、修改代码、提交 PR 或宣告任务完成前，必须遵守本规范。**

---

## 0. 仓库基准约定（对齐 `x-loadgen` 全栈标准）

| 基准项 | 初始基线（单体阶段） | 目标架构（全栈 Workspace 形态） |
|---|---|---|
| **Rust 根目录** | `.`（单体 Crate） | `.`（Cargo Workspace：`x-upt-bin/x-upt` + `x-upt-libs/*`，主二进制 `x-upt`） |
| **前端目录** | 暂无 | `x-upt-bin/x-upt/web`（Vue 3 + TypeScript + Vite + pnpm + Reka UI），产物 `x-upt-bin/x-upt/web/dist` |
| **管理面端口与前缀** | 独立管理端口 `18768` | 独立管理端口 `18768`，API 前缀 `/api/v1`，健康检查 `/health` |
| **模拟上游业务端口** | `16000`–`16008`（9 个核心面）+ `16010`–`16015`（6 个信封面） | 同左（`16000`–`16015` 端口段，含默认启用的 gRPC `:16005`），与管理面 `18768` 物理隔离 |
| **活态契约真源** | [docs/SPEC.md](./docs/SPEC.md)、[docs/API.md](./docs/API.md)、[proto/upupt.proto](./proto/upupt.proto)、[spec/upupt.wsdl](./spec/upupt.wsdl) | 管理面：`x-upt-bin/x-upt/openapi.json`；上游模拟面：`docs/SPEC.md` + 协议契约文件 |
| **运行时数据目录** | 二进制同级 `config/`、`imgs/`、`logs/` | `--data-dir` / `X_UPT_DATA_DIR`（容器 `/data`：SQLite、规则配置、密钥、图片、日志） |
| **容器与编排工程** | [Dockerfile](./Dockerfile) + [scripts/run.sh](./scripts/run.sh) | 同 `x-loadgen/deploy/` 自包含 Compose 工程（`config/compose.yml`、`scripts/*.sh`、`lib/common.sh`、`common/base.yml`） |

> **演进原则**：规则分「**现行强制**」与「**目标态启用**（引入 Cargo Workspace / `web/` / SQLite / 守卫脚本时同步解锁生效）」。禁止以阶段性演进为由绕过已启用的门禁或引入新债务。

---

## 一、架构解耦与「单二进制内嵌 Web」核心铁律

### 1. Rust Workspace 依赖倒置与入口防膨胀
1. **单向依赖边界（Workspace 形态强制）**：
   - `x-upt-domain`（纯契约层）：仅允许定义纯领域模型（`struct`/`enum`）、Mock 规则匹配模型、故障注入模型、统一错误类型（`AppError`）、纯计算逻辑与端口 `trait`（异步端口优先返回 `impl Future<Output = Result<T, AppError>> + Send`；仅动态注册表使用对象安全 `Arc<dyn ...>`）。**严禁引入 `axum`、`tonic`、`rusqlite`、`rust-embed` 或其他网络框架**。
   - 能力实现 Crate（如 `x-upt-crypto`、`x-upt-store`、`x-upt-core`、`x-upt-fronts`）：面向领域 `trait` 编程，能力层之间严禁跨 Crate 耦合内部实现。
   - 装配与二进制入口 Crate（`x-upt-bin/x-upt`）：唯一允许组装 `AppState`、挂载管理面路由、绑定业务接入面端口并内嵌前端静态产物的 Crate。
2. **端口物理隔离与入口防冲突**：
   - **管理面与业务接入面严格分端口监听**：WebUI 与 `/api/v1/*` 管理接口监听 `18768`，严禁挂载到 `16000`–`16015` 模拟上游接入端口上，避免污染上游协议契约、鉴权语义与访问日志（`access_log`）。
   - `src/main.rs` 仅负责 CLI/配置加载、日志与数据目录初始化、优雅停机信号与各监听服务装配，严禁堆砌业务 Handler。
   - 管理面 HTTP Handler 按领域拆分为 `src/api/<module>.rs`（参考 `x-loadgen`），通过 `Router::new()` 组合；模拟上游接入面继续按协议隔离在 `src/front/<front>.rs`。
3. **前端模块隔离（对齐 `x-loadgen` Vue 3 架构）**：
   - 统一使用 `pnpm` + Vue 3 + TypeScript + Vite + Tailwind + Reka UI。
   - HTTP 客户端按领域拆分为 `src/api/<module>.ts`（或 `src/api/http.ts` + 领域模块），严禁塞入单一巨型文件；页面位于 `src/views/`，基础交互原语收口于 `src/components/ui/Ui*.vue`，`src/App.vue` 与 `src/router/index.ts` 仅做布局、全局 Provider 与路由装配。

### 2. 内嵌二进制 Web 资源（Embedded SPA）五条硬约束
1. **构建时序与新鲜度物理锁死**：
   - `x-upt-bin/x-upt/build.rs` 通过 `rust-embed` 读取 `web/dist`，必须输出 `cargo:rerun-if-changed=web/dist` 与 `cargo:rerun-if-changed=web/dist/index.html`，并遵循 `#[cfg(frontend_embedded)]` 门控（保留 `X_UPT_SKIP_FRONTEND_EMBED=1` 仅用于纯后端 `cargo test`）。
   - **`--release` 构建、Dockerfile 构建与 CI 模式下严禁允许空 `web/dist` 或占位 HTML 静默通过编译**；`web/dist/index.html` 缺失时 `build.rs` 与 `embed-guard.py` 必须立即报错阻断。
2. **API 路由与 SPA Fallback 严格物理隔离（防吞 404）**：
   - 管理面监听上所有 `/health`、`/api/*`（含 `/api/v1/*`）、`/metrics` 未命中的请求**必须返回结构化 JSON `404 Not Found`（`Content-Type: application/json`）**，**绝对禁止**穿透到 SPA Fallback 返回 `200 OK` 的 `index.html`。
   - 仅非保留前缀、请求方法为 `GET`/`HEAD` 且路径不含扩展名的前端路由，允许回退返回内嵌的 `index.html`；缺失的具体静态文件（如 `/assets/missing.js`）必须返回真实 `404`。
3. **编译期 MIME、防嗅探与双轨缓存策略**：
   - 内嵌资源响应必须精确设置 `Content-Type`（`.html`、带 `charset=utf-8` 的 `.js`/`.mjs`/`.css`、`.wasm -> application/wasm`、`.svg -> image/svg+xml`、`.woff2`）并统一附带 `X-Content-Type-Options: nosniff`。
   - 带内容哈希指纹的产物（`/assets/*`）强制返回 `Cache-Control: public, max-age=31536000, immutable`；入口 `index.html` 等无哈希文件强制返回 `Cache-Control: no-cache` 并配对内容哈希 `ETag`（支持 `If-None-Match` 返回 `304`）。
4. **内嵌体积预算与业务素材物理分离**：
   - 严禁将 `*.map`（SourceMap）、`.env*`、`node_modules` 碎片或 `imgs/` 样本图打入 `web/dist` 进而永久驻留到前端静态资产包中。
   - 业务所需的测试图片（`imgs/img_001.png`–`img_100.png`）与可生成契约文件（如内置 `FileDescriptorSet`、WSDL）走独立运行目录或专用下载 API，不与前端 SPA 混包。
   - 开启预压缩（构建期 `.br`/`.gz` 或运行期 `CompressionLayer`），对已压缩二进制媒体免除二次压缩。
5. **全端口优雅停机与安全边界**：
   - 管理面（`18768`）、`16000`–`16015` 全部已启用接入面及 `JobStore` / 日志刷盘后台任务，必须统一接入 `CancellationToken` 监听 `SIGINT`/`SIGTERM`（容器内由 `tini` 转发），在超时窗口内释放监听与连接后以状态码 `0` 退出。
   - 管理面必须复刻 `x-loadgen` 的安全守卫：默认开启首登随机口令（`admin.passwd` + 强制改密，支持 `--no-auth` 联调开关）以及 `origin_and_host_guard`（防 CSRF 与非回环 `Host` 头攻击）。

### 3. `x-upt` 模拟上游核心契约不可破坏原则
1. **基线兼容性保护**：为 `posts`、`photos`、`imgdef` 及信封 6 面提供 WebUI 可配置模拟数据、故障注入或动态路由时，默认/基线配置下必须继续 100% 通过 [test/acceptance.py](./test/acceptance.py) 与 [test/envelope_acceptance.py](./test/envelope_acceptance.py)。
2. **契约文件同源生成**：页面生成或导出的 gRPC `FileDescriptorSet`（`.fds`/`.pb`，支持单 `.proto` 文本与多文件 `import` 压缩包编译）、`.proto`、SOAP WSDL、GraphQL SDL 与 OpenAPI JSON，必须与运行期实际服务的协议定义同源（编译期嵌入或运行期由同一份 AST/DescriptorPool 实时驱动），严禁手工维护两份分叉文件。
3. **热配置与零全局锁阻塞**：Mock 规则、延迟/故障注入与动态路由表支持运行期热更新（如 `ArcSwap` / `RwLock`），读路径禁止跨越 `.await` 持锁。

---

## 二、代码质量零容忍红线

### 1. Rust 零绕过红线
- **工具链与锁文件锁死**：[rust-toolchain.toml](./rust-toolchain.toml) 固定 Rust 版本（`1.100.0`）并显式包含 `clippy`、`rustfmt`；仓库**必须提交 [Cargo.lock](./Cargo.lock)**，CI 与门禁全程加 `--locked`。
- **Lints 统一管控（严禁局部放宽）**：
  - 根 `Cargo.toml` 统一定义 `[workspace.lints.{rust,rustdoc,clippy}]`，所有成员 Crate 必须声明 `[lints] workspace = true`。
  - **严禁在成员 `Cargo.toml`、`.cargo/config.toml` 或 [clippy.toml](./clippy.toml) 中覆写放宽规则、注入 `rustflags` 或覆盖 `alias`**（仓库内 `.cargo/config.toml` 不得提交入库；宿主机联调环境可能通过父目录 `.cargo` 配置内网私库，CI 始终直连 crates.io sparse）。
  - 所有库 Crate 的 `src/lib.rs` 顶部必须声明：`#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]`。
- **零豁免属性与零 `unsafe`（现行强制）**：
  - 全仓 `.rs` 源码严禁出现 `unsafe`、`#[allow(...)]`、`#![allow(...)]`、`#[expect(...)]`、`#![expect(...)]`、`#[warn(...)]`、`cfg_attr(..., allow/expect/warn)`、`#[doc(hidden)]`、`#[rustfmt::skip]`、`cfg(false)`、`cfg(any())` 以及动态拼接代码的 `include!`。
- **严禁崩溃、越界与未完成占位**：
  - 业务代码严禁 `unwrap()` / `expect()` / `todo!()` / `unimplemented!()`。
  - 目标态全仓 `deny`：`unwrap_used`、`expect_used`、`get_unwrap`、`panic`、`todo`、`unimplemented`、`unreachable`、`indexing_slicing`；切片访问一律用 `.get(...)`、`.first()`、`.split_first()` 或迭代器。
- **严禁隐式强转、静默吞错与标准输出**：
  - 目标态全 `deny`：`as_conversions`、`as_underscore`、`trivial_casts`、`lossy_float_literal`（统一用 `TryFrom` / `From`）；`let_underscore_must_use`、`unused_result_ok`、`mem_forget`（禁止静默吞错）；`dbg_macro`、`print_stdout`、`print_stderr`、`exit`（统一使用结构化 `tracing` 与 `ExitCode`）。
- **异步并发与复杂度上限**：
  - `rc_buffer`、`rc_mutex`、`infinite_loop`、`significant_drop_in_scrutinee`、`non_send_fields_in_send_ty` 在目标态全部 `deny`。
  - [clippy.toml](./clippy.toml) 目标上限：`too-many-lines-threshold = 100`、`cognitive-complexity-threshold = 25`、`too-many-arguments-threshold = 7`、`type-complexity-threshold = 250`、`max-fn-params-bools = 3`、`max-struct-bools = 3`、`enum-variant-size-threshold = 200`（大错误类型采用 `Box` 包装）。
- **全量 Rustdoc 覆盖**：
  - `missing_docs`、`missing_errors_doc`、`missing_panics_doc`、`rustdoc::all`（含 `unescaped_backticks`）全部 `deny`；所有返回 `Result` 的 `pub` 函数必须包含 `/// # Errors` 段落。

### 2. TypeScript / Web 前端红线
- **TypeScript 严格零逃逸**（对齐 `x-loadgen` 前端标准）：开启 `"strict": true`、`"noUnusedLocals": true`、`"noUnusedParameters": true`、`"noUncheckedIndexedAccess": true`。全仓严禁 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`@ts-nocheck`、`eslint-disable`；外部响应声明为 `unknown` 并经类型守卫窄化。
- **组件原语、资源释放与可测试性**：
  - 表单与弹窗交互统一复用 `src/components/ui/Ui*.vue` 原语，严禁业务页面绕过设计系统直接使用裸原生 `<select>` 或硬编码脱离主题变量的色值。
  - Canvas / ECharts / EventSource / WebSocket 组件在 `onUnmounted` 时必须执行销毁与监听清理。
  - 关键交互按钮、表单控件、抽屉弹窗与状态徽标必须提供稳定语义化的 `data-testid`。

---

## 三、三级强制门禁矩阵（G1 → G2 → G3）

任何提交、PR 审查或 AI 代理汇报“已完成”前，必须按顺序跑通对应门禁；任一步骤返回非 `0` 退出码或产生任何 Warning 即视为阻断（BLOCK）。（注：标注 `[目标态]` 的脚本随对应能力模块引入时同步落地并启用。）

### G1 · 毫秒级静态与脚本守卫（每次提交必跑）
1. **Shell 与工作流检查（现行强制，与 [ci.yml](./.github/workflows/ci.yml) 对齐）**：
   - `git ls-files '*.sh' | xargs shellcheck`
   - `actionlint -color`（读取 [.github/actionlint.yaml](./.github/actionlint.yaml)）
   - `./.github/scripts/release-test.sh`
2. `[目标态]` `python3 scripts/lint-guard.py`：核验 `rust-toolchain.toml`、`Cargo.lock`、`[workspace.lints]`、`clippy.toml` 及全量 `.rs` 零豁免 / 零 `unsafe` / 零静默吞错。
3. `[目标态]` `python3 scripts/web-guard.py`：核验 `tsconfig.json` Strict 模式、零 `any`、零 `@ts-ignore`/`eslint-disable`、UI 原语及 `onUnmounted` 清理。
4. `[目标态]` `python3 scripts/check_routes.py` / 路由契约单测：管理面 Router 与 `openapi.json` 双向比对（参考 `x-loadgen` `openapi.rs`），阻断暗路由、空头路由以及业务端口混入管理面路径。
5. `[目标态]` `python3 scripts/docs-and-plans-guard.py`：核验 Markdown 相对链接可达、记忆文件（`AGENTS.md`、`IRON_RULES.md`）单文件体积 `< 24,000` 字节、`.gitignore` 屏蔽 `target/`、`node_modules/`、`web/dist`、`.env*`、`logs/`、`*.pid`、`config/keys/envelope/`、`data/`、`backups/`。

### G2 · 分栈构建、内嵌产物审计与单元验证
> **顺序铁律（启用 `web/` 后）**：因 Rust 编译需内嵌 `web/dist`，必须**先前端构建、再内嵌产物审计、最后 Rust 全套验证**。

```bash
# Step 2.1 [启用 web/ 后] 前端检查与生产打包（pnpm + Vite，与 x-loadgen 一致）
(cd x-upt-bin/x-upt/web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test && pnpm build)

# Step 2.2 [启用 web/ 后] 内嵌产物纯净度与体积守卫（拦截缺失 index.html、残留 *.map / .env、单包超限）
python3 scripts/embed-guard.py

# Step 2.3 Rust 零警告全套验证（拆为 Workspace 后加 --workspace --all-features + cargo doc）
cargo fmt --all -- --check && \
  cargo clippy --all-targets --locked -- -D warnings && \
  cargo test --locked --no-fail-fast
# [Workspace 全量命令]
# cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
# RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked
# cargo test --workspace --locked && cargo test --workspace --doc --all-features --locked
```

### G3 · 单二进制脱离源码黑盒交付与 E2E 终验（`scripts/gate/all.sh`）
发布前与功能闭环验收必须通过**「脱离源码空目录单二进制黑盒测试」**，杜绝运行期依赖源码树导致的交付故障：
1. 执行 `cargo build --release --locked` 产出单一二进制文件（及容器镜像 `docker build` 验证）。
2. 创建与代码仓物理隔离的临时空目录 `TMP_RUN_DIR="$(mktemp -d)"`，仅放入该二进制文件与独立 `--data-dir`（不拷贝 `web/` 源码或 `web/dist/`），在 `TMP_RUN_DIR` 拉起服务并绑定测试端口（管理面 `18768` + 模拟上游端口 `16000`–`16015`）。
3. 自动执行以下黑盒硬断言：
   - **模拟上游基线回归（现行强制）**：执行 `python3 test/acceptance.py` 与 `python3 test/envelope_acceptance.py`，以及 gRPC `.fds` 下载（单文件与多文件 `import` 编译导出）和 RPC 调用冒烟，全部通过。
   - **管理面健康检查 `[启用 web/ 后]`**：`GET /health` 返回 `200` JSON。
   - **内嵌首页与深层客户端路由 `[启用 web/ 后]`**：`GET /` 与 `GET /app/deep-route-check` 均返回 `200`、`Content-Type: text/html`、`Cache-Control: no-cache` 且响应体包含 `/assets/` 指纹脚本引用。
   - **内嵌指纹静态资源 `[启用 web/ 后]`**：提取首页引用的 `/assets/*.js` 与 `*.css` 发起请求，断言 `200`、精确 MIME 类型、`Cache-Control: public, max-age=31536000, immutable`、`X-Content-Type-Options: nosniff`，且字节哈希与构建期 `web/dist` 完全一致。
   - **API 404 防穿透断言 `[启用 web/ 后]`**：`GET /api/v1/__nonexistent_probe__` 必须返回 `404` 且 `Content-Type: application/json`（严禁返回 HTML）。
   - **缺失静态文件防穿透断言 `[启用 web/ 后]`**：`GET /assets/__nonexistent__.js` 必须返回 `404`（严禁回退为 `index.html`）。
   - **真实浏览器冒烟 E2E `[启用 web/ 后]`**：Playwright 直连独立单二进制管理面（`18768`）跑通 `e2e/smoke.spec.ts`（首屏渲染成功、`console.error` 与 `pageerror` 为 `0`、模拟规则与 gRPC 文件生成读写闭环）。
   - **优雅停机断言**：向进程发送 `SIGTERM`，断言进程在停机窗口内（≤ 10 秒）以退出码 `0` 干净退出。
4. 门禁脚本末行必须输出固定口令：`ALL GATES PASS`（单步门禁输出 `GATE stepNN PASS`）；未见该口令严禁合入 `main`。

---

## 四、契约先行与多代理（Multi-Agent）协作纪律

1. **两段式合流（Contract-First）**：跨前后端或跨协议改动先锁定契约（幂等存储 Schema / 迁移 + 领域 DTO/Trait + `openapi.json` / `docs/SPEC.md` + 前端 `src/api/*.ts` 类型签名 + 门禁断言），再在独立 `git worktree` 中并行开发后端与前端实现。
2. **存储迁移只追加不篡改**：引入 SQLite 持久化后，已合入主干的迁移/初始化 SQL 严禁原地修改；结构演进一律新增递增版本迁移，DDL 保持幂等（`IF NOT EXISTS`），并与 `x-loadgen` 一样对不兼容主版本做显式版本校验提示。
3. **编写与审查分离（禁止自写自审）**：负责编码的 `Author` 完成后，必须由独立上下文的 `Verifier` 重新运行 G1 + G2 + G3 命令并核对 `git status` 无缓存产物残留，凭真实终端全绿输出签发 `PASS / BLOCK` 结论。
