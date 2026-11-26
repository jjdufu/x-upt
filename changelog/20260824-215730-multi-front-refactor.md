# Changelog — 2026-08-24 21:57:30 +0800

会话/任务主题：按 `docs/多接入方案.md` 拆业务核与九个接入面，访问日志加 `front`/`port`/`rpc`

## 1. 背景与目标

- 进程原先只绑一个 HTTP 端口、一种报文、一份全局 Token 表；关掉某种能力只能停整进程
- 按方案改成一层业务核 + 多个隔离接入面；删除 `[server]` 与旧路由/handler，不留兼容层
- 访问日志在原有一行 JSON 上增加接入面字段

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `src/core/mod.rs` | `Call` / `CoreError` / `dispatch`；三 method 计算仍走原 service |
| `src/config.rs` | `[fronts.*]`、`[methods]`、`[latency]`、`[sign]`、`[callback]`、`[job]`；有 `[server]` 则启动失败 |
| `src/auth/` | `TokenHub` 按接入面分桶；OAuth 换票、API Key、HMAC/RSA2/SM2、mTLS CN、nonce |
| `src/front/` | 九个接入面独立 bind / 路由 / 鉴权；HTTP 监听共用 `http_listen`（TLS/h2/h2c） |
| `src/logger/` `src/types/logger.rs` | `LogEntry` 增加 `front` `port` `rpc`；可选按面拆文件 |
| `src/callback/` `src/jobstore/` | 出站回调队列；按面隔离的任务表 |
| `proto/upupt.proto` `build.rs` | gRPC 契约与代码生成 |
| 删除 `src/handler/` `src/route/` `src/middleware/` `src/extract.rs` `src/service/token.rs` | 无兼容层 |

### 要点

- 默认只开 `oauth_gateway`，监听 `0.0.0.0:3000`。其余接入面配置写出但 `enabled=false`，不 bind
- 同一 `client_id` 可打所有已开面；票不跨面。REST 的 `X-API-Key` 只在 rest 面生效
- 业务 method 名与检索/出图/比图算法不变。`keyword=gRPC,user_id=6` 仍命中 id 18；同文件比图 `distance=0`
- 日志仍一行 JSON。新字段：`front`、`port`、`rpc`（换票/鉴权失败未进核时 `rpc` 为 `null`）
- `[log.split_by_front]=true` 时写 `logs/<stem>.<front>.log`

## 3. 数据库

| 项 | 结论 |
|---|---|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- `config/app.toml` 删除 `[server]`，改为方案中的 `[fronts.*]` 全表。旧配置若仍有 `[server]`，进程非 0 退出，提示改到 `fronts.oauth_gateway`
- 可执行文件同级仍需 `config/app.toml`。`cargo run` / `target/debug/upupt` 需把 `config/` 放到二进制旁边

## 5. 验证

- `cargo clippy --bin upupt -- -D warnings`：PASS
- 默认进程：只听 3000；3001–3008 连接失败
- `POST :3000/v3/oauth/token` + `POST :3000/v3/gateway` 检索 gRPC / user_id=6 → id 18；比图 `img_001.png` 同文件 `distance=0`
- 临时打开 sign+rest：3002 `X-API-Key` 检索字段与网关 `data` 一致；3001 HMAC `upupt_posts_search_response.code=10000`；3000 票打 3002 → 401，3002 票打 3000 → 401
- 访问日志含 `front`/`port`/`rpc`
- 带 `[server]` 的配置启动失败：`[server] is removed; move host/port to [fronts.oauth_gateway]`

## 6. 部署提示

- 重建并重启 `upupt`
- 压测基址从 `3001` 改为 `3000`（或只改 toml 的 `fronts.oauth_gateway.port`）
- 日志采集若锁死 JSON 键集合，需允许新键 `front` `port` `rpc`

## 7. 未做 / 明确不做

- 未对 GraphQL/gRPC/SOAP/WS/job 做本机联调（模块已链接，默认 `enabled=false`）
- 未生成示例 TLS/RSA/SM2 密钥；未开 callback
- 不做日志切割（仍交给 logrotate）；`split_by_front` 只按接入面分文件

## 8. 文件清单

```
Cargo.toml
Cargo.lock
build.rs
config/app.toml
config/keys/.gitkeep
proto/upupt.proto
docs/API.md
docs/SPEC.md
docs/openapi.yaml
docs/api-*.md
docs/openapi-*.yaml
docs/upupt.wsdl
src/main.rs
src/config.rs
src/core/mod.rs
src/auth/*
src/front/*
src/callback/mod.rs
src/jobstore/mod.rs
src/logger/*
src/service/apps.rs
src/service/mod.rs
src/types/logger.rs
src/types/gateway.rs
changelog/20260824-215730-multi-front-refactor.md
```

删除：`src/extract.rs`、`src/handler/*`、`src/middleware/*`、`src/route/*`、`src/service/token.rs`

---

*记录生成时区：+0800*
