# Changelog — 2026-09-16 16:31:12 +0800

会话/任务主题：访问日志补齐诊断字段（图/Base64 不动）

## 1. 背景与目标

- 原先一行 JSON 能对账 `in`/`out`，但缺业务码、query、鉴权方式、级别、进程、阶段耗时；WS 帧 / job 执行 / batch extra 丢掉 ip 与 bytes
- GraphQL 不落 query 原文
- 目标：同一套 access 行补齐上述字段。比图/出图的 Base64 **不截断、不改**

## 2. 代码改动

| 路径 | 变更 |
|---|---|
| `src/types/logger.rs` `src/logger/` `src/front/access.rs` | 新字段：`level,query,code,duration_auth,duration_core,duration_latency,error_kind,user_agent,referer,content_type,auth,pid,host`；query 脱敏；extra 行带上父请求 ip/bytes |
| `src/auth/mod.rs` | `authenticate` 返回实际 `AuthMethod` |
| `src/core/mod.rs` | `error_kind`；核耗时 |
| `src/config.rs` | `apply_latency` 返回毫秒 |
| 各 `src/front/*` `src/jobstore/` | 写业务码、鉴权、阶段耗时；GraphQL `in` 含 query；WS 帧与 job 执行行带握手/提交元数据 |

### 要点

- `level`：`status>=500` → `error`；`status>=400` 或有 `error` → `warn`；否则 `info`
- `query` 中 `api_key` / `access_token` / `authorization` / `secret` / `client_secret` 记 `***`
- `code` 跟该面对外码（信封数字、REST 短码、JSON-RPC、sign sub_code、SOAP faultcode）
- `auth` 为验过的方式；失败尽量从头推断
- 图相关日志仍全文，未做摘要

## 3. 数据库

| 项 | 结论 |
|---|---|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- 未改 `config/app.toml`

## 5. 验证

- `cargo clippy --bin upupt -- -D warnings`：PASS
- 未跑本机九面联调

## 6. 部署提示

- 重建并重启 `upupt`
- 日志采集若锁死 JSON 键集合，需允许新键：`level` `query` `code` `duration_auth` `duration_core` `duration_latency` `error_kind` `user_agent` `referer` `content_type` `auth` `pid` `host`

## 7. 未做 / 明确不做

- 不截断、不摘要图片 Base64
- 不上 OpenTelemetry，不做日志切割
- 成功 REST JSON 仍不塞 `request_id`

## 8. 文件清单

```
src/types/logger.rs
src/logger/mod.rs
src/logger/context.rs
src/front/access.rs
src/auth/mod.rs
src/auth/oauth.rs
src/core/mod.rs
src/config.rs
src/front/util.rs
src/front/rest.rs
src/front/oauth_gateway.rs
src/front/sign_gateway.rs
src/front/jsonrpc.rs
src/front/graphql.rs
src/front/grpc.rs
src/front/soap.rs
src/front/ws.rs
src/front/job.rs
src/jobstore/mod.rs
docs/SPEC.md
docs/API.md
changelog/20260916-163112-access-log-detail.md
```

---

*记录生成时区：+0800*
