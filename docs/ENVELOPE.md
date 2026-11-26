# `x-upt` 信封协议与画像规范（Envelope Specification）

`x-upt` 提供 6 个独立的信封协议模拟接入面（默认建议端口 `16010`–`16015`），用于确定性验证多槽位参数提取、规范化待签串拼装、国密与国际闭集签名/加解密、数字信封密钥封装（Key Wrapping）及双通道错误回包。核心业务面规范见 [API.md](./API.md)，底层存储与日志规范见 [SPEC.md](./SPEC.md)。

---

## 1. 6 个信封接入面与前置门控

信封接入面与 3 个核心业务接口完全解耦：不经过 `CoreDataset::dispatch`，不共用核心面的 `apps` 凭据表或 OAuth 令牌桶，而是完全由 `SQLite` 中持久化的信封密钥（`envelope_secrets`）与信封画像（`envelope_profiles`）驱动。

### 1.1 接入面分类矩阵

| 接入面名称 | 默认端口 | 身份校验机制 | 载荷形态 | 必需画像类型（`kind`） | `access_log` 中 `auth` 标识 |
|---|---|---|---|---|---|
| `envelope_token_plain` | `16010` | 静态凭据换票 + 本面令牌校验 | 明文 | 同时具备 `issue` 与 `call` | `envelope_token` |
| `envelope_token_crypt` | `16011` | 静态凭据换票 + 本面令牌校验 | 加密 | 同时具备 `issue` 与 `call` | `envelope_token` |
| `envelope_sign_plain` | `16012` | 逐请求规范化待签串验签 | 明文 | 至少 1 条 `call` | `envelope_sign` |
| `envelope_sign_crypt` | `16013` | 逐请求验签 + 载荷加解密 | 加密 | 至少 1 条 `call` | `envelope_sign` |
| `envelope_cred_plain` | `16014` | 多槽位静态凭据常量时间比对 | 明文 | 至少 1 条 `call` | `envelope_cred` |
| `envelope_cred_crypt` | `16015` | 静态凭据比对 + 载荷加解密 | 加密 | 至少 1 条 `call` | `envelope_cred` |

### 1.2 开启前置条件与运行期保护

- **开启校验（不满足返回 `409 Conflict`）**：
  1. 监听 `host:port` 未与管理控制台端口或其他已启用接入面冲突，且开启 TLS 时具备有效服务端证书与私钥 PEM；
  2. 当前信封面下至少配置 `1` 条有效画像，同一接入面内 `(methods, path)` 组合唯一无冲突；`envelope_token_plain` 与 `envelope_token_crypt` 必须同时配置至少 `1` 条 `kind = "issue"` 画像与 `1` 条 `kind = "call"` 画像（非换票面禁止配置 `kind = "issue"`）；
  3. 画像引用的全部密钥与凭据（`secrets`、`key`、`sign_key`、`private_key`、`public_key`）均已录入 `envelope_secrets`，且通过算法字节长度与公私钥格式校验。
- **令牌隔离与依赖锁**：两个换票信封面各自维护独立的内存 `EnvelopeTokenStore`，令牌互不跨面通用；当信封面处于 `enabled = true` 状态时，删除其画像正在引用的任何密钥将返回 `409 Conflict`。

---

## 2. 信封画像（`EnvelopeProfileSpec`）配置结构

每条画像定义一个 HTTP 入口路径的参数提取、验签、加解密与回包契约：

| 字段名 | 类型 | 必填 / 条件必填 | 取值范围与说明 |
|---|---|---|---|
| `name` | `string` | 必填 | 本接入面内唯一的画像名称 |
| `kind` | `string` | 必填 | `issue`（仅限两个 `envelope_token_*` 换票面）\| `call`（业务调用画像） |
| `methods` | `string[]` | 必填 | 允许的 HTTP 方法子集：`["POST"]`、`["GET"]` 或 `["GET", "POST"]` |
| `path` | `string` | 必填 | 以 `/` 开头的精确请求路径（如 `"/issue"`、`"/call"`） |
| `body_kind` | `string` | 必填 | 请求体结构：`json` \| `form` \| `raw` \| `empty` |
| `body_encoding` | `string` | 必填 | 外层请求体编码：`identity`（原样字节）\| `base64`（先整包 Base64 解码） |
| `slot_priority` | `string[]` | 可选 | 4 槽位合并优先级数组，默认 `["json", "form", "query", "header"]`（靠前槽位优先级更高） |
| `fields` | `object[]` | 可选 | 显式槽位字段声明列表，每项为 `{"name": "...", "slot": "...", "role": "..."}` |
| `biz_rest` | `bool` | 可选 | 默认 `false`；为 `true` 时允许未在 `fields` 声明的 `json`/`form`/`query` 字段自动归入 `biz` |
| `token_prefix` | `string` | `header` 含 `token` 时必填 | 令牌前缀（大小写敏感，如 `"Bearer "`） |
| `token_response_field` | `string` | `kind="issue"` 必填 | 换票响应 JSON 中存放令牌字符串的字段名（如 `"access_token"`） |
| `secrets` | `object` | 静态凭据面必填 | `{"<auth字段名>": "<envelope_secrets中的密钥名>"}` 映射表 |
| `canon` | `string` | 验签画像必填 | 规范化待签串拼装规则：`sorted_kv` \| `concat` \| `raw` |
| `canon_skip` | `string[]` | 可选 | 拼装待签串时跳过的字段名列表（如 `["sign"]`） |
| `canon_sep` | `string` | 可选 | 待签串字段间及前后缀密钥分隔符（默认 `""`，常用 `"&"`） |
| `canon_prefix_key` | `bool` | 可选 | 默认 `false`；为 `true` 时在待签串头部拼接密钥字节 |
| `canon_suffix_key` | `bool` | 可选 | 默认 `false`；为 `true` 时在待签串尾部拼接密钥字节 |
| `canon_fields` | `string[]` | `concat`/`raw` 使用 | 指定参与 `concat` 拼接的有序字段名列表，或 `raw` 模式下读取的单个字段名 |
| `sign_alg` | `string` | 签名画像必填 | `md5` \| `hmac_sha256` \| `sm3` \| `hmac_sm3` \| `rsa_sha256` \| `sm2_sign` |
| `sign_encoding` | `string` | 可选 | 签名文本编码：`hex`（默认）\| `base64` |
| `hex_case` | `string` | 任一编码为 `hex` 时必填 | 十六进制字母大小写：`lower` \| `upper`（验签与密文解码均严格校验大小写） |
| `crypt_alg` | `string` | 加密画像必填 | 闭集 10 种加解密算法标识（见第 5.2 节） |
| `cipher_scope` | `string` | 可选 | 密文承载范围：`fields`（默认，字段级密文）\| `body`（配合 `body_kind="raw"` 整包密文） |
| `cipher_fields` | `string[]` | `fields` 加密可选 | 指定需解密的字段名列表；未填时自动取 `fields` 中所有 `role="cipher"` 的字段 |
| `crypt_encoding` | `string` | 可选 | 密文与封装密钥编码：`base64`（默认）\| `hex` |
| `padding` | `string` | `aes_cfb` 必填 | `none` \| `pkcs5` |
| `segment_bits` | `usize` | `aes_cfb` 必填 | CFB 反馈位宽，范围 `8`–`128` 且必须为 `8` 的整数倍（如 `8`、`128`） |
| `iv_len` | `usize` | `aes_gcm` 必填 | GCM Nonce 长度：`12` 或 `16`（字节） |
| `iv_from` | `string` | 需 IV 的对称算法必填 | IV 获取方式：`key` \| `random_prefix` \| `field` |
| `tag_bits` | `usize` | `aes_gcm` 必填 | GCM 认证标签位长，固定为 `128` |
| `user_id` | `string` | `sm2_sign` 必填 | SM2 ZA 杂凑用户标识字符串（如 `"1234567812345678"`） |
| `sm2_mode` | `string` | 可选 | SM2 密文排列顺序：`c1c3c2`（默认）\| `c1c2c3` |
| `key` / `sign_key` | `string` | 按算法必填 | 引用 `envelope_secrets` 中的对称加解密密钥名 / HMAC 或前后缀签名密钥名 |
| `private_key` / `public_key` | `string` | 非对称算法必填 | 引用 `envelope_secrets` 中的 RSA / SM2 私钥名与公钥名 |
| `wrap` | `string` | 可选 | 数字信封会话密钥封装算法：`rsa_pkcs1_public` \| `sm2_encrypt` |
| `order` | `string` | 同时配置签名与加密时必填 | `sign_then_encrypt` \| `encrypt_then_sign`（未配置 `crypt_alg` 时禁止设置） |
| `fail_in` | `string` | 必填 | 错误回包通道：`http` \| `envelope` |
| `response_body_kind` | `string` | 可选 | 响应体形态覆盖，默认继承 `body_kind` |
| `response_encoding` | `string` | 可选 | 响应体外层编码覆盖，默认继承 `body_encoding` |
| `response_crypt_alg` | `string` | 可选 | 响应加密算法覆盖；设为 `"none"` 时请求解密但响应以明文 JSON 返回 |

---

## 3. 请求体限制、4 类槽位与词法 JSON 保真

### 3.1 1 MiB 载荷硬上限与外层解码

1. **1 MiB 大小限制**：入站原始 HTTP 请求体超过 `1,048,576` 字节（1 MiB）时，直接短路返回 HTTP `413 Payload Too Large`，响应体固定为明文 JSON `{"ok":false,"reason":"bad_shape","hit":false,"fields":{},"digest":""}`。
2. **外层编码（`body_encoding`）**：
   - `identity`：直接将原始请求体字节送入 `body_kind` 解析器；
   - `base64`：先校验请求体为合法 UTF-8，去除首尾空白后（空串视为空字节）按标准 Base64 解码，解码失败判为 `bad_shape`。
3. **请求体形态（`body_kind`）与 `Content-Type` 校验**：
   - `json`：要求请求头 `Content-Type` 媒体类型为 `application/json`（若附带 `charset` 参数，必须为大小写不敏感的 `utf-8`；如 `charset=gbk` 直接判为 `bad_shape`），解码后字节必须为非空且合法的顶层 JSON 对象；
   - `form`：要求 `Content-Type` 为 `application/x-www-form-urlencoded`（`charset` 若存在须为 `utf-8`），所有表单值统一保留为字符串（例如 `n=1` 解析为字符串 `"1"`，不自动转为数字）；
   - `raw`：不校验键值结构，配合 `cipher_scope = "body"` 将解码后的完整字节流作为待解密密文；
   - `empty`：要求解码后的请求体长度为 `0` 字节，非空即判为 `bad_shape`。

### 3.2 4 类参数槽位（`slot`）与 8 种角色（`role`）

- **4 类槽位**：
  - `header`：HTTP 请求头，按 ASCII 小写归一化匹配（同名多头取首个）；未在 `fields` 中显式声明的请求头（如 `X-Request-Id`、`User-Agent`）一律忽略，绝不混入业务字段或待签串；
  - `query`：URL 查询字符串（`application/x-www-form-urlencoded` 规则解码，值均为字符串）；
  - `form`：表单请求体键值；
  - `json`：顶层 JSON 对象键值。
- **8 种字段角色**：
  - `auth`：静态身份凭据字段（与 `secrets` 映射表逐一常量时间比对），或 `iv_from = "field"` 时名为 `iv` 的初始向量字段；
  - `token`：换票面调用时携带的访问令牌字段；
  - `sign`：请求携带的签名字符串字段；
  - `cipher`：承载加密业务数据的密文字段；
  - `wrap_key`：承载经非对称公钥加密后的临时对称会话密钥字段；
  - `timestamp`：Unix 秒级整数时间戳字段（存在时自动按全局 `sign_window_seconds` 校验时间窗，默认允许正负 300 秒偏差，超窗判为 `bad_sign`）；
  - `nonce`：防重放随机串字段（存在时不能为空，并在本面 `NonceStore` 内按 `sign_nonce_ttl_seconds` 防重放，默认 600 秒，重复判为 `bad_sign`）；
  - `biz`：参与确定性 `digest` 计算与回显的业务数据字段。

### 3.3 `biz_rest` 与 `slot_priority` 合并规则

1. `fields` 数组中声明的每一个 `{name, slot, role}` 必须在对应槽位中存在，任一缺失立即判为 `bad_shape`。
2. 当 `biz_rest = false`（默认）时，`json`、`form`、`query` 三个槽位中只要出现任何未在 `fields` 对应槽位声明的额外字段，立即拒绝并判为 `bad_shape`。
3. 当 `biz_rest = true` 时，除已在 `fields` 任一槽位命名的控制字段外，`json`、`form`、`query` 中的其余字段全部自动归入 `biz` 角色。若多个槽位出现同名未声明字段，按 `slot_priority`（默认 `["json", "form", "query", "header"]`）从低优先级向高优先级依次写入，最终保留最高优先级槽位的值（`header` 槽位永远不参与 `biz_rest` 自动收集）。
4. 若请求包含加密载荷，解密得到的 UTF-8 JSON 对象顶层键值在解密阶段合并写入 `biz` 表（同名键覆盖外层同名 `biz` 值），并在生成最终响应前剥离所有非 `biz` 角色的显式控制字段名。

### 3.4 JSON 词法保真（Lexical JSON Preservation）

信封引擎内置词法保真 JSON 解析器，不将 JSON 数字转换为浮点数：
- 数值字面量原样保留原始字符序列（例如 `1.50` 保留为 `1.50`，不会变成 `1.5`）；
- 字符串 `"1"`、数值 `1`、布尔值 `true`/`false`、`null`、数组元素顺序及嵌套对象严格保持各自类型；
- 计算紧凑规范化 JSON 时，对象键按 UTF-8 字节字典序升序排列，冒号与逗号后无空格，保证任意客户端计算待签串与响应 `digest` 时零精度漂移。

---

## 4. 3 种规范化待签串规则（`canon`）

验签与响应加签共用同一待签串构造器（`canon`）：

| `canon` 模式 | 主体字节构造规则 |
|---|---|
| `sorted_kv` | 收集当前槽位表中的全部 `biz` 字段以及 `auth`、`cipher`、`timestamp`、`nonce`、`token`、`wrap_key` 角色字段（不含 `sign` 角色），过滤掉 `canon_skip` 列表中的字段名；按字段名 ASCII 字节升序排序并按名去重；每个字段格式化为 `key=value`（标量取词法原值，数组或对象取键升序紧凑 JSON），以 `canon_sep` 连接为 UTF-8 字节串 |
| `concat` | 严格按照 `canon_fields` 数组给定的字段名顺序依次查找字段文本值（跳过 `canon_skip` 中的字段，未找到的字段取空串 `""`），直接以 `canon_sep` 连接为 UTF-8 字节串，不带 `key=` 前缀 |
| `raw` | 在 `order = "encrypt_then_sign"` 解密后验签时，直接取首个解密出的原始明文字节流；其余场景取 `canon_fields` 第一个字段名对应的原始 UTF-8 字节 |

**前后缀密钥拼接**：拼接密钥优先读取 `sign_key`，未配置 `sign_key` 时读取 `key`。开启 `canon_prefix_key = true` 时在主体前拼接 `key_bytes + canon_sep`；开启 `canon_suffix_key = true` 时在主体后拼接 `canon_sep + key_bytes`（当 `canon_sep` 为空串时不插入分隔符）。

---

## 5. 闭集密码学算法与密钥约束

信封密钥统一存储在 `SQLite` 的 `envelope_secrets` 表中，按编码类型（`SecretKind`）分为三类：`text`（UTF-8 凭据字符串）、`bytes_hex`（十六进制表示的原始二进制密钥字节）、`pem`（PEM 格式公私钥）。

### 5.1 闭集签名算法（6 种 `sign_alg`）

| `sign_alg` | 密钥引用与格式要求 | 摘要 / 签名规则 |
|---|---|---|
| `md5` | 可选配合 `sign_key` 或 `key` 做前后缀拼串 | 对待签串计算 16 字节 MD5 摘要 |
| `hmac_sha256` | 必填 `sign_key`（任意非空字节） | 计算 32 字节 `HMAC-SHA256(sign_key, material)` |
| `sm3` | 无需密钥（或配合前后缀拼串） | 计算 32 字节国密 `SM3` 杂凑值 |
| `hmac_sm3` | 必填 `sign_key`（块长 64 字节） | 基于国密 `SM3` 计算 32 字节 `HMAC-SM3` |
| `rsa_sha256` | 验签必填 `public_key`（SPKI PEM），响应加签必填 `private_key`（PKCS#8 PEM） | `RSASSA-PKCS1-v1_5` with `SHA-256` |
| `sm2_sign` | 必填 `user_id`；验签必填 `public_key`（65 字节未压缩点 `04\|\|X\|\|Y`），响应加签同时必填 `private_key`（32 字节）与 `public_key` | 国密 `SM2` 椭圆曲线签名（含 `user_id` 与公钥计算的 `ZA` 预处理），签名值为 ASN.1 DER 编码 |

### 5.2 闭集加解密算法（10 种 `crypt_alg`）

| `crypt_alg` | 密钥字段与字节长度 | IV 长度与必填项 | 填充与分组细节 |
|---|---|---|---|
| `aes_ecb_pkcs5` | `key`：`16`、`24` 或 `32` 字节 | 无 IV | AES-128/192/256 ECB，16 字节块 PKCS#7 填充 |
| `aes_cfb` | `key`：`16`、`24` 或 `32` 字节 | 16 字节；必填 `iv_from`、`segment_bits`（`8`..=`128` 且整除 `8`）、`padding`（`none` \| `pkcs5`） | 按 `segment_bits / 8` 字节步进移位寄存器反馈 |
| `aes_gcm` | `key`：`16`、`24` 或 `32` 字节 | `iv_len`：`12` 或 `16` 字节；必填 `iv_from`、`tag_bits = 128` | AEAD（AAD 为空 `b""`），密文末尾紧跟 16 字节 Tag |
| `des_cbc_pkcs5` | `key`：固定 `8` 字节 | 8 字节；必填 `iv_from` | DES-CBC，8 字节块 PKCS#7 填充 |
| `sm4_ecb_pkcs7` | `key`：固定 `16` 字节 | 无 IV | 国密 SM4-ECB，16 字节块 PKCS#7 填充 |
| `sm4_cbc_pkcs7` | `key`：固定 `16` 字节 | 16 字节；必填 `iv_from` | 国密 SM4-CBC，16 字节块 PKCS#7 填充 |
| `sm4_ecb_nopadding` | `key`：固定 `16` 字节 | 无 IV | 国密 SM4-ECB 无填充，明文与密文长度须为 `16` 的整数倍 |
| `rsa_pkcs1_public` | 入站解密用 `private_key`，出站加密用 `public_key`（PEM） | 无 IV | RSA PKCS#1 v1.5 Type 2 公钥加密 / 私钥解密 |
| `rsa_pkcs1_private` | 入站解封用 `public_key`，出站封装用 `private_key`（PEM） | 无 IV | RSA PKCS#1 v1.5 Type 1 私钥块运算 / 公钥解块 |
| `sm2_encrypt` | 入站解密用 `private_key`（32 字节），出站加密用 `public_key`（65 字节） | 无 IV；可选 `sm2_mode`（默认 `c1c3c2`，可选 `c1c2c3`） | 国密 SM2 公钥加密，密文总长 `>= 97` 字节（65 字节 C1 + 32 字节 C3 + 明文等长 C2） |

### 5.3 IV 来源（`iv_from`）规则

- `key`：直接截取当前对称密钥的前 `N` 字节作为固定 IV（`des_cbc_pkcs5` 取前 8 字节，`aes_cfb` 与 `sm4_cbc_pkcs7` 取前 16 字节，`aes_gcm` 取前 `iv_len` 字节）。
- `random_prefix`：密文编码解码后的原始二进制格式固定为 `IV(N 字节) || Ciphertext`；入站解密先切分前 `N` 字节作为 IV，出站响应加密自动生成密码学安全随机 `N` 字节前缀拼在密文之前再做 `crypt_encoding` 编码。
- `field`：入站解密从 `role = "auth"` 且 `name = "iv"` 的请求槽位读取字符串，按 `crypt_encoding` 解码得到 `N` 字节 IV；出站加密生成随机 IV。

---

## 6. 密钥封装（`wrap`）与签名加密顺序（`order`）

### 6.1 数字信封密钥封装（`wrap`）

当画像配置 `wrap = "rsa_pkcs1_public"` 或 `wrap = "sm2_encrypt"` 时：
1. **入站解封**：客户端生成临时对称密钥加密业务 JSON，再用服务端公钥将该临时密钥加密并按 `crypt_encoding` 编码放入 `role = "wrap_key"` 的请求字段；服务端先用 `private_key` 解出临时对称密钥，再按 `crypt_alg` 解密业务密文。
2. **出站封装**：服务端使用本画像配置的对称 `key` 加密响应 JSON，同时用 `public_key` 封装该对称密钥，在加密响应 JSON 中额外输出 `"wrap_key": "<encoded_wrapped_key>"`。

### 6.2 签名与加解密执行顺序（`order`）

| `order` 取值 | 入站请求处理顺序 | 出站响应封装顺序 |
|---|---|---|
| `sign_then_encrypt` | 先对请求外层槽位（含密文字段编码串）执行验签、验票与验凭据；全部通过后再解密密文提取 `biz` | 先将响应对象加密并编码为密文字符串，再对该编码后的密文字符串字节计算响应签名 |
| `encrypt_then_sign` | 先解封/解密请求密文并将明文 JSON 合并进 `biz` 槽位，再基于解密后的明文执行验签、验票与验凭据 | 先计算加密前响应明文紧凑 JSON 字节的签名，同时将响应明文加密编码后与签名一并装入外层 JSON |

---

## 7. 错误通道（`fail_in`）与确定性响应契约

### 7.1 换票响应（`kind = "issue"`）

静态凭据校验通过后，在本接入面独立令牌桶生成有效期 1800 秒（默认 30 分钟）的随机令牌，返回 HTTP `200`（`Content-Type: application/json`）：

```json
{"access_token":"...","expires_in":1800}
```

（令牌键名由 `token_response_field` 决定。）

### 7.2 业务调用确定性契约（`kind = "call"`）

所有 `call` 画像的逻辑输出固定为包含 5 个有序键的 JSON 对象 `{ok, reason, hit, fields, digest}`：

```json
{
  "ok": true,
  "reason": "",
  "hit": true,
  "fields": {"amount": 1.50, "city": "hz"},
  "digest": "eb7b62..."
}
```

| 字段 | 类型 | 成功时取值 | 失败时取值 |
|---|---|---|---|
| `ok` | `bool` | `true` | `false` |
| `reason` | `string` | 空字符串 `""` | 闭集错误原因码之一（见下表） |
| `hit` | `bool` | `fields` 非空时为 `true`，`fields` 为空对象 `{}` 时为 `false` | `false` |
| `fields` | `object` | 剥离控制字段后的词法保真业务键值对象 | 空对象 `{}` |
| `digest` | `string` | 对 `fields` 按键名 UTF-8 字节升序、紧凑无空白序列化后的 `SHA-256` 小写十六进制摘要（空 `{}` 为 `SHA-256("{}")`） | 空字符串 `""` |

当响应需要加密时（即配置了 `crypt_alg` 且未设 `response_crypt_alg = "none"`），上述 `{ok, reason, hit, fields, digest}` 紧凑 JSON 字符串被加密编码后放入外层 JSON 的首个密文字段名（如 `{"payload": "<cipher>", "sign": "<sig>"}`）。

### 7.3 5 种闭集失败原因（`reason`）与 `fail_in` 双通道行为

| `reason` 枚举 | 触发条件 | `fail_in = "http"` 状态码 | `fail_in = "envelope"` 行为 |
|---|---|---|---|
| `bad_token` | 令牌缺失、`token_prefix` 不匹配（大小写敏感）或不属于本面活跃令牌 | HTTP `401` 明文 JSON | 未配置 `wrap`（或 `encrypt_then_sign` 已解密）时返回 HTTP `200` 加密失败信封；否则回退 HTTP `401` |
| `bad_sign` | 签名不匹配、`timestamp` 超出时间窗或 `nonce` 为空/重放 | HTTP `401` 明文 JSON | 同上（可封装时返回 HTTP `200` 加密失败信封，否则回退 HTTP `401`） |
| `bad_credential` | `secrets` 中任一静态凭据常量时间比对失败 | HTTP `401` 明文 JSON | `issue` 画像固定返回 HTTP `401`；`call` 画像可封装时返回 HTTP `200` 加密失败信封 |
| `bad_decrypt` | 密文编码非法、填充错误、密钥解封失败或解密结果非合法 JSON 对象 | HTTP `400` 明文 JSON | 解密阶段失败无法建立安全通道，固定回退返回 HTTP `400` 明文失败 JSON |
| `bad_shape` | Content-Type/字符集错误、缺必填槽位、多余未声明字段或请求体超 1 MiB | 超 1 MiB 为 `413`，其余为 `400` 明文 JSON | 槽位解析阶段即失败，固定回退返回 HTTP `400` / `413` 明文失败 JSON |

> [!NOTE]
> 请求路径未命中本面任何画像 `path` 时返回 HTTP `404`（`text/plain` `not found`）；命中 `path` 但请求 HTTP 方法不在画像 `methods` 列表内时返回 HTTP `405`（`text/plain` `method not allowed`）。

---

## 8. 配置与调用示例

### 8.1 `envelope_sign_plain`（MD5 排序键值对 + 尾部拼接密钥）

- **画像定义（`PUT /api/v1/envelopes/envelope_sign_plain/profiles` 元素示例）**：

```json
{
  "name": "md5_sorted_call",
  "kind": "call",
  "methods": ["POST"],
  "path": "/call",
  "body_kind": "json",
  "body_encoding": "identity",
  "fields": [
    {"name": "sign", "slot": "json", "role": "sign"}
  ],
  "biz_rest": true,
  "canon": "sorted_kv",
  "canon_skip": ["sign"],
  "canon_sep": "&",
  "canon_suffix_key": true,
  "sign_alg": "md5",
  "sign_encoding": "hex",
  "hex_case": "lower",
  "sign_key": "demo_md5_secret",
  "fail_in": "http"
}
```

- **客户端计算签名与调用**（假设 `demo_md5_secret` 为文本密钥 `demo-sign-key-01`）：

```bash
# 待签串: "city=hz&demo-sign-key-01"
SIGN=$(printf '%s' 'city=hz&demo-sign-key-01' | md5sum | awk '{print $1}')

curl -sS -X POST 'http://127.0.0.1:16012/call' \
  -H 'Content-Type: application/json' \
  -d "{\"city\":\"hz\",\"sign\":\"${SIGN}\"}"
```

### 8.2 `envelope_sign_crypt`（SM4-CBC 随机前缀 IV + HMAC-SM3 内层明文签名 + 信封内报错）

- **画像定义（`PUT /api/v1/envelopes/envelope_sign_crypt/profiles` 元素示例）**：

```json
{
  "name": "sm4_cbc_hmac_sm3",
  "kind": "call",
  "methods": ["POST"],
  "path": "/sm4",
  "body_kind": "json",
  "body_encoding": "identity",
  "fields": [
    {"name": "payload", "slot": "json", "role": "cipher"},
    {"name": "sign", "slot": "json", "role": "sign"}
  ],
  "biz_rest": false,
  "canon": "raw",
  "sign_alg": "hmac_sm3",
  "sign_encoding": "hex",
  "hex_case": "lower",
  "crypt_alg": "sm4_cbc_pkcs7",
  "cipher_scope": "fields",
  "cipher_fields": ["payload"],
  "crypt_encoding": "base64",
  "iv_from": "random_prefix",
  "key": "demo_sm4_16b_key",
  "sign_key": "demo_hmac_sm3_key",
  "order": "encrypt_then_sign",
  "fail_in": "envelope"
}
```

- **报文流转说明**：
  1. 客户端构造明文业务 JSON `{"city":"hz"}`，对明文字节计算 `HMAC-SM3` 小写十六进制字符串填入 `sign`；
  2. 生成 16 字节随机 `IV`，计算 `SM4-CBC-PKCS7(key, IV, plain)`，将 `IV || Ciphertext` 做标准 Base64 编码填入 `payload`，`POST` 至 `http://127.0.0.1:16013/sm4`；
  3. 服务端先剥离前 16 字节 `IV` 解密 `payload`，校验明文 `HMAC-SM3` 签名（若签名错误，因 `fail_in = "envelope"` 且已完成解密，服务端将 `{"ok":false,"reason":"bad_sign",...}` 重新用随机前缀 IV 加密后以 HTTP `200` 返回）；
  4. 验签通过后，服务端对成功契约明文加密并附上明文响应签名，返回 `{"payload":"<base64_iv_and_cipher>","sign":"<hex_hmac_sm3>"}`。

---

## 9. 注意事项

- 信封密钥通过 `POST /api/v1/envelopes/secrets` 上传或随机生成后，`GET /api/v1/envelopes/secrets` 仅返回密钥名称、类型、SHA-256 指纹与公开材料（公钥 PEM），永不回显对称密钥或私钥原文。
- 所有十六进制编码字段（`sign_encoding = "hex"`、`crypt_encoding = "hex"`）均严格执行 `hex_case` 大小写比对，大小写不一致直接判为验签或解密失败。
