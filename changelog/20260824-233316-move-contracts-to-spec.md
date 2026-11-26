# Changelog — 2026-08-24 23:33:16 +0800

会话/任务主题：机器契约迁出 `docs/`，删除四份占位 OpenAPI，拆掉对应发现接口

## 1. 背景与目标

- `docs/` 里混着给人看的 md 和编译进二进制的 YAML/WSDL；四份 `openapi-*.yaml` 只有 path 清单、不是合法完整 OAS，却挂在 `GET /openapi.yaml`
- 目标：`docs/` 只留调用说明与规范；真契约放到 `spec/`；占位 YAML 与空壳发现接口一并去掉

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `spec/oauth_gateway.yaml` | 由 `docs/openapi.yaml` 迁入；oauth_gateway 仍 `include_str`，`GET /v3/openapi.yaml` 不变 |
| `spec/upupt.wsdl` | 由 `docs/upupt.wsdl` 迁入；soap 仍 `GET /soap` 返回 |
| `docs/openapi-*.yaml` | 删除 sign / rest / jsonrpc / job 四份占位 |
| `src/front/{sign_gateway,rest,jsonrpc,job}.rs` | 去掉 `GET /openapi.yaml` |
| `src/front/oauth_gateway.rs` `src/front/soap.rs` | `include_str` 改到 `spec/` |
| `docs/API.md` `docs/SPEC.md` | 契约位置与路径表按现状改写 |

### 要点

- **破坏性**：`sign_gateway` / `rest` / `jsonrpc` / `job` 的 `GET /openapi.yaml` 现为 404。oauth 的 `GET /v3/openapi.yaml`、soap 的 `GET /soap`、graphql 的 `GET /sdl`、`proto/upupt.proto` 不变
- `docs/` 现仅 `API.md`、`SPEC.md`
- 机器契约：`spec/`（OpenAPI、WSDL）与 `proto/`（gRPC）并列；GraphQL SDL 仍运行时生成

## 3. 数据库

| 项 | 结论 |
|---|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- 未改 `config/app.toml`

## 5. 验证

- `cargo clippy --bin upupt -- -D warnings`：PASS
- 本机 `target/debug/upupt`（九面全开）：
  - `GET :3000/v3/openapi.yaml` → 200，`application/yaml`，10461 字节，标题仍是换票网关
  - `GET :3001/openapi.yaml`、`:3002`、`:3003`、`:3008` → 404
  - `GET :3006/soap` → 200，`text/xml`，WSDL
  - `GET :3004/sdl` → 200

## 6. 部署提示

- 必须重建并重启。依赖各面 `GET /openapi.yaml` 做发现的客户端会 404，应改看 `docs/API.md` 或本面真实入口
- 仓库布局：契约在 `spec/`，不再从 `docs/` 读 YAML/WSDL

## 7. 未做 / 明确不做

- 未把 sign / rest / jsonrpc / job 补成完整 OpenAPI
- 未改换票网关、SOAP、gRPC、GraphQL 的对外契约内容
- 未把 `proto/` 并进 `spec/`（`build.rs` 仍从 `proto/` 编译）

## 8. 文件清单

```
spec/oauth_gateway.yaml
spec/upupt.wsdl
docs/API.md
docs/SPEC.md
src/front/oauth_gateway.rs
src/front/soap.rs
src/front/sign_gateway.rs
src/front/rest.rs
src/front/jsonrpc.rs
src/front/job.rs
changelog/20260824-233316-move-contracts-to-spec.md
```

删除：`docs/openapi.yaml`、`docs/upupt.wsdl`、`docs/openapi-sign_gateway.yaml`、`docs/openapi-rest.yaml`、`docs/openapi-jsonrpc.yaml`、`docs/openapi-job.yaml`

---

*记录生成时区：+0800*
