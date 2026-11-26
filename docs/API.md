# `x-upt` 上游调用与协议参考手册

`x-upt` 是由 `SQLite`（`<data-dir>/x-upt.db`）持久化驱动的确定性多协议模拟上游服务，内嵌 WebUI 管理控制台（默认监听 `0.0.0.0:18768`）。信封接入面的槽位与加解密规范见 [ENVELOPE.md](./ENVELOPE.md)，系统架构、存储模型与运维参数见 [SPEC.md](./SPEC.md)。

---

## 1. 启用流程与前置门控（Precondition Guard）

全新数据目录首次启动时，数据库不预置任何测试图片、帖子记录、应用凭据或信封密钥；3 个核心业务接口与 15 个模拟上游接入面初始全部处于关闭状态（`enabled = false`，不监听模拟端口）。

### 1.1 标准启用顺序

| 步骤 | 操作目标 | 管理控制台 / API 入口 | 结果 |
|---|---|---|---|
| 1 | 启动服务并连接管理端 | `http://127.0.0.1:18768`（`GET /api/v1/status`） | 仅控制台端口监听，可查询各开关就绪度与缺失条件 |
| 2 | 导入业务素材与凭据 | `POST /api/v1/images`、`/api/v1/posts/import`、`/api/v1/apps` | 素材落盘并写入 `x-upt.db`，同步刷新内存快照 |
| 3 | 开启业务子接口开关 | `POST /api/v1/methods/{method}/enable` | 校验通过后解锁对应业务计算核 |
| 4 | 开启协议接入面开关 | `POST /api/v1/fronts/{front}/enable` | `FrontSupervisor` 零重启热绑定目标 `host:port` |
| 5 | 发起上游协议调用 | 目标接入面实时配置的 `host:port` | 按协议契约鉴权并返回确定性业务结果 |

### 1.2 开关前置校验与运行期依赖锁

前置条件未满足时请求开启开关，或启用期间删除最后一条被依赖资源，管理 API 均返回 HTTP `409 Conflict`（JSON 结构 `{"error": "conflict_precondition" | "conflict_in_use", "message": "...", "missing": [...]}`）：

| 门控对象 | 允许开启的前置条件 | 运行期依赖锁定（启用状态下禁止的操作） |
|---|---|---|
| `x_upt.posts.search` | `posts` 表至少存在 `1` 条帖子记录 | 禁止删除最后 `1` 条帖子记录 |
| `x_upt.photos.render` | `article_cover`、`avatar`、`banner` 三个场景下均至少导入 `1` 张图片 | 禁止删除任一场景下的最后 `1` 张图片 |
| `x_upt.imgdef.compare` | `images` 表总记录数 `>= 1` | 禁止删除图片库中的最后 `1` 张图片 |
| 9 个核心业务接入面 | ① `host:port` 未与控制台端口或其他已启用面冲突；② 开启 TLS 时具备有效证书与私钥 PEM；③ 至少已开启 `1` 个业务子接口；④ 除 `auth` 包含 `none` 外，至少存在 `1` 个授权了当前已启用子接口的 `apps` 凭据 | 禁止删除使已启用面失去有效授权凭据的最后一个 `apps` 记录；禁止关闭所有已启用核心面正在依赖的最后一个业务子接口 |
| 6 个信封协议接入面 | ① `host:port` 无冲突且 TLS 配置完整；② 至少配置 `1` 条合法画像（`envelope_token_*` 须同时包含 `issue` 与 `call` 画像）；③ 画像引用的全部密钥/凭据均存在于 `envelope_secrets` 且符合算法字节长度与格式要求 | 禁止删除已启用信封面正在引用的密钥或清空其有效画像 |

---

## 2. 15 个模拟接入面总览

表中端口号为初始化 `x-upt.db` 时的默认建议值，支持通过 WebUI 或 `PUT /api/v1/fronts/{front}` 修改并持久化。

| 接入面名称 | 类别 | 默认端口 | 协议形态 | 业务与元数据路径 | 默认鉴权配置 |
|---|---|---|---|---|---|
| [oauth_gateway](#71-oauth_gateway默认端口-16000) | 核心面 | `16000` | HTTP OAuth 2.0 JSON 网关 | `POST /v3/oauth/token`、`POST /v3/gateway` | `oauth_cc` |
| [sign_gateway](#72-sign_gateway默认端口-16001) | 核心面 | `16001` | HTTP 表单双向签名网关 | `POST /gateway.do` | 固定表单 `HMAC-SHA256` |
| [rest](#73-rest默认端口-16002) | 核心面 | `16002` | REST HTTP/1.1 | `/posts/search`、`/photos/render`、`/imgdef/compare`、`POST /oauth/token`、`GET /gettoken` | `oauth_cc`, `api_key`, `gettoken`, `basic`, `bearer`, `jwt`, `digest`, `hawk`, `aws_sigv4` |
| [jsonrpc](#74-jsonrpc默认端口-16003) | 核心面 | `16003` | JSON-RPC 2.0 | `POST /rpc`、`POST /oauth/token` | `oauth_cc`, `api_key`, `basic`, `bearer`, `jwt`, `digest`, `hawk`, `aws_sigv4` |
| [graphql](#75-graphql默认端口-16004) | 核心面 | `16004` | GraphQL | `POST /graphql`、`GET /sdl`（免鉴权）、`POST /oauth/token` | `oauth_cc`, `api_key`, `basic`, `bearer`, `jwt`, `digest`, `hawk`, `aws_sigv4` |
| [grpc](#76-grpc默认端口-16005) | 核心面 | `16005` | gRPC Unary（h2c / TLS） | `/x_upt.v1.XUpt/{SearchPosts,RenderPhoto,CompareImage}` | `api_key`（支持开启 `none`, `bearer`, `jwt`） |
| [soap](#77-soap默认端口-16006) | 核心面 | `16006` | SOAP 1.1 XML | `POST /soap`、`GET /soap`（免鉴权 WSDL） | `basic`（支持叠加 `none`, `api_key`） |
| [ws](#78-ws默认端口-16007) | 核心面 | `16007` | WebSocket 文本帧 | `GET /ws`、`POST /oauth/token` | `oauth_cc`, `api_key`（支持开启 `none`, `bearer`） |
| [job](#79-job默认端口-16008) | 核心面 | `16008` | 异步任务提交与轮询 | `POST /jobs`、`GET /jobs/{id}`、`POST /oauth/token` | `oauth_cc`, `api_key`, `basic`, `bearer`, `jwt`, `digest`, `hawk`, `aws_sigv4` |
| `envelope_token_plain` | 信封面 | `16010` | 换票 + 明文请求响应 | 由该面 `issue` 与 `call` 画像 `path` 定义 | 画像静态凭据换票 + 本面令牌 |
| `envelope_token_crypt` | 信封面 | `16011` | 换票 + 密文请求响应 | 由该面 `issue` 与 `call` 画像 `path` 定义 | 画像静态凭据换票 + 本面令牌 + 加解密 |
| `envelope_sign_plain` | 信封面 | `16012` | 逐请求验签 + 明文载荷 | 由该面 `call` 画像 `path` 定义 | 画像规范化拼串验签 |
| `envelope_sign_crypt` | 信封面 | `16013` | 逐请求验签 + 密文载荷 | 由该面 `call` 画像 `path` 定义 | 画像验签 + 闭集加解密（含可选 `wrap`） |
| `envelope_cred_plain` | 信封面 | `16014` | 静态凭据 + 明文载荷 | 由该面 `call` 画像 `path` 定义 | 画像多槽位静态凭据比对 |
| `envelope_cred_crypt` | 信封面 | `16015` | 静态凭据 + 密文载荷 | 由该面 `call` 画像 `path` 定义 | 画像静态凭据 + 闭集加解密 |

---

## 3. 请求流水号（`X-Request-Id`）规则

1. **HTTP 接入面**：入站请求头 `X-Request-Id` 匹配正则 `^[A-Za-z0-9._-]{1,128}$` 时，服务端原样采用该值并在响应头 `X-Request-Id` 中原样回显；缺失、为空或包含非法字符/超长时，自动生成新的 `UUIDv4` 字符串并在响应头 `X-Request-Id` 返回。
2. **gRPC 接入面**：读取入站 Metadata 键 `x-request-id`（校验规则同上），并在响应初始 Metadata 的 `x-request-id` 回显。
3. **WebSocket 接入面**：从上行文本 JSON 帧的 `id` 字段提取（字符串或整数转换文本满足 `^[A-Za-z0-9._-]{1,128}$` 时直接作为 `request_id`，否则生成 `UUIDv4`），在响应帧顶层同时返回 `id` 与 `request_id`。
4. **单一追踪标识**：服务仅识别 `X-Request-Id`（以及 gRPC `x-request-id`、WS 帧 `id`），不读取其他追踪头；所有模拟请求的最终 `request_id` 统一写入 31 列 `access_log`。

---

## 4. 同源协议契约下载

无论某个模拟接入面当前是否处于开启状态，管理控制台端口（默认 `18768`）始终提供与运行期代码同源生成的机器契约与在线文档数据：

| 契约类型 | 管理端下载路径 | 内容说明 | 仓库静态真源 |
|---|---|---|---|
| gRPC Proto3 | `GET /api/v1/contracts/grpc.proto` | `package x_upt.v1`，定义 `service XUpt` 与三条 Unary RPC 消息 | [x_upt.proto](../proto/x_upt.proto) |
| gRPC 描述符集 | `GET /api/v1/contracts/grpc.fds` | 编译期生成的二进制 `FileDescriptorSet`（可直接供 `grpcurl -protoset` 加载） | 构建期由 [x_upt.proto](../proto/x_upt.proto) 生成 |
| SOAP 1.1 WSDL | `GET /api/v1/contracts/soap.wsdl` | 自动代入当前数据库中 `soap` 接入面配置的实际监听地址 | [x_upt.wsdl](../spec/x_upt.wsdl) |
| GraphQL SDL | `GET /api/v1/contracts/graphql.sdl` | 运行期动态 Schema 导出的完整 GraphQL 类型与查询/变更定义 | 同源动态构建 |
| 上游 OpenAPI | `GET /api/v1/contracts/upstream-openapi.json` | 汇总所有 HTTP 接入面实时配置端口与路径的 OpenAPI 3.0.3 文档 | 同源动态构建 |
| 在线文档模型 | `GET /api/v1/docs/fronts` | 返回 15 个接入面的结构化端点列表、示例报文与代入当前端口的 CLI 命令 | 同源动态构建 |

---

## 5. 3 个核心确定性业务接口

9 个核心接入面共享同一纯内存快照业务核（`CoreDataset::dispatch`）。入参结构体统一开启严格反序列化（`deny_unknown_fields`）：未知字段、缺失必填项或类型不匹配返回各协议对应的参数校验错误（如 REST `422 unprocessable`、JSON-RPC `-32602`）。除 GraphQL 入参采用 `camelCase` 外，其余协议请求字段统一使用 `snake_case`。

### 5.1 `x_upt.posts.search`（帖子检索）

从已导入的 `posts` 数据集中按作者与关键词过滤、确定性排序并分页返回。

- **入参约束**：

| 字段 | 类型 | 必填 | 取值约束与语义 |
|---|---|---|---|
| `keyword` | `string` | 是 | 匹配 `title` 或 `body` 子串（区分大小写）；空字符串 `""` 匹配该 `user_id` 下的全部帖子 |
| `user_id` | `u64` | 是 | 作者用户 ID 精确过滤 |
| `page` | `u32` | 是 | 页码，从 `1` 开始（`< 1` 返回参数错误） |
| `page_size` | `u32` | 是 | 每页条数，范围 `1`–`20`（超出范围返回参数错误） |
| `sort` | `string` | 是 | 枚举：`id_asc` \| `id_desc` \| `title_asc` \| `title_desc`（按标题排序遇到相同标题时按 `id` 升序决胜） |

- **出参结构**（注意单条帖子作者字段名为驼峰 `userId`）：

```json
{
  "items": [
    {
      "userId": 6,
      "id": 18,
      "title": "gRPC Unary Service Contract",
      "body": "Deterministic upstream response payload."
    }
  ],
  "total": 1,
  "page": 1,
  "page_size": 10
}
```

### 5.2 `x_upt.photos.render`（场景图片渲染）

在指定场景已导入的基准图片集合中按入参哈希确定性选图，精确缩放并编码为目标图片格式。

- **入参约束**：

| 字段 | 类型 | 必填 | 取值约束与语义 |
|---|---|---|---|
| `scene` | `string` | 是 | 场景分类枚举：`article_cover` \| `avatar` \| `banner` |
| `width` | `u32` | 是 | 输出宽度（像素），范围 `8`–`1024` |
| `height` | `u32` | 是 | 输出高度（像素），范围 `8`–`1024` |
| `format` | `string` | 是 | 输出图片编码枚举：`png` \| `jpeg` \| `gif` |

- **确定性选图与渲染算法**：
  1. 当前场景下的已导入图片按 `(filename, id)` 升序稳定排列，记数量为 `N`（`N >= 1`）。
  2. 对 `scene` 字符串、`width`（4 字节小端序）、`height`（4 字节小端序）、`format` 字符串以 `|` 分隔计算 `SHA-256` 摘要，取摘要前 8 字节按无符号 64 位小端整数对 `N` 取模得到下标 `idx`。相同 `(scene, width, height, format)` 始终命中同一张源图。
  3. 使用双线性三角滤波（`Triangle`）精确缩放到 `width × height` 并编码为目标格式（对应 MIME：`image/png`、`image/jpeg`、`image/gif`）。

- **出参结构**（JSON / XML / GraphQL 返回标准 Base64 字符串；gRPC `RenderPhotoResponse` 将原始字节填入 `image_bytes` 字段，同时将 `image_base64` 置为空字符串 `""`）：

```json
{
  "scene": "article_cover",
  "width": 64,
  "height": 64,
  "format": "png",
  "image": {
    "filename": "cover_sample.png",
    "mime": "image/png",
    "size": 612,
    "base64": "iVBORw0KGgoAAAANSUhEUgAA..."
  }
}
```

### 5.3 `x_upt.imgdef.compare`（感知哈希图片比对）

解码调用方上传的图片，计算 64 位均值感知哈希（`aHash`），与图片库中指定 `filename` 的预计算哈希计算汉明距离。

- **入参约束**：

| 字段 | 类型 | 必填 | 取值约束与语义 |
|---|---|---|---|
| `filename` | `string` | 是 | 已导入基准图片的纯文件名（长度 `1`–`128` 字节，禁止包含 `/`、`\`、`..` 或空字节；文件不存在返回 `404` / `not_found`） |
| `image` | `string` | 是 | 待比对图片的标准 Base64 字符串（兼容 `data:image/*;base64,` 前缀）；`rest` 接入面另支持 `multipart/form-data` 直接上传二进制文件字段 `image` |
| `threshold` | `u32` | 是 | 判定匹配的汉明距离上限，范围 `0`–`64` |

- **确定性比对算法**：将图片缩放到 `8 × 8` 灰度图，比较 64 个像素与均值生成 64 位 `aHash`，与目标基准图 `ahash_u64` 异或后统计置位比特数得到汉明距离 `distance`（`0`–`64`），当 `distance <= threshold` 时 `matched = true`。

- **出参结构**：

```json
{
  "matched": true,
  "filename": "cover_sample.png",
  "distance": 0,
  "threshold": 5
}
```

---

## 6. 核心接入面认证体系（10 种模式）

每个核心接入面（`sign_gateway` 除外）通过 `auth` 数组配置允许的鉴权方式，数组内多项之间为**逻辑或（OR）**关系，共用同一监听端口。动态签发的访问令牌（`oauth_cc`、`gettoken`）按接入面物理隔离内存令牌桶，A 接入面签发的令牌在 B 接入面验证一律视为无效。

### 6.1 10 种 `AuthMethod` 校验规则

| 模式标识 | 凭据注入位置 | 校验规则 |
|---|---|---|
| `none` | 无需携带 | 免鉴权放行，允许调用当前全局已开启的全部业务子接口 |
| `api_key` | 请求头 `X-API-Key: <client_secret>`，或 Query `api_key=<client_secret>`（兼容 `x-api-key`），或未命中本面动态令牌桶的 `Authorization: Bearer <client_secret>` | 常量时间比对 `apps.client_secret`，解析出所属 `client_id` 与授权方法集 |
| `bearer` | 请求头 `Authorization: Bearer <token_or_secret>` 或 Query `access_token=<token_or_secret>` | 优先在本面动态令牌桶中查找未过期令牌；未命中时回退常量时间匹配 `apps.client_secret` |
| `basic` | 请求头 `Authorization: Basic <base64(client_id:client_secret)>` | RFC 7617 解码 `client_id:client_secret`，按 `client_id` 查找并常量时间比对密钥 |
| `oauth_cc` | 先调用本面换票端点获取 `access_token`，再通过 `Authorization: Bearer <access_token>` 或 Query `access_token` 调用 | RFC 6749 `client_credentials` 模式；令牌仅在本接入面内存桶有效，默认有效期 30 分钟（1800 秒，由 `expire_minutes` 控制） |
| `gettoken` | 仅 `rest` 面支持：先 `GET /gettoken?app_id=<client_id>&secret=<url_encoded_secret>`，再 `Bearer <access_token>` 调用 | 校验 `app_id` 与 `secret` 后返回 `{"access_token":"...","expires_in":1800}`，令牌存入 `rest` 本面令牌桶 |
| `jwt` | 请求头 `Authorization: Bearer <jwt>` 或 Query `access_token=<jwt>` | 支持对称 `HS256`/`HS384`/`HS512`（密钥取 `client_secret`）与非对称 `RS256`/`RS384`/`RS512`（公钥取 `apps.rsa_public_pem`）；要求 Payload 中 `iss` 或 `sub` 匹配 `client_id`，且 `exp` 未过期 |
| `digest` | 请求头 `Authorization: Digest ...` | RFC 7616 HTTP Digest，`realm="x-upt"`，`qop="auth"`；无凭据请求返回 `401` 并同时附带 `MD5` 与 `SHA-256` 两条 `WWW-Authenticate` 挑战头；二次请求校验 `response` 摘要且同一 `nonce` 下 8 位十六进制 `nc` 必须严格递增（Nonce 有效期 600 秒） |
| `hawk` | 请求头 `Authorization: Hawk id="...", ts="...", nonce="...", mac="...", hash="..."` | 基于 `SHA-256` 与 `client_secret` 校验 `hawk.1.header` 规范串及可选 `hawk.1.payload` 请求体哈希；`ts` 允许正负 300 秒时间窗（`sign_window_seconds`），`(id, ts, nonce)` 一次性防重放 |
| `aws_sigv4` | 请求头 `Authorization: AWS4-HMAC-SHA256 Credential=<client_id>/<YYYYMMDD>/<region>/<service>/aws4_request, SignedHeaders=..., Signature=<hex>`，配合 `Host`、`X-Amz-Date` 与可选 `X-Amz-Content-Sha256` | 以 `"AWS4" + client_secret` 按日期、`region`、`service`、`aws4_request` 四级派生签名密钥，校验规范请求 SHA-256 摘要；`X-Amz-Date` 时间窗 300 秒 |

### 6.2 OAuth 2.0 `client_credentials` 换票规范

- **端点路径**：`oauth_gateway` 为 `POST /v3/oauth/token`；其余开启 `oauth_cc` 的核心 HTTP 面（`rest`、`jsonrpc`、`graphql`、`ws`、`job`）统一为 `POST /oauth/token`。
- **请求头与请求体**：`Content-Type: application/x-www-form-urlencoded`，必填 `grant_type=client_credentials`。客户端凭据支持通过 `Authorization: Basic <base64(client_id:client_secret)>` 请求头传递（优先采用），或在表单中传递 `client_id` 与 `client_secret`；可选 `scope` 字段传空格分隔的方法名子集。
- **成功响应**（HTTP `200`，响应头固定附带 `Cache-Control: no-store` 与 `Pragma: no-cache`）：

```json
{
  "access_token": "tok_7f8a9b0c1d2e3f4a5b6c7d8e",
  "token_type": "Bearer",
  "expires_in": 1800,
  "scope": "x_upt.posts.search x_upt.photos.render x_upt.imgdef.compare"
}
```

- **失败响应**：凭据缺失或错误返回 HTTP `401`（附 `WWW-Authenticate: Basic realm="x-upt"`），`grant_type` 错误返回 HTTP `400`，报文符合 RFC 6749 `{"error": "invalid_client" | "unsupported_grant_type" | "invalid_scope", "error_description": "..."}`。

---

## 7. 9 个核心接入面详细规范与调用示例

以下示例统一使用中立演示应用 `client_id = "demo_client"`、密钥占位 `<CLIENT_SECRET>` 与默认建议端口；实际调用时请替换为在控制台创建的凭据与当前配置的监听端口。

### 7.1 `oauth_gateway`（默认端口 `16000`）

采用 `POST /v3/oauth/token` 换票 + `POST /v3/gateway` JSON 信封调用。

- **请求体**（`POST /v3/gateway`，`Content-Type: application/json`）：`{"method": "<method>", "biz_content": { ... }}`。
- **响应体**：固定包裹 `{code, msg, request_id, data}`：
  - `0`（HTTP `200`）：成功，`msg = "ok"`，`data` 为业务结果对象；
  - `40001`（HTTP `400`）：请求格式或参数范围不合法；
  - `40100`（HTTP `401`）：缺少或格式错误的认证头；
  - `40102`（HTTP `401`）：访问令牌无效或已过期；
  - `40300`（HTTP `403`）：当前应用未获授该方法；
  - `40400`（HTTP `400`）：方法名未知或该业务子接口未开启；
  - `40401`（HTTP `404`）：路径不存在或目标图片文件未找到；
  - `42200`（HTTP `422`）：`biz_content` 字段结构校验失败；
  - `50000`（HTTP `500`）：内部处理异常。

```bash
# 1. 换取 oauth_gateway 本面令牌
TOKEN=$(curl -sS -X POST 'http://127.0.0.1:16000/v3/oauth/token' \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'grant_type=client_credentials' \
  --data-urlencode 'client_id=demo_client' \
  --data-urlencode 'client_secret=<CLIENT_SECRET>' | jq -r '.access_token')

# 2. 调用网关业务接口
curl -sS -X POST 'http://127.0.0.1:16000/v3/gateway' \
  -H "Authorization: Bearer ${TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'X-Request-Id: req-oauth-001' \
  -d '{"method":"x_upt.posts.search","biz_content":{"keyword":"gRPC","user_id":6,"page":1,"page_size":10,"sort":"id_desc"}}'
```

### 7.2 `sign_gateway`（默认端口 `16001`）

仅暴露 `POST /gateway.do`（`Content-Type: application/x-www-form-urlencoded`），固定使用双向 `HMAC-SHA256` 签名，不走通用 HTTP 头鉴权。

- **9 个必填表单字段**（出现任何第 10 个未知字段或任一字段为空均拒绝）：
  - `app_id`：对应 `apps.client_id`（如 `demo_client`）；
  - `method`：`x_upt.posts.search` \| `x_upt.photos.render` \| `x_upt.imgdef.compare`；
  - `charset`：大小写不敏感的 `UTF-8`（如 `UTF-8` 或 `utf-8`）；
  - `version`：固定为 `1.0`；
  - `timestamp`：当前 Unix 秒级整数时间戳字符串，允许偏差窗口 300 秒（`sign_window_seconds`）；
  - `nonce`：长度 `8`–`64` 字节、匹配 `^[A-Za-z0-9._-]{8,64}$` 的一次性防重放随机串（600 秒内禁止重复）；
  - `sign_type`：固定为 `HMAC-SHA256`；
  - `biz_content`：序列化后的业务入参 JSON 字符串；
  - `sign`：请求签名值。
- **待签串与签名计算规则**：
  1. 从请求表单中剔除 `sign` 字段及所有值为空字符串的项；
  2. 剩余字段按字段名 ASCII 字节升序排列，以 `key=value` 形式用 `&` 拼接得到待签串（拼接时使用 URL 解码后的字段原值）；
  3. 计算 `Base64(HMAC-SHA256(key = client_secret, msg = 待签串))` 填入 `sign`。
- **响应格式与错误码**（HTTP 状态码恒为 `200`）：
  - 成功响应：顶层包含 `<method 点号替换为下划线>_response`（即 `x_upt_posts_search_response`、`x_upt_photos_render_response`、`x_upt_imgdef_compare_response`）与服务端对内部响应紧凑 JSON 计算的 `sign`：
    ```json
    {
      "x_upt_posts_search_response": {
        "code": "10000",
        "msg": "Success",
        "request_id": "a1b2c3d4-...",
        "items": [],
        "total": 0,
        "page": 1,
        "page_size": 10
      },
      "sign": "mK8v..."
    }
    ```
  - 失败响应：顶层返回 `{"error_response": {"code": "40004", "msg": "Business Failed", "sub_code": "...", "sub_msg": "...", "request_id": "..."}, "sign": ""}`。其中 `sub_code` 取值包括：`isv.missing-parameter`、`isv.invalid-timestamp`、`isv.invalid-nonce`、`isv.invalid-app-id`、`isv.not-supported-sign-type`、`isv.invalid-signature`、`isv.invalid-method`、`isv.insufficient-isv-permissions`、`isv.invalid-parameter`、`isv.resource-not-exist`、`isp.unknow-error`。

```bash
python3 - <<'PY'
import base64, hashlib, hmac, json, time, urllib.parse, urllib.request, uuid

client_id = "demo_client"
secret = "<CLIENT_SECRET>"
fields = {
    "app_id": client_id,
    "method": "x_upt.posts.search",
    "charset": "UTF-8",
    "version": "1.0",
    "timestamp": str(int(time.time())),
    "nonce": uuid.uuid4().hex[:16],
    "sign_type": "HMAC-SHA256",
    "biz_content": json.dumps({"keyword": "", "user_id": 1, "page": 1, "page_size": 5, "sort": "id_asc"}, separators=(",", ":")),
}
canonical = "&".join(f"{k}={v}" for k, v in sorted(fields.items()) if v)
fields["sign"] = base64.b64encode(hmac.new(secret.encode(), canonical.encode(), hashlib.sha256).digest()).decode()

req = urllib.request.Request(
    "http://127.0.0.1:16001/gateway.do",
    data=urllib.parse.urlencode(fields).encode(),
    headers={"Content-Type": "application/x-www-form-urlencoded"},
    method="POST",
)
with urllib.request.urlopen(req) as resp:
    print(resp.read().decode())
PY
```

### 7.3 `rest`（默认端口 `16002`）

提供三条业务路径：`/posts/search`、`/photos/render`、`/imgdef/compare`，以及可选换票端点 `POST /oauth/token` 与 `GET /gettoken`。

- **多形态入参一致性**（同一路径不同编码返回完全一致的业务 JSON）：
  - `GET`：从 URL Query 读取业务参数（`user_id`、`page`、`page_size`、`width`、`height`、`threshold` 自动转为无符号整数，查询串中的鉴权参数自动剥离不进入业务核）；
  - `POST` JSON：`Content-Type` 包含 `json` 子串（含 `application/json`、`application/vnd.api+json`）时按 JSON 解析（上限 32 MiB）；
  - `POST` Form：`Content-Type: application/x-www-form-urlencoded` 时按表单键值解析并转换数值字段（上限 2 MiB）；
  - `POST` Multipart：`/imgdef/compare` 额外支持 `multipart/form-data`，包含文本字段 `filename`、`threshold` 与文件字段 `image`；
  - `HEAD`：执行完整鉴权校验，通过后直接返回 `200 OK` 空响应体，不执行业务计算。
- **错误响应格式**：`{"error": "<code>", "message": "<desc>", "request_id": "<id>"}`，状态码与 `error` 映射为：`400 bad_request` / `unknown_method`、`401 unauthorized` / `invalid_token`、`403 forbidden`、`404 not_found`、`415 unsupported_media_type`、`422 unprocessable`、`500 internal`。

```bash
# POST JSON + X-API-Key
curl -sS -X POST 'http://127.0.0.1:16002/posts/search' \
  -H 'X-API-Key: <CLIENT_SECRET>' \
  -H 'Content-Type: application/json' \
  -d '{"keyword":"","user_id":1,"page":1,"page_size":5,"sort":"id_asc"}'

# GET Query + HTTP Basic
curl -sS -u 'demo_client:<CLIENT_SECRET>' \
  'http://127.0.0.1:16002/photos/render?scene=article_cover&width=64&height=64&format=png'

# Multipart 上传图片比对
curl -sS -X POST 'http://127.0.0.1:16002/imgdef/compare' \
  -H 'X-API-Key: <CLIENT_SECRET>' \
  -F 'filename=cover_sample.png' \
  -F 'threshold=5' \
  -F 'image=@./cover_sample.png'
```

### 7.4 `jsonrpc`（默认端口 `16003`）

入口为 `POST /rpc`（开启 `oauth_cc` 时另提供 `POST /oauth/token`）。

- **请求规范**：标准 JSON-RPC 2.0 对象 `{"jsonrpc": "2.0", "method": "<method>", "params": { ... }, "id": 1}`：
  - `params` 必须为 JSON 对象，传 JSON 数组返回 `-32602`；
  - 不含 `id` 键的请求视为 Notification（通知），执行后返回 HTTP `204 No Content` 空响应体；
  - 顶层支持 JSON 数组批量调用（Batch），全部元素均为通知时返回 `204 No Content`。
- **响应与错误码**（除 JSON 解析失败 `-32700` 返回 HTTP `400` 外，其余业务与鉴权错误均返回 HTTP `200` 并在 `error` 对象承载）：
  - 成功：`{"jsonrpc": "2.0", "result": { ... }, "id": 1}`；
  - 失败：`{"jsonrpc": "2.0", "error": {"code": -32601, "message": "unknown method", "data": {"request_id": "..."}}, "id": 1}`；
  - 错误码映射：`-32700`（JSON 语法错误）、`-32600`（非法 JSON-RPC 请求）、`-32601`（未知或未开启方法）、`-32602`（参数结构或范围错误）、`-32603`（内部错误）、`-32001`（鉴权失败）、`-32003`（方法未授权）、`-32004`（目标资源不存在）。

```bash
curl -sS -X POST 'http://127.0.0.1:16003/rpc' \
  -H 'X-API-Key: <CLIENT_SECRET>' \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"x_upt.posts.search","params":{"keyword":"","user_id":1,"page":1,"page_size":5,"sort":"id_desc"},"id":1}'
```

### 7.5 `graphql`（默认端口 `16004`）

业务入口仅接受 `POST /graphql`（`GET /graphql` 返回 HTTP `405 Method Not Allowed`）；`GET /sdl` 免鉴权返回当前 Schema SDL 文本。

- **操作与参数映射**（GraphQL 参数与 `pageSize` 字段采用 `camelCase`，枚举采用 `SCREAMING_SNAKE_CASE`）：
  - `Query.postsSearch(keyword: String!, userId: Int!, page: Int!, pageSize: Int!, sort: PostSort!): PostsResult!`（`PostSort`：`ID_ASC` \| `ID_DESC` \| `TITLE_ASC` \| `TITLE_DESC`）；
  - `Mutation.photosRender(scene: PhotoScene!, width: Int!, height: Int!, format: PhotoFormat!): PhotosResult!`（`PhotoScene`：`ARTICLE_COVER` \| `AVATAR` \| `BANNER`；`PhotoFormat`：`PNG` \| `JPEG` \| `GIF`）；
  - `Mutation.imgdefCompare(filename: String!, image: String!, threshold: Int!): ImgdefResult!`。
- **响应扩展**：每次响应顶层固定携带 `"extensions": {"request_id": "..."}`；业务错误写入 `errors[].extensions.code`。

```bash
curl -sS -X POST 'http://127.0.0.1:16004/graphql' \
  -H 'X-API-Key: <CLIENT_SECRET>' \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "query($k:String!,$u:Int!){ postsSearch(keyword:$k,userId:$u,page:1,pageSize:5,sort:ID_DESC){ total pageSize items{ id userId title } } }",
    "variables": {"k": "", "u": 1}
  }'
```

### 7.6 `grpc`（默认端口 `16005`）

默认开启 HTTP/2 明文（`h2c`）与 TLS ALPN `h2`，对应契约文件 [x_upt.proto](../proto/x_upt.proto) 中的 `x_upt.v1.XUpt` 服务（不开启 Server Reflection，客户端通过 `.proto` 或下载的 `grpc.fds` 调用）。

- **3 个 Unary RPC**：`SearchPosts`、`RenderPhoto`、`CompareImage`。
- **Metadata 鉴权与追踪**：通过 `x-api-key: <client_secret>` 或 `authorization: Bearer <token_or_jwt>` 传递凭据；通过 `x-request-id` 传递与回显流水号。
- **二进制字段约定**：`RenderPhotoResponse` 中编码后的图片字节写入 `bytes image_bytes = 8`，`string image_base64 = 9` 恒为空字符串；遵循 Proto3 线格式，数值 `0`（如 `distance = 0`）或 `false` 默认值在序列化字节流中省略。
- **gRPC Status 映射**：鉴权失败返回 `UNAUTHENTICATED`，未授权返回 `PERMISSION_DENIED`，未知或未开启方法返回 `UNIMPLEMENTED`，参数越界返回 `INVALID_ARGUMENT`，图片不存在返回 `NOT_FOUND`。

```bash
# 下载同源描述符集并调用 Unary RPC
curl -sSf 'http://127.0.0.1:18768/api/v1/contracts/grpc.fds' -o /tmp/x_upt.fds

grpcurl -plaintext \
  -protoset /tmp/x_upt.fds \
  -H 'x-api-key: <CLIENT_SECRET>' \
  -H 'x-request-id: req-grpc-001' \
  -d '{"keyword":"","user_id":1,"page":1,"page_size":5,"sort":"id_asc"}' \
  127.0.0.1:16005 x_upt.v1.XUpt/SearchPosts
```

### 7.7 `soap`（默认端口 `16006`）

`GET /soap` 免鉴权返回代入实时地址的 WSDL（真源模板见 [x_upt.wsdl](../spec/x_upt.wsdl)）；`POST /soap` 处理 SOAP 1.1 XML 调用（`Content-Type: text/xml; charset=utf-8`）。

- **报文结构**：`<s:Body>` 下首个子元素名必须为 `SearchPosts`、`RenderPhoto` 或 `CompareImage`，内部子元素名与核心 `snake_case` 入参一致。
- **成功与 Fault 响应**：成功返回 HTTP `200` `<{Op}Response>`，列表元素包裹于 `<item>` 标签内（帖子作者标签为 `<userId>`）；业务或解析错误返回 HTTP `500` SOAP 1.1 `<s:Fault>`（`<faultcode>` 取 `Client` 或 `Server`，`<detail><request_id>...</request_id></detail>` 携带流水号）；HTTP 鉴权失败直接返回 HTTP `401`。

```bash
curl -sS -u 'demo_client:<CLIENT_SECRET>' \
  -X POST 'http://127.0.0.1:16006/soap' \
  -H 'Content-Type: text/xml; charset=utf-8' \
  -d '<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <SearchPosts>
      <keyword></keyword>
      <user_id>1</user_id>
      <page>1</page>
      <page_size>5</page_size>
      <sort>id_asc</sort>
    </SearchPosts>
  </s:Body>
</s:Envelope>'
```

### 7.8 `ws`（默认端口 `16007`）

`GET /ws` 升级为 WebSocket 连接（超过本面 `ws_max_conn` 并发上限时返回 HTTP `503 Service Unavailable`；连接建立后按 `ws_ping_secs` 周期发送 Ping 帧）。

- **三种鉴权传递方式**：
  1. 握手请求头：`Authorization: Bearer <token>` 或 `X-API-Key: <client_secret>`；
  2. 握手 URL Query：`?access_token=<token>` 或 `?api_key=<client_secret>`（同名情况下 Query 覆盖握手请求头）；
  3. 首帧鉴权：握手未携带凭据时，客户端须在连接建立后 `3` 秒内发送首条文本帧 `{"type": "auth", "access_token": "<token>"}` 或 `{"type": "auth", "api_key": "<client_secret>"}`；超时或校验失败立即发送 Close 帧断开连接。
- **业务帧交互**：
  - 请求文本帧：`{"id": "req-1", "method": "x_upt.posts.search", "biz_content": { ... }}`；
  - 响应文本帧：`{"id": "req-1", "request_id": "req-1", "code": 0, "msg": "ok", "data": { ... }}`（错误码与 `oauth_gateway` 数值码一致）；
  - 发送二进制（Binary）帧将记录 `400 bad_request` 日志并立即关闭连接。

```bash
printf '%s\n' '{"id":"ws-001","method":"x_upt.posts.search","biz_content":{"keyword":"","user_id":1,"page":1,"page_size":5,"sort":"id_asc"}}' \
  | websocat -n1 'ws://127.0.0.1:16007/ws?api_key=<CLIENT_SECRET>'
```

### 7.9 `job`（默认端口 `16008`）

采用异步提交与客户端隔离轮询模式（后台并发度由本面 `job_workers` 控制，任务保留时长由全局 `job_expire_seconds` 控制，默认 600 秒）。

- **提交任务**（`POST /jobs`）：
  - 请求体：`{"method": "<method>", "biz_content": { ... }}`；入队前同步校验 `method` 是否开启、当前 `client_id` 是否获授该方法以及 `biz_content` 是否为 JSON 对象，前置校验失败直接返回 HTTP `400` / `403`；
  - 入队成功返回 HTTP `202 Accepted`：`{"job_id": "<uuid>", "status": "pending"}`。
- **轮询结果**（`GET /jobs/{id}`）：
  - 严格校验任务归属，仅提交该任务的同一 `client_id` 可读取，跨客户端或不存在的任务返回 HTTP `404`；
  - 响应体固定包含 5 个字段：`{"job_id": "...", "status": "...", "result": ..., "error": ..., "message": ...}`；
  - `status` 闭集枚举：`pending` \| `running` \| `succeeded` \| `failed` \| `expired`（执行成功固定为 `succeeded`，结果位于 `result`）。

```bash
# 1. 提交异步任务（返回 202 Accepted）
JOB_ID=$(curl -sS -X POST 'http://127.0.0.1:16008/jobs' \
  -H 'X-API-Key: <CLIENT_SECRET>' \
  -H 'Content-Type: application/json' \
  -d '{"method":"x_upt.posts.search","biz_content":{"keyword":"","user_id":1,"page":1,"page_size":5,"sort":"id_asc"}}' | jq -r '.job_id')

# 2. 轮询任务终态与结果
curl -sS "http://127.0.0.1:16008/jobs/${JOB_ID}" \
  -H 'X-API-Key: <CLIENT_SECRET>'
```

---

## 8. 注意事项

- 模拟上游面不提供通用回显（Echo）或通配路由桩；未命中已定义协议路径的请求按各接入面规范返回 `404`（GraphQL `GET /graphql` 返回 `405`）。
- 6 个信封接入面的请求路径、槽位提取与闭集密码学算法由各自持久化的画像驱动，完整定义见 [ENVELOPE.md](./ENVELOPE.md)。
