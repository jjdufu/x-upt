# upupt 调用说明

一个进程。原来九面常开，听 `3000`–`3008`，仍是三个 method。runtime 打什么，这九面就验什么、回什么。票不跨面。`[[apps]]` 只给这九面用。

另外六个信封面听 `3010`–`3015`：`envelope_token_plain`、`envelope_token_crypt`、`envelope_sign_plain`、`envelope_sign_crypt`、`envelope_cred_plain`、`envelope_cred_crypt`。它们不进 `core::dispatch`，假数据契约以 `docs/信封改造手册.md` 为准。

预置：`client_id=jwczjzl`，`client_secret=K9mQ2vXp#8Lr4TzYw7nR`，三 method 全开。业务默认 sleep 20–100ms。

机器契约：gRPC `proto/upupt.proto`；SOAP `spec/upupt.wsdl`（`GET :3006/soap`）；GraphQL `GET :3004/sdl`。

---

## 接入面

| 接入面 | 端口 | adapter | 入口 | 鉴权 |
|---|---|---|---|---|
| [oauth_gateway](#oauth_gateway) | 3000 | http_envelope | `POST /v3/gateway`；`POST /v3/oauth/token` | oauth_cc |
| [sign_gateway](#sign_gateway) | 3001 | sign_form | `POST /gateway.do` | 每请求 HMAC |
| [rest](#rest) | 3002 | http_json | 三路径 JSON/form/GET/HEAD；`POST /oauth/token`；`GET /gettoken` | HTTP 鉴权 |
| [jsonrpc](#jsonrpc) | 3003 | jsonrpc | `POST /rpc` | HTTP 鉴权 |
| [graphql](#graphql) | 3004 | graphql | **只** `POST /graphql`；`GET /sdl` 不鉴权 | HTTP 鉴权 |
| [grpc](#grpc) | 3005 | grpc | h2c unary `upupt.v1.Upupt/{SearchPosts,RenderPhoto,CompareImage}` | `x-api-key` 或 `authorization` |
| [soap](#soap) | 3006 | soap_xml | `POST /soap` SOAP 1.1；`GET /soap` WSDL 不鉴权 | basic，可叠加 api_key |
| [ws](#ws) | 3007 | ws_unary | `GET /ws` 升级 | header / query / 首帧 auth |
| [job](#job) | 3008 | job_poll | `POST /jobs`；`GET /jobs/{id}` | HTTP 鉴权 |

原来九面没有第四个 method、没有 echo、没有健康检查当业务、没有任意 path stub。rest 只做三路径 JSON/form/GET/HEAD。GraphQL 只 POST。gRPC 不提供服务发现。sign_gateway 只 HMAC-SHA256。没有 CONNECT / TRACE。没有异步回调 URL。信封六面按画像路径收参，不走这三个 method。联调画像速查：[信封画像一览.md](./信封画像一览.md)（JSON：`信封画像.json`）。

---

## 鉴权

`auth` 是或。同一 listener，不按鉴权拆端口。inject 位置：header 与 query 都认。

| profile | 怎么验 |
|---|---|
| none | 不验 |
| api_key | 头或 query：`X-API-Key` = `client_secret`；或 `Authorization: Bearer <secret>` 且本面桶没有该串 |
| bearer | `Authorization: Bearer <token或secret>` |
| basic | `Authorization: Basic base64(client_id:client_secret)` |
| oauth_cc | 先换票，再 Bearer 本面票 |
| gettoken | 仅 rest 出票；用票同 oauth |
| jwt | 验 mazbri 签的 JWT。HS256/384/512 用 `client_secret`；RS256/384/512 用 app `rsa_public_pem`。iss/sub 能对上 `client_id` 即可 |
| digest | 无凭据 → 401 + `WWW-Authenticate`（MD5 与 SHA-256 都要能挑战）。第二次按 RFC 验；`nc` 为 8 位 hex 且须大于该 nonce 上次值 |
| hawk | 验 `Authorization: Hawk ...`，算法 sha256，凭据 `client_id` / `client_secret`。`ts` 窗 300s；`id+ts+nonce` 一次性 |
| aws_sigv4 | 验 `Authorization` AWS4-HMAC-SHA256。`access_key_id=client_id`，`secret_access_key=client_secret`。region/service 不匹配则 401，不写死拒绝未知 region |

grpc：`x-api-key` 或 `authorization`。soap：Basic。sign：只 HMAC，不走上表。

换票（oauth_cc）：

* `Content-Type: application/x-www-form-urlencoded`
* `grant_type=client_credentials`
* 凭证在 body 的 `client_id`/`client_secret`，或 HTTP Basic（Basic 优先）
* 成功：`{"access_token","token_type":"Bearer","expires_in","scope"}`。jsonpath：`$.access_token` `$.expires_in`
* 头 `Cache-Control: no-store`、`Pragma: no-cache`
* 401 带 `WWW-Authenticate: Basic realm="upupt"`

gettoken：`GET /gettoken?app_id=&secret=`，secret URL 编码。成功 `{access_token,expires_in}`。票进 **rest 本面** 桶。

jwt RS 需要配 `rsa_public_pem`（与 mazbri 工作台 RS 私钥对）。预置公钥：`config/keys/jwczjzl_rsa_pub.pem`。

---

## 三个 method

`deny_unknown_fields`。缺必填 / 类型错 → 该面「字段不合法」。请求字段 snake_case（GraphQL 入参除外）。响应条目里作者是 `userId`。

### `upupt.posts.search`

入：`keyword`(str) `user_id`(u64) `page`(≥1) `page_size`(1–20) `sort` ∈ `id_asc|id_desc|title_asc|title_desc`  
出：`items[]`（`userId,id,title,body`）`total` `page` `page_size`

目录：id `1`–`20` 手写冻结；`21`–`1020` 本地决定论生成（无外网）。`keyword` 空串 = 该 `user_id` 全部帖。`user_id=6` 除 id 18 外 title/body 不含 `gRPC`。`user_id=10` 与 `20` 空词 `total=72`，可翻页；`user_id=99` 无帖。

探针：`keyword=gRPC,user_id=6,page=1,page_size=10,sort=id_desc` → `total=1`，`items[0].id=18`

### `upupt.photos.render`

入：`scene` ∈ `article_cover|avatar|banner`；`width,height` 8–1024；`format` ∈ `png|jpeg|gif`  
出：同入参 + `image.{filename,mime,size,base64}`

决定论：同一 `scene+width+height+format` 必须同一 `filename`（hash 取模选区间内文件）。`article_cover`→`img_001.png`–`040`；`avatar`→`041`–`070`；`banner`→`071`–`100`。

### `upupt.imgdef.compare`

入：`filename`（纯文件名）`image`（标准 Base64，允许 data-URI 前缀）`threshold` 0–64  
出：`matched` `filename` `distance` `threshold`  
`matched = distance <= threshold`

---

## 流水号

头 `X-Request-Id` 匹配 `[A-Za-z0-9._-]{1,128}` → 原样用、原样回响应头；否则新 UUID，回新值。  
gRPC metadata `x-request-id`。WS 用帧 `id`。  
日志每行：`request_id,ts,level,event,method,path,query,status,code,duration,duration_auth,duration_core,duration_latency,ip,client_id,bytes_in,bytes_out,error,error_kind,in,out,front,port,rpc,job_id,user_agent,referer,content_type,auth,pid,host`。
`event`：`access` = 真实入站请求；`job_done` / `job_expired` = 异步任务状态行（沿用提交时的 request_id 等字段，不是新请求）。  
`level` 由 status/`error` 推导。`query` 脱敏，其中 `image` 记 `***`。`code` 是该面业务码。`auth` 是实际鉴权方式。GraphQL `in` 含 query 原文，但原文里的 `image` 字符串字面量写成空串。`in` / `out` 不写图片 Base64；响应体仍返回。  
REST 成功 JSON 可以不塞 `request_id`（对账用头）。oauth 信封、错误 JSON 要带。

不要读 `X-Trace-Id`。mazbri 负责把平台 `request_id` 写成 `X-Request-Id`。

---

## oauth_gateway

`auth=["oauth_cc"]`。

| 方法 | 路径 | 鉴权 |
|---|---|---|
| POST | `/v3/oauth/token` | 换票 |
| POST | `/v3/gateway` | Bearer（本面桶） |

body `{method,biz_content}`。成功 `{code:0,msg:"ok",request_id,data}`。

---

## sign_gateway

`POST /gateway.do` form。只 HMAC-SHA256。

必填：`app_id,method,charset,version,timestamp,nonce,sign_type,sign,biz_content`  
`charset`：**大小写不敏感** 的 utf-8（runtime 默认 `UTF-8`）。  
`version=1.0` `sign_type=HMAC-SHA256`  
待签串：去掉 sign 和空值，键 ASCII 升序，`k=v` 用 `&` 连接。  
`HMAC-SHA256`：`Base64(HMAC-SHA256(key=client_secret, msg=待签串))`  
`timestamp` unix 秒，窗 300s。nonce 一次性。  
成功 HTTP 200，键 `upupt_posts_search_response` 等，带响应 HMAC。

---

## rest

三路径：`/posts/search` `/photos/render` `/imgdef/compare`。

GET：业务字段只从 query 读。  
POST form：`application/x-www-form-urlencoded`，键同 JSON，数字当数字。  
POST JSON：`Content-Type` 含 `json` 即解析（含 `application/vnd.api+json`）。  
imgdef 另认 `multipart/form-data`：字段 `filename` `threshold` 文件字段 `image`。三条路径都要，不要第二 URL。  
HEAD：鉴权通过后 200，无 body，不跑业务。

同一路径 JSON、form、GET query 结果一致。

---

## jsonrpc

`POST /rpc`。`{"jsonrpc":"2.0","method","params":{业务对象},"id"}`。`params` 必须是对象，数组 → `-32602`。成功 `result`，失败 `error.data.request_id`。鉴权失败 `-32001`（HTTP 仍 200）。无 `id` 为 notification → 204 空 body。顶层数组为 batch。`auth` 含 `oauth_cc` 时另有 `POST /oauth/token`。流水号仍以 **HTTP 头** `X-Request-Id` 为准。

---

## graphql

只 `POST /graphql` `{query,variables,operationName?}`。`GET /graphql` → 405。`GET /sdl` 不鉴权。`auth` 含 `oauth_cc` 时另有 `POST /oauth/token`。  
Query：`postsSearch`。Mutation：`photosRender`、`imgdefCompare`。  
变量默认 camelCase（`userId,pageSize`）。枚举 `ID_DESC` / `ARTICLE_COVER` / `PNG` 等。

---

## grpc

plaintext h2（`Http2Only`），`http://` 必须能通。三 RPC：`SearchPosts` `RenderPhoto` `CompareImage`。`RenderPhoto` 填处理后的 `image_bytes`，`image_base64` 空串。proto3 默认值 0 不上线（如 `distance=0`）。

---

## soap

`POST /soap` SOAP 1.1。Body 第一子元必须是 `SearchPosts` / `RenderPhoto` / `CompareImage`。响应条目 `userId`，数组包 `<item>`。

---

## ws

`GET /ws` 升级。帧 `{"id","method","biz_content"}`。回 `{"id","request_id","code","msg","data"}`。二进制帧直接关连接。  
鉴权：升级头 `Authorization`/`X-API-Key`；或 query `access_token`/`api_key`；或首帧 `{"type":"auth",...}`。query 覆盖同名头。`auth` 含 `oauth_cc` 时另有 `POST /oauth/token`。

---

## job

`POST /jobs` `{method,biz_content}` → 202 `{job_id,status:pending}`。  
`GET /jobs/{id}` body 含 `job_id,status,result,error,message`。  
状态字面量：`pending|running|succeeded|failed|expired`。成功必须是 `succeeded`。`auth` 含 `oauth_cc` 时另有 `POST /oauth/token`。
