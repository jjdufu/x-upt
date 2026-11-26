# Changelog — 2026-08-25 10:17:28 +0800

会话/任务主题：去掉 oauth 的 OpenAPI YAML 与 `GET /v3/openapi.yaml`，不留兼容层

## 1. 背景与目标

- 本项目是 mock 上游，下游只打换票和网关；`spec/oauth_gateway.yaml` 只给机器发现用，业务路径用不到
- 目标：删除该 YAML、拆掉对应路由和辅助函数，文档不再把它当契约

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `spec/oauth_gateway.yaml` | 删除 |
| `src/front/oauth_gateway.rs` | 去掉 `include_str`、`GET /v3/openapi.yaml`、`handle_openapi` |
| `src/front/util.rs` | 删除只给该路径用的 `yaml_response` |
| `docs/API.md` | 机器契约不再列 OpenAPI；oauth 路径表只留 token / gateway |
| `docs/SPEC.md` | 文件清单去掉该 YAML |

### 要点

- **破坏性**：`GET :3000/v3/openapi.yaml` 现为 404 信封（`code` 40401），无重定向、无旧路径
- SOAP `GET /soap`、GraphQL `GET /sdl`、`proto/upupt.proto` 未动

## 3. 数据库

| 项 | 结论 |
|----|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- 未改 `config/app.toml`

## 5. 验证

- `cargo clippy --bin upupt -- -D warnings`：PASS
- 重建并 `./scripts/run.sh restart` 后：
  - `GET :3000/v3/openapi.yaml` → 404 `{"code":40401,"msg":"not found",...}` PASS
  - `POST /v3/oauth/token` 换票 200 PASS
  - `POST /v3/gateway` `upupt.posts.search` keyword=gRPC user_id=6 → id 18 PASS
  - `GET :3006/soap` 仍 200 PASS

## 6. 部署提示

- 必须重建并重启。依赖 `GET /v3/openapi.yaml` 的客户端会 404，改看 `docs/API.md`
- `spec/` 现仅 `upupt.wsdl`

## 7. 未做 / 明确不做

- 未改 SOAP WSDL / GraphQL SDL / gRPC proto
- 未给已删除路径做 301/兼容响应

## 8. 文件清单

```
spec/oauth_gateway.yaml          （删除）
src/front/oauth_gateway.rs
src/front/util.rs
docs/API.md
docs/SPEC.md
changelog/20260825-101728-drop-oauth-openapi.yaml.md
```

---

*记录生成时区：+0800*
