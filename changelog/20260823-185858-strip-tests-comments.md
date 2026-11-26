# Changelog — 2026-08-23 18:58:58 +0800

会话/任务主题：清除全部单元测试与源码注释，保持运行时行为不变

## 1. 背景与目标

- 仓库内散落 `#[cfg(test)]` 模块与注释，与对外 Mock API 无运行时关系
- 目标：删干净测试代码和注释，不改接口、配置、业务逻辑

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `src/logger/mod.rs` | 删除 `tests` 模块与 `init` 文档注释 |
| `src/service/token.rs` | 删除凭证校验 / token 重发单元测试 |
| `src/service/posts.rs` | 删除搜索过滤、排序分页、非法字段测试 |
| `src/service/imgdef.rs` | 删除图像哈希比对测试与临时图辅助函数 |
| `src/service/imgs.rs` | 删除 `generate_png`（仅测试）及 PNG 加载测试 |
| `src/service/photos.rs` | 删除场景选图、缩放编码、维度校验测试 |
| `src/bin/gen_imgs.rs` | 删除内联注释 |
| `scripts/start.sh` | 删除脚本注释 |

### 要点

- 未改 handler / route / middleware / types / 配置
- `gen_imgs` 仍是生成 `imgs/` 的工具二进制，不是单元测试，仅去掉注释
- `docs/`、`changelog/` 历史文档未动

## 3. 数据库

| 项 | 结论 |
|----|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 本项目无数据库 |

## 4. 配置

- 服务 TOML / bootstrap / env / CI：未改
- 运行时动态配置：无

## 5. 验证

- `cargo clippy --bin upupt -- -D warnings`：PASS
- `cargo check --bin gen_imgs`：PASS
- `cargo test --bin upupt`：0 tests，ok
- 未做：HTTP 接口回归（逻辑未改）

## 6. 部署提示

- 无需重建即可继续跑现有二进制；若已部署无需重启
- 本次无运行时行为变化

## 7. 未做 / 明确不做

- 未删除 `src/bin/gen_imgs.rs`（图片库生成工具）
- 未删除 `docs/` 与既有 changelog
- 未改 `gen_imgs` 里既有 clippy pedantic 告警

## 8. 文件清单

```
scripts/start.sh
src/bin/gen_imgs.rs
src/logger/mod.rs
src/service/imgdef.rs
src/service/imgs.rs
src/service/photos.rs
src/service/posts.rs
src/service/token.rs
changelog/20260823-185858-strip-tests-comments.md
```

---

*记录生成时区：+0800*
