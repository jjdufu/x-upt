# Changelog — 2026-10-11 19:56:55 +0800

会话/任务主题：访问日志不再写入图片 Base64

## 1. 背景与目标

- 比图 `in.image` 和出图 `out.image.base64` 会把整段图片 Base64 写进每一行访问日志
- 库图在 `imgs/`，比对成功或失败都不需要靠日志还原像素；响应体仍要返回 Base64
- 目标：只从日志副本去掉图片 Base64，比对、响应和其他日志字段不动

## 2. 代码改动

| 路径 | 变更 |
|---|---|
| `src/logger/mod.rs` | 落盘前去掉字符串字段 `image` 和 `base64`；URL query 的 `image` 记 `***`；GraphQL query 里的 `image` 字符串字面量写成空串 |

### 要点

- 只改 `logger::log` 写出前的副本，不改业务 JSON，也不改 job 内存里的结果
- 出图日志仍留 `image.filename` / `mime` / `size`；比图日志仍留 `filename`、`threshold` 和比对结果
- 九个接入面、extra 行、job 执行行都走同一个落盘函数，所以一起生效

## 3. 数据库

| 项 | 结论 |
|---|---|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库 |

## 4. 配置

- 未改 `config/app.toml`

## 5. 验证

- `cargo test --bin upupt -- image_`：3 passed（`query_hides_image_param`、`log_json_drops_image_payload_only`、`graphql_selection_is_kept`）
- `cargo clippy --bin upupt --all-targets -- -D warnings`：PASS
- 未跑本机九面联调

## 6. 部署提示

- 重建并重启 `upupt`
- 日志采集若依赖 `in.image` 或 `out.image.base64` 的全文，这两处不再有图片字节

## 7. 未做 / 明确不做

- 不改对外响应，不改 aHash 比对
- 不给上传图补 sha256 或摘要字段
- 不上 OpenTelemetry，不做日志切割

## 8. 文件清单

```
src/logger/mod.rs
docs/API.md
docs/SPEC.md
changelog/20261011-195655-omit-image-base64-from-logs.md
```

---

*记录生成时区：+0800*
