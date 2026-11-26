# upupt 技术规范

进程拆成业务核与接入面。原来九面进核：核只认 method 与 `biz_content`，调用 `posts` / `photos` / `imgdef`。这九面各自 bind、各自路由、各自 Token 桶。信封六面听 `3010`–`3015`，不进 `core::dispatch`。

```text
[fronts.*] 监听
  └── access_log（request_id / front / port / rpc / in / out）
        └── 该面鉴权
              └── core::dispatch
                    └── posts / photos / imgdef
```

接入面之间不互相调用、不转发、不共用 Router。打到 A 端口的请求只由 A 处理。原来九面常开，三个 method，没有第四个业务。信封六面也常开，假数据不进核。

---

## 启动与配置

配置相对可执行文件目录的 `config/app.toml`。已删除 `[server]`；监听只来自 `[fronts.*]`。若配置仍有 `[server]` 或 `[callback]`，启动失败。未知键（`deny_unknown_fields`）启动失败。

启动顺序：读 toml → 解析 `[[apps]]` / `[methods]` / `[fronts.*]` → 校验 → `logger::init` → 共享 `AppRegistry` 与核 → 对每个 enabled 面建 Token 桶 / nonce 表 / job 表，全部 bind 成功后再 accept。

校验失败则进程退出：

* 至少一个 `enabled=true` 的接入面
* 任一 enabled 面 bind 失败，或两个面同一 `host:port`
* 未知接入面节名、未知键
* TLS `enabled=true` 但缺 `cert`/`key`，或文件不存在
* 某面 `auth` 写了该面不支持的值
* `[[apps]]`：至少一台；`client_id` 唯一且 1–64 字符；`client_secret` 8–128 字节；`methods` 非空且都是三个已知 method

`enabled=false` 不 bind、不建 Token 桶。图片目录：进程当前工作目录下的 `imgs/`（存在则用），否则可执行文件同级的 `imgs/`。

`SIGTERM` / `Ctrl+C` 后 10 秒 graceful shutdown。运维入口是 `scripts/run.sh`。

原来九面默认 `enabled=true`，端口 `3000`–`3008`。TLS 默认关。latency 20–100ms。rest `auth` 含：`oauth_cc,api_key,gettoken,basic,bearer,jwt,digest,hawk,aws_sigv4`。grpc `auth=["api_key"]`，h2c。soap `auth=["basic"]`。sign_gateway 只 HMAC。信封六面端口 `3010`–`3015`，`auth = []`，不走这套 HTTP 鉴权。

能力要么常开，要么代码删除。不为协议变体再留默认关的配置键。

---

## 业务核

`src/core/mod.rs`。核不读 HTTP 头、不签发 Token、不绑端口。

1. method 去空白；空 → `BadRequest`
2. 不是三个已知 method，或全局 `[methods]` 为 false → `UnknownMethod`
3. 不在 `allowed_methods` → `Forbidden`
4. `biz_content` 不是对象 → `BadRequest`
5. 反序列化失败（`deny_unknown_fields`）→ `Unprocessable`（该面「字段不合法」）
6. 调用 `posts::search_posts` / `photos::render_photo` / `imgdef::compare`

posts：id 1–20 冻结；21–1020 由 `posts_gen` 按 id 决定论展开。`user_id=6` 生成帖不含 `gRPC`，探针 `keyword=gRPC` 仍只中 id 18。

photos：同一 `scene+width+height+format` 用 hash 取模选区间内文件，同一入参同一 `filename`。

延迟 sleep 在进入核之前，沿用 `[latency]`。

---

## 身份与票

`[[apps]]` 共用。字段：`client_id`、`client_secret`、可选 `name`、`methods`、可选 `rsa_public_pem`（JWT RS）。

`TokenHub` 按接入面分桶。给 `auth` 含 `oauth_cc` 或 `gettoken` 的 enabled 面建桶，两个换票信封面也建桶。A 签发的票 B 的 `validate` 找不到。API Key 用 `client_secret`，不进 Token 桶。信封票不进 `[[apps]]`，也不跨面。

应用层鉴权枚举不含 `mtls`（TLS 不是免代码鉴权）。

HTTP 鉴权（`authenticate`）用于 rest / jsonrpc / graphql / job / grpc / soap / ws；oauth 面只 oauth_cc。header 与 query 都认。WS query 覆盖同名头。

digest 无凭据时 401，带 MD5 与 SHA-256 两份 `WWW-Authenticate`。`nc` 为 8 位 hex，须大于该 nonce 上次值。hawk：`ts` 窗写死 300s，`id+ts+nonce` 一次性。digest/hawk 的 nonce 表 TTL 600s，不另开配置键。

---

## 流水号

头 `X-Request-Id` 匹配 `[A-Za-z0-9._-]{1,128}` → 原样用、原样回；否则新 UUID。gRPC metadata `x-request-id`。WS 用帧 `id`。不读 `X-Trace-Id`。不为流水号发明第二套 ID。

日志每行：`request_id,ts,level,event,method,path,query,status,code,duration,duration_auth,duration_core,duration_latency,ip,client_id,bytes_in,bytes_out,error,error_kind,in,out,front,port,rpc,job_id,user_agent,referer,content_type,auth,pid,host`。

`event`：`access` 为一次真实入站请求；`job_done` / `job_expired` 为异步任务完成 / 过期的状态行，沿用提交时的 `request_id`、`method`、`path`、`ip`、UA，不是新的入站请求，按入站请求统计时只取 `event=access`。

`level`：`status>=500` 为 `error`，`status>=400` 或有 `error` 为 `warn`，否则 `info`。`query` 去掉密钥类参数（`api_key` / `access_token` / `authorization` / `secret` / `client_secret` / `image` 记 `***`）。`in` / `out` 不写图片 Base64：比图去掉字符串字段 `image`，出图去掉 `image.base64`，文件名和比对结果保留。响应体仍返回 Base64。`code` 为该面业务码（信封数字、REST 短码、JSON-RPC 码、sign `sub_code`、SOAP faultcode）。`auth` 为实际鉴权方式；失败时尽量从请求头推断。`duration_*` 为鉴权 / 核 / 人为延迟的毫秒。GraphQL 的 `in` 含 `query` / `operationName` / `variables`。

---

## 源码布局

```
src/main.rs
src/config.rs
src/core/mod.rs
src/auth/mod.rs             HTTP 鉴权或
src/auth/nonce.rs           nonce 表（一次性 / digest nc）
src/auth/jwt.rs             HS/RS JWT
src/auth/digest.rs          RFC digest
src/auth/hawk.rs            Hawk sha256，窗 300s
src/auth/aws.rs             AWS SigV4
src/auth/sign.rs            HMAC-SHA256
src/front/rest.rs
src/front/oauth_gateway.rs
src/front/sign_gateway.rs
src/front/jsonrpc.rs
src/front/graphql.rs
src/front/grpc.rs
src/front/soap.rs
src/front/ws.rs
src/front/job.rs
src/service/posts.rs        检索目录（1–20 冻结）
src/service/posts_gen.rs    id 21–1020 决定论生成
src/service/photos.rs       决定论选图
src/service/imgdef.rs       aHash 比对
```

没有 callback 模块，不发出站回调。
