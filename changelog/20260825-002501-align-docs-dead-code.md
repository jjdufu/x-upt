# Changelog — 2026-08-25 00:25:01 +0800

会话/任务主题：手册与实现对齐，清掉死分支，补上流水号/回调/mTLS 缺口

## 1. 背景与目标

- `docs/API.md` / `docs/SPEC.md` 与代码有多处对不上：回调 body、REST 非法 JSON、job 鉴权 `request_id`、SOAP 缺字段填 0、mTLS 只在四面核 CN、GraphQL 示例查不出来
- 目标：对外契约以手册为准改代码；内部描述改 SPEC；删掉走不到的分支和无用接线

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `src/front/util.rs` | `RestJson` 提取器；`rest_short_to_envelope`（`unknown_method`→`40400`） |
| `src/front/rest.rs` | 非法 JSON 走项目失败 JSON；`jobs`/`workers` 只属 rest；SSE query 缺字段进核 |
| `src/front/job.rs` | 鉴权失败带 `request_id`；去掉未用 `_core`/`_callback` |
| `src/front/jsonrpc.rs` | 传入 CN；失败也回调；删 204 死分支 |
| `src/front/soap.rs` | 传入 CN；缺子元素 / 坏数字 Client Fault |
| `src/front/grpc.rs` `ws.rs` `sign_gateway.rs` | 应用层核对 CN |
| `src/front/graphql.rs` | 401 带 `request_id`；`FieldValue::owned_any` 修好检索 |
| `src/jobstore/mod.rs` | 回调 body 与 `GET /jobs/{id}` 相同；超时删除 |
| `src/config.rs` `config/app.toml` | `jobs` 移出公共节；CommonToml 拒未知键 |
| `docs/API.md` `docs/SPEC.md` `spec/oauth_gateway.yaml` | 按现行行为改 |

### 要点

- 回调：oauth 信封；sign 整包（含失败）；jsonrpc `result`/`error`；任务面用 job GET JSON。出站头增加已有的 `X-Upupt-Front`
- REST `envelope=true` 时按短码填信封 `code`，不再一律 `400→40001`
- 九面 mTLS 都把 CN 送进应用层；sign 验签后要求 CN=`app_id`
- GraphQL 手册里的 `postsSearch` curl 以前因 parent 不是 `serde_json::Value` 而 internal，现已修好

## 3. 数据库

| 项 | 结论 |
|---|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- `config/app.toml`：从 `[fronts.oauth_gateway]` 去掉无效的 `jobs = false`
- `jobs` / `workers` 只允许出现在 `[fronts.rest]`（`workers` 默认 4，可不写）
- 别的面写 `jobs` 会启动失败：`unknown field jobs`

## 5. 验证

- `cargo build` PASS
- 启动九面后 curl：REST 非法 JSON 带 `request_id`；SOAP `missing user_id`；job 401 JSON 有 `request_id`；GraphQL 手册示例 id 18 + `extensions.request_id`；GraphQL 401 为 rest 形状；sign HMAC；jsonrpc 检索；job 提交/查询无 `expired`
- 临时配置 `[fronts.graphql] jobs=true` → 启动失败 `unknown field jobs` PASS
- 未测：真 mTLS 证书链（无测试 CA）；回调实打出站（改了组装，未起 notify 接收端）；REST `envelope=true` 的 `40400` 只核对了映射函数

## 6. 部署提示

- 需重建二进制并重启
- 若旧配置在非 rest 面写了 `jobs=`，启动会失败，删掉该键

## 7. 未做 / 明确不做

- 未加测试套件（仓库仍无 `tests/`）
- `[server]` 启动拒绝提示保留（旧配置唯一兼容层）
- `gettoken` 的 `corpid`/`corpsecret`、Bearer 先票后 secret、RSA/SM2 多种 PEM 仍保留

## 8. 文件清单

```
config/app.toml
docs/API.md
docs/SPEC.md
spec/oauth_gateway.yaml
src/auth/sign.rs
src/config.rs
src/front/graphql.rs
src/front/grpc.rs
src/front/job.rs
src/front/jsonrpc.rs
src/front/mod.rs
src/front/rest.rs
src/front/sign_gateway.rs
src/front/soap.rs
src/front/util.rs
src/front/ws.rs
src/jobstore/mod.rs
changelog/20260825-002501-align-docs-dead-code.md
```

---

*记录生成时区：+0800*
