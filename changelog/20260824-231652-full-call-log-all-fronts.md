# Changelog — 2026-08-24 23:16:52 +0800

会话/任务主题：按调用记录方案改流水号与完整 in/out 日志；默认九个接入面全开

## 1. 背景与目标

- 访问日志原先是摘要（`trace`/`params`），WS 业务帧和任务执行不落盘，gRPC 业务写在另一份上下文上
- 流水号没有打穿：REST 成功体无号、WS 非法 id 回旧值、job 回调用 `job_id`
- 默认只开换票网关；改为默认全开，仍可用 toml 关掉某面

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `src/types/logger.rs` `src/logger/` `src/front/access.rs` | 日志字段改为 `request_id,in,out,job_id`；HTTP 出门写主行，batch/多字段写 extra 行 |
| `src/core/mod.rs` | 去掉摘要拼装；`in`=完整 biz，`out`=完整业务 JSON |
| `src/front/grpc.rs` | 共用 access_log 的 `LogContext`；响应 metadata 回 `x-request-id` |
| `src/front/graphql.rs` | `extensions.request_id`；每字段 extra 行 |
| `src/front/jsonrpc.rs` | batch 每元素一行，HTTP 行 `in={"batch":N}` |
| `src/front/ws.rs` | 每帧写日志；回包带 `request_id`；非法 `id` 回新 UUID |
| `src/jobstore/` `src/callback/` | 任务带着提交时的 `request_id`；执行/过期/回调各写一行；回调头用该号 |
| `config/app.toml` `src/config.rs` | 九个接入面默认 `enabled=true` |
| `docs/API.md` `docs/SPEC.md` | 流水号、日志字段、默认全开 |

### 要点

- 对外流水号只叫 `request_id`。合法 `X-Request-Id` 原样用；非法则生成 UUID，回包必须是新值
- `in`/`out` 不截断。不写 `client_secret`、`sign`、access_token 明文
- REST 成功 JSON 仍不塞 `request_id`（对账用响应头）

## 3. 数据库

| 项 | 结论 |
|---|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- `config/app.toml`：`[fronts.*] enabled=true`（3000–3008）
- 代码缺省 `FrontName::default_enabled()` 现为全部 true
- 未改动态配置

## 5. 验证

- `cargo clippy --bin upupt -- -D warnings`：PASS
- 默认进程打印 9 个 front 均 bind
- 网关 `X-Request-Id: rid-1`：头、信封、日志均为 `rid-1`；`in.keyword=gRPC`，`out.items[0].id=18` 全文标题
- 非法头 `rid/1`：头与信封与日志为同一新 UUID
- REST 成功 body 无 `request_id`，头与日志为 `rid-rest`，`out` 等于 body
- JSON-RPC batch：同一 `request_id` 三行（汇总 + gRPC + Redis keyword）
- job `rid-j`：提交行 + 执行行同一号，`out` 含 items；查询 succeeded
- 比图日志 `in.image` 为完整 Base64；出图日志 `out.image.base64` 有内容
- GraphQL `extensions.request_id=rid-gql`
- SOAP 头 `rid-soap`，检索命中
- WS：合法 `id=1` 日志同行；非法 `bad/id` 回包 `id` 仍非法、`request_id` 为 UUID 且日志命中
- 换票日志 `out` 无 access_token 明文；全日志无 `trace`/`params` 键

未测：本机无 grpc 客户端，未打 gRPC RPC（进程已 bind `:3005`）

## 6. 部署提示

- 重建并重启 `upupt`
- 默认同进程听 3000–3008；只要部分形态则把对应 `[fronts.*.enabled]` 设 false
- 日志采集若锁死 JSON 键：`trace`→`request_id`，`params`→`in`+`out`，新增 `job_id`
- 出图/比图日志会变大（含 Base64）

## 7. 未做 / 明确不做

- 未给 gRPC 做本机 RPC 联调
- 未改 OpenAPI YAML / proto / WSDL 机器契约
- REST 的 `sse`/`envelope`/`jobs` 等可选开关仍默认关（只开九个接入面本身）

## 8. 文件清单

```
src/types/logger.rs
src/logger/mod.rs
src/logger/context.rs
src/core/mod.rs
src/front/access.rs
src/front/oauth_gateway.rs
src/front/sign_gateway.rs
src/front/rest.rs
src/front/jsonrpc.rs
src/front/graphql.rs
src/front/grpc.rs
src/front/soap.rs
src/front/ws.rs
src/front/job.rs
src/jobstore/mod.rs
src/callback/mod.rs
src/auth/oauth.rs
src/config.rs
config/app.toml
docs/API.md
docs/SPEC.md
changelog/20260824-231652-full-call-log-all-fronts.md
```

---

*记录生成时区：+0800*
