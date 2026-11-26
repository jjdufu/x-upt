# Changelog — 2026-08-23 18:30:00 +0800

会话/任务主题：三条产品接口改为带真实业务 body 的独立 POST，去掉 proxy 兼容层

## 1. 背景与目标

- 原先两条 HTTP 路由：`POST /get/token`（可传 `{}`）、`POST /api/proxy`（用 `endpoint` + 可选 `params` 模拟 GET 查询）
- `/posts`、`/photos` 可不传业务字段；`/imgdef` 缺参仍 200 `matched:false`
- 目标：压测打到真实业务计算。请求体每个字段都改变选数据、算结果或输出字节；不保留无用兼容层

## 2. 代码改动

| 路径 | 变更 |
|------|------|
| `src/route/router.rs` | 注册 `POST /auth/token`、`/api/posts`、`/api/photos`、`/api/imgdef` |
| `src/handler/router.rs` | **删除** proxy 分发器 |
| `src/types/proxy.rs` | **删除** |
| `src/types/{posts,photos,imgdef,token,logger}.rs` | 强类型请求体，`deny_unknown_fields` |
| `src/service/{posts,photos,imgdef,imgs,token}.rs` | 检索/选图缩放转码/aHash 比对/图库解码/Token 绑定用户 |
| `src/handler/*` | 薄封装：延迟、摘要日志 |
| `src/main.rs` / `config/app.toml` | 去掉未使用的 `imgdef.status`；`imgs/` 优先工作目录 |
| `Cargo.toml` | 去掉 `mime_guess` |
| `docs/API.md` / `docs/SPEC.md` | 按当前协议重写，无兼容说明 |

### 要点

- **破坏性协议变更**：`/get/token`、`/api/proxy` 不再存在
- `POST /auth/token`：必填 `username`+`password`；同用户重签会使旧票失效
- `POST /api/posts`：`keyword`（title+body）、`user_id`、`page`、`page_size`、`sort` 全进热路径
- `POST /api/photos`：`scene` 映射 `imgs/img_001.png`–`img_100.png` 区间，再 `resize_exact` + png/jpeg/gif 重编码
- `POST /api/imgdef`：`filename`+`image`+`threshold` 进入 aHash；响应含 `distance`；缺参/坏图 400/404
- 日志只写摘要，不写 Base64 和密码

## 3. 数据库

| 项 | 结论 |
|----|------|
| schema / init-dbs / 迁移 | **无需修改** |
| 说明 | 无数据库；文章目录内存静态，图片用 `imgs/` |

## 4. 配置

- 删除 `config/app.toml` 的 `[imgdef]`
- `[server]` / `[token]` / `[log]` 保持；默认端口 `3001`

## 5. 验证

- `cargo test`：21 passed
- `cargo clippy --bin upupt -- -D warnings`：PASS
- 本机 `http://127.0.0.1:3001` curl：
  - 空 `{}` / 未知字段 → 422；短密码 / `page=0` → 400 JSON
  - `/api/posts` keyword=gRPC,user_id=6 → id 18；user1 page2 size2 id_desc → `[3,2]`
  - `/api/photos` article_cover jpeg 40×30；banner 文件号 71–100；gif 编码成功
  - `/api/imgdef` 用 `imgs/img_001.png` 自比对 distance=0；img_050 vs img_001 distance=32，threshold 0/64 改变 `matched`
  - `/api/proxy`、`/get/token` → 404
- 未跑 `cargo clippy --all-targets -D warnings`（`src/bin/gen_imgs.rs` 原有 pedantic 告警，本次未改）

## 6. 部署提示

- 必须重建并重启 `upupt` 进程
- 压测脚本改为打新 URL 和新 JSON，不能再发 `{}` 或 `/api/proxy`
- 工作目录需有 `imgs/`（或二进制同级 `imgs/`）
- `cargo run` 时需把 `config/app.toml` 放到可执行文件同级 `config/`（`start.sh` 的项目根布局不受影响）

## 7. 未做 / 明确不做

- 未把 `/posts` 改成发帖写入
- 未给请求增加只撑体积、不参与计算的字段
- 未修改 `src/bin/gen_imgs.rs`

## 8. 文件清单

```
Cargo.lock
Cargo.toml
config/app.toml
docs/API.md
docs/SPEC.md
src/handler/imgdef.rs
src/handler/mod.rs
src/handler/photos.rs
src/handler/posts.rs
src/handler/router.rs          (deleted)
src/handler/token.rs
src/logger/mod.rs
src/main.rs
src/route/router.rs
src/service/imgdef.rs          (new)
src/service/imgs.rs            (new)
src/service/mod.rs
src/service/photos.rs          (new)
src/service/posts.rs
src/service/token.rs
src/types/imgdef.rs            (new)
src/types/logger.rs
src/types/mod.rs
src/types/photos.rs            (new)
src/types/posts.rs             (new)
src/types/proxy.rs             (deleted)
src/types/token.rs
changelog/20260823-183000-post-api-real-bodies.md
```

---

*记录生成时区：+0800*
