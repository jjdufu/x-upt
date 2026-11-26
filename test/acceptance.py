#!/usr/bin/env python3
"""Call the release binary. Artifacts go to /tmp/tte."""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import socket
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
TTE = "/tmp/tte"
CLIENT_ID = "jwczjzl"
SECRET = "K9mQ2vXp#8Lr4TzYw7nR"
POSTS = {
    "keyword": "gRPC",
    "user_id": 6,
    "page": 1,
    "page_size": 10,
    "sort": "id_desc",
}
PHOTO = {"scene": "article_cover", "width": 32, "height": 32, "format": "png"}
RESULTS: list[str] = []


def log(ok: bool, name: str, detail: str = "") -> None:
    mark = "PASS" if ok else "FAIL"
    line = f"{mark} {name}" + (f" :: {detail}" if detail else "")
    RESULTS.append(line)
    print(line, flush=True)


def fail_detail(err: BaseException) -> str:
    if isinstance(err, urllib.error.HTTPError):
        body = err.read().decode("utf-8", "replace")[:400]
        return f"HTTP {err.code} {body}"
    return str(err)


def urlopen(req: urllib.request.Request, timeout: float = 15):
    return urllib.request.urlopen(req, timeout=timeout)


def http(
    method: str,
    url: str,
    *,
    headers: dict | None = None,
    data: bytes | None = None,
    timeout: float = 15,
):
    hdrs = headers or {}
    req = urllib.request.Request(url, data=data, method=method, headers=hdrs)
    def header_map(h) -> dict:
        out = dict(h)
        if hasattr(h, "get_all"):
            multi = h.get_all("WWW-Authenticate") or h.get_all("www-authenticate") or []
            if multi:
                out["WWW-Authenticate"] = ", ".join(multi)
        return out

    try:
        with urlopen(req, timeout=timeout) as resp:
            body = resp.read()
            return resp.status, header_map(resp.headers), body
    except urllib.error.HTTPError as e:
        body = e.read()
        return e.code, header_map(e.headers), body


def json_body(obj: dict) -> bytes:
    return json.dumps(obj, separators=(",", ":")).encode()


def b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def posts_ok(obj: dict, where: str) -> bool:
    try:
        total = obj.get("total")
        items = obj.get("items") or []
        ok = total == 1 and items and items[0].get("id") == 18
        log(ok, where, f"total={total} id={items[0].get('id') if items else None}")
        return ok
    except Exception as e:
        log(False, where, str(e))
        return False


def wait_port(port: int, seconds: float = 20) -> bool:
    deadline = time.time() + seconds
    while time.time() < deadline:
        s = socket.socket()
        s.settimeout(0.3)
        try:
            s.connect(("127.0.0.1", port))
            s.close()
            return True
        except OSError:
            time.sleep(0.2)
        finally:
            s.close()
    return False


def setup_config() -> None:
    os.makedirs(os.path.join(ROOT, "target/release/config/keys"), exist_ok=True)
    src = os.path.join(ROOT, "config/app.toml")
    dst = os.path.join(ROOT, "target/release/config/app.toml")
    with open(src, "rb") as f:
        data = f.read()
    with open(dst, "wb") as f:
        f.write(data)
    pub_src = os.path.join(ROOT, "config/keys/jwczjzl_rsa_pub.pem")
    pub_dst = os.path.join(ROOT, "target/release/config/keys/jwczjzl_rsa_pub.pem")
    with open(pub_src, "rb") as f:
        data = f.read()
    with open(pub_dst, "wb") as f:
        f.write(data)
    env_src = os.path.join(ROOT, "config/keys/envelope")
    env_dst = os.path.join(ROOT, "target/release/config/keys/envelope")
    if os.path.isdir(env_src):
        os.makedirs(env_dst, exist_ok=True)
        for name in os.listdir(env_src):
            src = os.path.join(env_src, name)
            if os.path.isfile(src):
                with open(src, "rb") as f:
                    blob = f.read()
                with open(os.path.join(env_dst, name), "wb") as f:
                    f.write(blob)


def start_server() -> subprocess.Popen:
    for p in list(range(3000, 3009)) + list(range(3010, 3016)):
        subprocess.run(["fuser", "-k", f"{p}/tcp"], check=False, capture_output=True)
    time.sleep(0.4)
    env = os.environ.copy()
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target/release/upupt")],
        cwd=ROOT,
        stdout=open(os.path.join(TTE, "console.log"), "ab"),
        stderr=subprocess.STDOUT,
        env=env,
    )
    with open(os.path.join(TTE, "upupt.pid"), "w") as f:
        f.write(str(proc.pid))
    return proc


def bind_ok() -> None:
    ok = all(wait_port(p) for p in range(3000, 3009))
    log(ok, "1.bind-3000-3008")


def oauth_token(port: int, path: str) -> str:
    body = urllib.parse.urlencode(
        {
            "grant_type": "client_credentials",
            "client_id": CLIENT_ID,
            "client_secret": SECRET,
        }
    ).encode()
    st, _, raw = http(
        "POST",
        f"http://127.0.0.1:{port}{path}",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        data=body,
    )
    obj = json.loads(raw.decode())
    if st != 200 or "access_token" not in obj:
        raise RuntimeError(f"token {port}{path} -> {st} {raw[:200]!r}")
    return obj["access_token"]


def rest_posts(headers: dict, data: bytes, ctype: str, name: str) -> dict:
    st, hdr, raw = http(
        "POST",
        "http://127.0.0.1:3002/posts/search",
        headers={**headers, "Content-Type": ctype},
        data=data,
    )
    obj = json.loads(raw.decode() or "{}")
    ok = st == 200 and obj.get("total") == 1
    log(ok, name, f"status={st} total={obj.get('total')}")
    return obj


def test_three_methods_nine_fronts() -> None:
    token0 = oauth_token(3000, "/v3/oauth/token")
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3000/v3/gateway",
        headers={
            "Authorization": f"Bearer {token0}",
            "Content-Type": "application/json",
        },
        data=json_body({"method": "upupt.posts.search", "biz_content": POSTS}),
    )
    env = json.loads(raw.decode())
    posts_ok(env.get("data") or {}, "2.oauth.posts")

    # sign
    ts = str(int(time.time()))
    nonce = uuid.uuid4().hex[:16]
    biz = json.dumps(POSTS, separators=(",", ":"))
    fields = {
        "app_id": CLIENT_ID,
        "method": "upupt.posts.search",
        "charset": "UTF-8",
        "version": "1.0",
        "timestamp": ts,
        "nonce": nonce,
        "sign_type": "HMAC-SHA256",
        "biz_content": biz,
    }
    src = "&".join(f"{k}={v}" for k, v in sorted(fields.items()) if v)
    fields["sign"] = base64.b64encode(
        hmac.new(SECRET.encode(), src.encode(), hashlib.sha256).digest()
    ).decode()
    form = urllib.parse.urlencode(fields).encode()
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3001/gateway.do",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        data=form,
    )
    obj = json.loads(raw.decode())
    inner = obj.get("upupt_posts_search_response") or {}
    posts_ok(inner, "2.sign.posts UTF-8")

    fields2 = dict(fields)
    fields2["charset"] = "utf-8"
    fields2["nonce"] = uuid.uuid4().hex[:16]
    src2 = "&".join(f"{k}={v}" for k, v in sorted(fields2.items()) if k != "sign" and v)
    fields2["sign"] = base64.b64encode(
        hmac.new(SECRET.encode(), src2.encode(), hashlib.sha256).digest()
    ).decode()
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3001/gateway.do",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        data=urllib.parse.urlencode(fields2).encode(),
    )
    inner = json.loads(raw.decode()).get("upupt_posts_search_response") or {}
    posts_ok(inner, "4.sign.charset utf-8")

    key_h = {"X-API-Key": SECRET}
    rest_posts(key_h, json_body(POSTS), "application/json", "2.rest.json")

    # jsonrpc
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3003/rpc",
        headers={"Content-Type": "application/json", "X-API-Key": SECRET},
        data=json_body(
            {
                "jsonrpc": "2.0",
                "method": "upupt.posts.search",
                "params": POSTS,
                "id": 1,
            }
        ),
    )
    obj = json.loads(raw.decode())
    posts_ok(obj.get("result") or {}, "2.jsonrpc.posts")

    # graphql
    q = (
        "query($keyword:String!,$userId:Int!,$page:Int!,$pageSize:Int!,$sort:PostSort!)"
        "{ postsSearch(keyword:$keyword,userId:$userId,page:$page,pageSize:$pageSize,sort:$sort)"
        "{ total items { id } } }"
    )
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3004/graphql",
        headers={"Content-Type": "application/json", "X-API-Key": SECRET},
        data=json_body(
            {
                "query": q,
                "variables": {
                    "keyword": "gRPC",
                    "userId": 6,
                    "page": 1,
                    "pageSize": 10,
                    "sort": "ID_DESC",
                },
            }
        ),
    )
    obj = json.loads(raw.decode())
    data = ((obj.get("data") or {}).get("postsSearch")) or {}
    log(
        st == 200 and data.get("total") == 1 and data.get("items", [{}])[0].get("id") == 18,
        "2.graphql.posts",
        f"status={st} total={data.get('total')}",
    )

    st, _, _ = http("GET", "http://127.0.0.1:3004/graphql")
    log(st == 405, "6.graphql.GET.405", f"status={st}")

    # soap
    xml = (
        '<?xml version="1.0"?>'
        '<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">'
        "<s:Body><SearchPosts>"
        "<keyword>gRPC</keyword><user_id>6</user_id>"
        "<page>1</page><page_size>10</page_size><sort>id_desc</sort>"
        "</SearchPosts></s:Body></s:Envelope>"
    )
    basic = "Basic " + base64.b64encode(f"{CLIENT_ID}:{SECRET}".encode()).decode()
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3006/soap",
        headers={"Content-Type": "text/xml", "Authorization": basic},
        data=xml.encode(),
    )
    text = raw.decode()
    log(st == 200 and "<id>18</id>" in text and "<userId>6</userId>" in text, "2.soap.posts")

    # job
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3008/jobs",
        headers={"Content-Type": "application/json", "X-API-Key": SECRET},
        data=json_body({"method": "upupt.posts.search", "biz_content": POSTS}),
    )
    job = json.loads(raw.decode())
    log(st == 202 and job.get("status") == "pending", "5.job.submit", f"status={st} {job.get('status')}")
    job_id = job.get("job_id")
    result = None
    for _ in range(40):
        time.sleep(0.15)
        st, _, raw = http(
            "GET",
            f"http://127.0.0.1:3008/jobs/{job_id}",
            headers={"X-API-Key": SECRET},
        )
        view = json.loads(raw.decode())
        if view.get("status") == "succeeded":
            result = view.get("result") or {}
            break
    posts_ok(result or {}, "5.job.poll.succeeded")
    log(
        isinstance(result, dict) and "items" in (result or {}),
        "5.job.result.json",
    )


def test_rest_shapes() -> None:
    key_h = {"X-API-Key": SECRET}
    a = rest_posts(key_h, json_body(POSTS), "application/json", "3.rest.json")
    form = urllib.parse.urlencode(
        {
            "keyword": "gRPC",
            "user_id": "6",
            "page": "1",
            "page_size": "10",
            "sort": "id_desc",
        }
    ).encode()
    b = rest_posts(key_h, form, "application/x-www-form-urlencoded", "3.rest.form")
    qs = urllib.parse.urlencode(
        {"keyword": "gRPC", "user_id": "6", "page": "1", "page_size": "10", "sort": "id_desc"}
    )
    st, _, raw = http(
        "GET",
        f"http://127.0.0.1:3002/posts/search?{qs}",
        headers=key_h,
    )
    c = json.loads(raw.decode())
    log(st == 200 and c.get("total") == 1, "3.rest.get", f"status={st} total={c.get('total')}")
    log(
        a.get("total") == b.get("total") == c.get("total") == 1,
        "3.rest.shapes.equal",
    )
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3002/posts/search",
        headers={**key_h, "Content-Type": "application/vnd.api+json"},
        data=json_body(POSTS),
    )
    obj = json.loads(raw.decode() or "{}")
    log(st != 415 and obj.get("total") == 1, "3.rest.vnd.api+json", f"status={st}")

    st, hdr, raw = http(
        "HEAD",
        "http://127.0.0.1:3002/posts/search",
        headers=key_h,
    )
    log(st == 200 and len(raw) == 0, "HEAD.200.empty", f"status={st} len={len(raw)}")

    img_path = os.path.join(ROOT, "imgs/img_001.png")
    with open(img_path, "rb") as f:
        img_bytes = f.read()
    b64 = base64.b64encode(img_bytes).decode()
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3002/imgdef/compare",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body({"filename": "img_001.png", "image": b64, "threshold": 10}),
    )
    obj = json.loads(raw.decode())
    log(st == 200 and "matched" in obj, "2.rest.imgdef.json", f"status={st} {obj.get('matched')}")

    boundary = "----upuptbound"
    def part(name: str, content: bytes, filename: str | None = None) -> bytes:
        disp = f'name="{name}"'
        if filename:
            disp += f'; filename="{filename}"'
        return (
            f"--{boundary}\r\nContent-Disposition: form-data; {disp}\r\n\r\n".encode()
            + content
            + b"\r\n"
        )
    mp = (
        part("filename", b"img_001.png")
        + part("threshold", b"10")
        + part("image", img_bytes, "img_001.png")
        + f"--{boundary}--\r\n".encode()
    )
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3002/imgdef/compare",
        headers={**key_h, "Content-Type": f"multipart/form-data; boundary={boundary}"},
        data=mp,
    )
    obj = json.loads(raw.decode() or "{}")
    log(st == 200 and "matched" in obj, "3.rest.imgdef.multipart", f"status={st}")


def test_auth_profiles() -> None:
    url = "http://127.0.0.1:3002/posts/search"
    body = json_body(POSTS)

    st, _, raw = http("POST", url, headers={"Content-Type": "application/json", "X-API-Key": SECRET}, data=body)
    log(st == 200, "8.api_key", f"status={st}")

    basic = "Basic " + base64.b64encode(f"{CLIENT_ID}:{SECRET}".encode()).decode()
    st, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": basic}, data=body)
    log(st == 200, "8.basic", f"status={st}")

    tok = oauth_token(3002, "/oauth/token")
    st, _, _ = http(
        "POST",
        url,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {tok}"},
        data=body,
    )
    log(st == 200, "8.oauth_bearer", f"status={st}")

    # jwt HS256
    now = int(time.time())
    header = b64url(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
    payload = b64url(
        json.dumps({"iss": CLIENT_ID, "sub": CLIENT_ID, "iat": now, "exp": now + 300}).encode()
    )
    msg = f"{header}.{payload}"
    sig = b64url(hmac.new(SECRET.encode(), msg.encode(), hashlib.sha256).digest())
    jwt = f"{msg}.{sig}"
    st, _, _ = http(
        "POST",
        url,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {jwt}"},
        data=body,
    )
    log(st == 200, "8.jwt.HS256", f"status={st}")

    # digest
    st, hdr, _ = http("POST", url, headers={"Content-Type": "application/json"}, data=body)
    wa = hdr.get("WWW-Authenticate") or hdr.get("www-authenticate") or ""
    # urllib may join multiple
    has_md5 = "MD5" in wa or "md5" in wa.lower()
    has_sha = "SHA-256" in wa or "sha-256" in wa.lower()
    log(st == 401 and has_md5 and has_sha, "8.digest.challenge", f"status={st} wa={wa[:180]}")
    # parse nonce from first Digest
    nonce = ""
    realm = "upupt"
    algo = "MD5"
    for chunk in wa.replace(", Digest", "\nDigest").split("\n"):
        if "Digest" in chunk and "MD5" in chunk:
            for part in chunk.split(","):
                part = part.strip()
                if part.lower().startswith("nonce="):
                    nonce = part.split("=", 1)[1].strip().strip('"')
                if part.lower().startswith("realm="):
                    realm = part.split("=", 1)[1].strip().strip('"')
    if nonce:
        uri = "/posts/search"
        ha1 = hashlib.md5(f"{CLIENT_ID}:{realm}:{SECRET}".encode()).hexdigest()
        ha2 = hashlib.md5(f"POST:{uri}".encode()).hexdigest()
        cnonce = "0a1b2c3d"
        nc = "00000001"
        resp = hashlib.md5(f"{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}".encode()).hexdigest()
        auth = (
            f'Digest username="{CLIENT_ID}", realm="{realm}", nonce="{nonce}", '
            f'uri="{uri}", algorithm={algo}, response="{resp}", qop=auth, nc={nc}, cnonce="{cnonce}"'
        )
        st, _, _ = http(
            "POST",
            url,
            headers={"Content-Type": "application/json", "Authorization": auth},
            data=body,
        )
        log(st == 200, "8.digest.verify", f"status={st}")
    else:
        log(False, "8.digest.verify", "no nonce")

    # hawk
    ts = str(int(time.time()))
    hn = "j4h3g2"
    host = "127.0.0.1"
    port = "3002"
    resource = "/posts/search"
    payload_hash = base64.b64encode(
        hashlib.sha256(f"hawk.1.payload\napplication/json\n{body.decode()}\n".encode()).digest()
    ).decode()
    normalized = (
        f"hawk.1.header\n{ts}\n{hn}\nPOST\n{resource}\n{host}\n{port}\n{payload_hash}\n\n"
    )
    mac = base64.b64encode(hmac.new(SECRET.encode(), normalized.encode(), hashlib.sha256).digest()).decode()
    hawk = (
        f'Hawk id="{CLIENT_ID}", ts="{ts}", nonce="{hn}", mac="{mac}", hash="{payload_hash}"'
    )
    st, _, raw = http(
        "POST",
        url,
        headers={"Content-Type": "application/json", "Authorization": hawk, "Host": "127.0.0.1:3002"},
        data=body,
    )
    log(st == 200, "8.hawk", f"status={st} {raw[:120]!r}")

    # aws sigv4
    now = time.gmtime()
    amz_date = time.strftime("%Y%m%dT%H%M%SZ", now)
    date_stamp = time.strftime("%Y%m%d", now)
    region, service = "us-east-1", "execute-api"
    payload_hex = hashlib.sha256(body).hexdigest()
    canonical_headers = (
        f"host:127.0.0.1:3002\n"
        f"x-amz-content-sha256:{payload_hex}\n"
        f"x-amz-date:{amz_date}\n"
    )
    signed_headers = "host;x-amz-content-sha256;x-amz-date"
    canonical = f"POST\n/posts/search\n\n{canonical_headers}\n{signed_headers}\n{payload_hex}"
    canon_hash = hashlib.sha256(canonical.encode()).hexdigest()
    scope = f"{date_stamp}/{region}/{service}/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{canon_hash}"

    def hmac_sha256(key: bytes, msg: str) -> bytes:
        return hmac.new(key, msg.encode(), hashlib.sha256).digest()

    k_date = hmac_sha256(f"AWS4{SECRET}".encode(), date_stamp)
    k_region = hmac_sha256(k_date, region)
    k_service = hmac_sha256(k_region, service)
    k_signing = hmac_sha256(k_service, "aws4_request")
    signature = hmac.new(k_signing, sts.encode(), hashlib.sha256).hexdigest()
    authz = (
        f"AWS4-HMAC-SHA256 Credential={CLIENT_ID}/{scope}, "
        f"SignedHeaders={signed_headers}, Signature={signature}"
    )
    st, _, raw = http(
        "POST",
        url,
        headers={
            "Content-Type": "application/json",
            "Authorization": authz,
            "X-Amz-Date": amz_date,
            "X-Amz-Content-Sha256": payload_hex,
            "Host": "127.0.0.1:3002",
        },
        data=body,
    )
    log(st == 200, "8.aws_sigv4", f"status={st} {raw[:120]!r}")


def test_request_id() -> None:
    rid = str(uuid.uuid4())
    st, hdr, _ = http(
        "POST",
        "http://127.0.0.1:3002/posts/search",
        headers={
            "Content-Type": "application/json",
            "X-API-Key": SECRET,
            "X-Request-Id": rid,
        },
        data=json_body(POSTS),
    )
    got = hdr.get("X-Request-Id") or hdr.get("x-request-id")
    log(st == 200 and got == rid, "9.rest.x-request-id", f"got={got}")
    st, hdr, _ = http(
        "POST",
        "http://127.0.0.1:3002/posts/search",
        headers={"Content-Type": "application/json", "X-API-Key": SECRET},
        data=json_body(POSTS),
    )
    got = hdr.get("X-Request-Id") or hdr.get("x-request-id") or ""
    log(st == 200 and len(got) >= 8, "9.rest.generated-id", f"got={got}")
    logpath = os.path.join(ROOT, "logs/upupt.log")
    found = False
    if os.path.isfile(logpath):
        with open(logpath, encoding="utf-8", errors="replace") as f:
            tail = f.readlines()[-30:]
        found = any(rid in line for line in tail)
    log(found, "9.log.has-request-id")


def varint(n: int) -> bytes:
    out = bytearray()
    while n > 0x7F:
        out.append((n & 0x7F) | 0x80)
        n >>= 7
    out.append(n)
    return bytes(out)


def proto_str(field: int, s: str) -> bytes:
    b = s.encode()
    return varint((field << 3) | 2) + varint(len(b)) + b


def proto_var(field: int, n: int) -> bytes:
    return varint((field << 3) | 0) + varint(n)


def grpc_frame(payload: bytes) -> bytes:
    return b"\x00" + struct.pack(">I", len(payload)) + payload


def read_varint(buf: bytes, i: int) -> tuple[int, int]:
    n = 0
    shift = 0
    while i < len(buf):
        b = buf[i]
        i += 1
        n |= (b & 0x7F) << shift
        if b < 0x80:
            return n, i
        shift += 7
    raise ValueError("bad varint")


def proto_fields(buf: bytes) -> dict[int, object]:
    i = 0
    out: dict[int, object] = {}
    while i < len(buf):
        key, i = read_varint(buf, i)
        field, wire = key >> 3, key & 7
        if wire == 0:
            val, i = read_varint(buf, i)
            out[field] = val
        elif wire == 2:
            ln, i = read_varint(buf, i)
            out[field] = buf[i : i + ln]
            i += ln
        else:
            raise ValueError(f"wire {wire}")
    return out


def grpc_unary(rpc: str, payload: bytes, extra: list[str] | None = None) -> bytes:
    frame = grpc_frame(payload)
    path = f"/upupt.v1.Upupt/{rpc}"
    cmd = [
        "curl",
        "--http2-prior-knowledge",
        "-sS",
        "-D",
        os.path.join(TTE, "grpc.hdr"),
        "-o",
        os.path.join(TTE, "grpc.bin"),
        "-X",
        "POST",
        f"http://127.0.0.1:3005{path}",
        "-H",
        "content-type: application/grpc",
        "-H",
        "te: trailers",
        "-H",
        f"x-api-key: {SECRET}",
        "--data-binary",
        "@-",
    ]
    if extra:
        cmd.extend(extra)
    p = subprocess.run(cmd, input=frame, capture_output=True, check=False)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.decode()[:300])
    with open(os.path.join(TTE, "grpc.bin"), "rb") as f:
        return f.read()


def test_grpc() -> None:
    payload = (
        proto_str(1, "gRPC")
        + proto_var(2, 6)
        + proto_var(3, 1)
        + proto_var(4, 10)
        + proto_str(5, "id_desc")
    )
    raw = grpc_unary("SearchPosts", payload)
    log(len(raw) >= 5, "7.grpc.search.bytes", f"len={len(raw)}")
    if len(raw) >= 5:
        msg = raw[5:]
        fields = proto_fields(msg)
        total = fields.get(2, 0)
        log(total == 1, "7.grpc.posts.total", f"total={total}")

    photo = proto_str(1, "article_cover") + proto_var(2, 32) + proto_var(3, 32) + proto_str(4, "png")
    raw1 = grpc_unary("RenderPhoto", photo)
    raw2 = grpc_unary("RenderPhoto", photo)
    fn1 = fn2 = b""
    if len(raw1) >= 5:
        fn1 = proto_fields(raw1[5:]).get(5, b"")
    if len(raw2) >= 5:
        fn2 = proto_fields(raw2[5:]).get(5, b"")
    if isinstance(fn1, bytes):
        fn1 = fn1.decode()
    if isinstance(fn2, bytes):
        fn2 = fn2.decode()
    log(bool(fn1) and fn1 == fn2, "7.grpc.photos.filename", f"{fn1} == {fn2}")

    rid = str(uuid.uuid4())
    raw = grpc_unary("SearchPosts", payload, extra=["-H", f"x-request-id: {rid}", "-D", os.path.join(TTE, "grpc.rid.hdr")])
    hdr = open(os.path.join(TTE, "grpc.rid.hdr"), encoding="utf-8", errors="replace").read()
    log(rid in hdr or rid.replace("-", "") in hdr, "9.grpc.x-request-id", hdr[:200])


def ws_frame(payload: bytes, opcode: int = 1) -> bytes:
    ln = len(payload)
    mask = os.urandom(4)
    if ln < 126:
        hdr = bytes([0x80 | opcode, 0x80 | ln])
    elif ln < 65536:
        hdr = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack(">H", ln)
    else:
        hdr = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack(">Q", ln)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return hdr + mask + masked


def read_ws(sock: socket.socket) -> bytes:
    hdr = sock.recv(2)
    if len(hdr) < 2:
        return b""
    ln = hdr[1] & 0x7F
    if ln == 126:
        ln = struct.unpack(">H", sock.recv(2))[0]
    elif ln == 127:
        ln = struct.unpack(">Q", sock.recv(8))[0]
    if hdr[1] & 0x80:
        sock.recv(4)
    data = b""
    while len(data) < ln:
        chunk = sock.recv(ln - len(data))
        if not chunk:
            break
        data += chunk
    return data


def test_ws() -> None:
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        "GET /ws?api_key=" + urllib.parse.quote(SECRET) + " HTTP/1.1\r\n"
        "Host: 127.0.0.1:3007\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    )
    s = socket.create_connection(("127.0.0.1", 3007), timeout=10)
    s.sendall(req.encode())
    handshake = b""
    while b"\r\n\r\n" not in handshake:
        handshake += s.recv(4096)
    ok_hs = b"101" in handshake.split(b"\r\n", 1)[0]
    frame = json_body(
        {"id": "1", "method": "upupt.posts.search", "biz_content": POSTS}
    )
    s.sendall(ws_frame(frame))
    s.settimeout(8)
    data = read_ws(s)
    s.close()
    try:
        obj = json.loads(data.decode())
        posts_ok(obj.get("data") or {}, "2.ws.posts")
    except Exception as e:
        log(False, "2.ws.posts", f"hs={ok_hs} {e} {data[:200]!r}")


def test_photos_rest() -> None:
    key_h = {"X-API-Key": SECRET}
    st, _, raw1 = http(
        "POST",
        "http://127.0.0.1:3002/photos/render",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body(PHOTO),
    )
    st2, _, raw2 = http(
        "POST",
        "http://127.0.0.1:3002/photos/render",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body(PHOTO),
    )
    a = json.loads(raw1.decode())
    b = json.loads(raw2.decode())
    fn1 = (a.get("image") or {}).get("filename")
    fn2 = (b.get("image") or {}).get("filename")
    log(st == 200 and st2 == 200 and fn1 and fn1 == fn2, "7.rest.photos.filename", f"{fn1} == {fn2}")


def test_no_fourth_and_flags() -> None:
    st, _, raw = http(
        "POST",
        "http://127.0.0.1:3002/posts/search",
        headers={"X-API-Key": SECRET, "Content-Type": "application/json"},
        data=json_body({"method": "upupt.echo", "biz_content": {}}),
    )
    # unknown fields on posts.search
    log(st in (400, 422), "10.no-echo-on-posts", f"status={st}")
    banned = [
        "binary_photo",
        "multipart_imgdef",
        "playground",
        "base64_image",
        "http_token",
        "reflection",
    ]
    hits = []
    for root, dirs, files in os.walk(ROOT):
        if "target" in root.split(os.sep) or ".git" in root.split(os.sep):
            continue
        for fn in files:
            if "changelog" in root.split(os.sep):
                continue
            if fn.endswith((".rs", ".toml")):
                path = os.path.join(root, fn)
                try:
                    text = open(path, encoding="utf-8", errors="ignore").read()
                except OSError:
                    continue
                for b in banned:
                    if b in text:
                        hits.append(f"{path}:{b}")
    log(not hits, "10.no-deleted-flags", "; ".join(hits[:8]))


def main() -> int:
    os.makedirs(TTE, exist_ok=True)
    setup_config()
    proc = start_server()
    try:
        bind_ok()
        test_three_methods_nine_fronts()
        test_rest_shapes()
        test_photos_rest()
        test_auth_profiles()
        test_request_id()
        test_grpc()
        test_ws()
        test_no_fourth_and_flags()
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=12)
        except subprocess.TimeoutExpired:
            proc.kill()
    out = os.path.join(TTE, "results.txt")
    with open(out, "w", encoding="utf-8") as f:
        f.write("\n".join(RESULTS) + "\n")
    failed = sum(1 for l in RESULTS if l.startswith("FAIL"))
    print(f"\n{failed} failed, {len(RESULTS)} total. wrote {out}", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
