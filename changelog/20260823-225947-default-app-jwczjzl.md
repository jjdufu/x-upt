# Changelog — 2026-08-23 22:59:47 +0800

会话/任务主题：内置默认调用方改为 `jwczjzl`，三个 method 全开，并同步文档

## 1. 背景与目标

- 原先预置应用是 `ak_loadtest` / `sk_loadtest_secret_1`
- 改为指定账号 `jwczjzl`，复杂 `client_secret`，开通全部网关 method

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `config/app.toml` | 唯一 `[[apps]]`：`client_id=jwczjzl`，三个 method 全开 |
| `docs/API.md` | 预置凭证表与全部 curl 示例 |
| `docs/SPEC.md` | 配置样例 |

### 要点

- 无运行时接口变更；换票账号从 `ak_loadtest` 换成 `jwczjzl`
- `methods` 仍为 `upupt.posts.search`、`upupt.photos.render`、`upupt.imgdef.compare`

## 3. 数据库

| 项 | 结论 |
|----|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 应用仍来自 TOML |

## 4. 配置

- `config/app.toml` 预置应用替换为 `jwczjzl`
- `client_secret` 含 `#`，TOML 中必须用引号；curl 示例用单引号传递

## 5. 验证

- 未重启进程做 curl（仅改配置与文档）
- 配置需随二进制同级 `config/app.toml` 部署后重启才生效

## 6. 部署提示

- 必须重启 `upupt`；内存中的旧 `ak_loadtest` 票全部失效
- 压测/脚本里的 client 凭证改为新值
- `cargo run` 时仍要从二进制目录读取 `config/`，需同步拷贝 `app.toml`

## 7. 未做 / 明确不做

- 未增加「创建用户」HTTP 接口
- 未保留 `ak_loadtest`

## 8. 文件清单

```
config/app.toml
docs/API.md
docs/SPEC.md
changelog/20260823-225947-default-app-jwczjzl.md
```

---

*记录生成时区：+0800*
