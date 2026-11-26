# Changelog — 2026-08-23 18:52:52 +0800

会话/任务主题：访问日志统一落盘，对外流水号 `X-Request-Id` 可对账

## 1. 背景与目标

- 原先 `trace` 只在 handler 内生成，不回给客户；401 / 422 / 空 body 无日志；业务 400 不记 error；`/api/*` 不知道是谁
- 目标：客户可传入或收回流水号，一笔请求一行日志，失败也能溯源，图片/密码/超长 keyword 不原样落盘

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `src/middleware/access.rs` | 外层 access log：解析/生成流水号、计时、写一行、回 `X-Request-Id` |
| `src/logger/context.rs` | 请求级 `LogContext`（trace / username / params / error） |
| `src/extract.rs` | `AppJson<T>`：反序列化失败写入 error + `request_id` |
| `src/middleware/auth.rs` | 校验返回 username；401 JSON 带 `request_id` 并记日志上下文 |
| `src/handler/*` | 去掉各自 `log_http`，只填摘要；错误 JSON 带 `request_id` |
| `src/service/token.rs` | `validate_token` → `Option<String>`（绑定用户名） |
| `src/route/router.rs` | fallback 404；套 access_log |
| `src/main.rs` | `into_make_service_with_connect_info::<SocketAddr>()` |
| `docs/API.md` / `docs/SPEC.md` | 按当前流水号与日志约定重写 |

### 要点

- 请求头 `X-Request-Id` 合法则沿用，否则生成 UUID；所有响应都回同一值
- 成功业务 JSON 不塞 `request_id`；错误 JSON 有 `error` + `request_id`
- keyword 超 128 字符截断；imgdef 只记 `image_chars` + `image_head`（12 字符）
- 写盘 `writeln` 后 `flush`；Mutex poison 用 `into_inner`

## 3. 数据库

| 项 | 结论 |
|----|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- 未改 `config/app.toml`

## 5. 验证

- `cargo test --bin upupt`：26 passed
- `cargo clippy --bin upupt -- -D warnings`：PASS
- 本机 HTTP：
  - 客户 `X-Request-Id: client-rid-*` 原样回写；非法 `has space` 则服务端生成 UUID
  - 空 body / 未知字段 422 JSON 带 `request_id` 且有日志
  - 无 Token 401 有日志、`error=Missing authorization token`
  - posts `hit_ids=[18]`，`username=loadtest_001`
  - 200 字符 keyword 日志为 128 字符 + `...(len=200)`
  - imgdef 日志无完整 Base64，有 `image_head` / `image_chars` / `bytes_in`
  - `/api/proxy` 404 有流水号和日志
  - `bytes_out` 有值（token 141、posts 283）
- 未跑 `clippy --all-targets -D warnings`（`gen_imgs` 原有告警）

## 6. 部署提示

- 重建并重启 `upupt`
- 下游应对齐：保存响应头 `X-Request-Id`；错误 body 现含 `request_id`
- 压测脚本可自带 `X-Request-Id` 与 `upupt.log` 的 `trace` JOIN

## 7. 未做 / 明确不做

- 成功 JSON 不增加 `request_id`
- 不上 OpenTelemetry
- 不做日志切割（交给 logrotate）
- 不给上传图做 SHA（用 `image_head`）

## 8. 文件清单

```
docs/API.md
docs/SPEC.md
src/extract.rs
src/handler/error.rs
src/handler/imgdef.rs
src/handler/mod.rs
src/handler/photos.rs
src/handler/posts.rs
src/handler/token.rs
src/logger/context.rs
src/logger/mod.rs
src/main.rs
src/middleware/access.rs
src/middleware/auth.rs
src/middleware/mod.rs
src/route/router.rs
src/service/mod.rs
src/service/token.rs
src/types/logger.rs
changelog/20260823-185252-access-log-request-id.md
```

---

*记录生成时区：+0800*
