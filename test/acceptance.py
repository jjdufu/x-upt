#!/usr/bin/env python3
"""End-to-end acceptance suite for x-upt: blank precondition gates, API seeding, 9 core fronts & 10 auth modes."""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib

CLIENT_ID = "demo_client"
SECRET = "DemoSecret_9xK2mQp8Lz4Vw7"
POSTS_REQ = {
    "keyword": "gRPC",
    "user_id": 6,
    "page": 1,
    "page_size": 10,
    "sort": "id_desc",
}
PHOTO_REQ = {"scene": "article_cover", "width": 32, "height": 32, "format": "png"}
RESULTS: list[str] = []


def log(ok: bool, name: str, detail: str = "") -> None:
    mark = "PASS" if ok else "FAIL"
    line = f"{mark} {name}" + (f" :: {detail}" if detail else "")
    RESULTS.append(line)
    print(line, flush=True)


def make_png(seed: int) -> bytes:
    width, height = 16, 16
    sig = b"\x89PNG\r\n\x1a\n"

    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend(((x * 13 + seed * 37) & 0xFF, (y * 17 + seed * 53) & 0xFF, ((x ^ y) * 19 + seed * 11) & 0xFF))
    idat = zlib.compress(bytes(rows))
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def http(
    method: str,
    url: str,
    *,
    headers: dict[str, str] | None = None,
    data: bytes | None = None,
    timeout: float = 15,
) -> tuple[int, dict[str, str], bytes]:
    hdrs = dict(headers or {})
    req = urllib.request.Request(url, data=data, method=method, headers=hdrs)

    def extract_headers(h: object) -> dict[str, str]:
        out = dict(h.items()) if hasattr(h, "items") else {}
        if hasattr(h, "get_all"):
            multi = h.get_all("WWW-Authenticate") or h.get_all("www-authenticate") or []
            if multi:
                out["WWW-Authenticate"] = ", ".join(multi)
        return out

    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, extract_headers(resp.headers), resp.read()
    except urllib.error.HTTPError as err:
        return err.code, extract_headers(err.headers), err.read()


def json_body(obj: object) -> bytes:
    return json.dumps(obj, separators=(",", ":")).encode("utf-8")


def b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def admin_req(
    admin_url: str,
    method: str,
    path: str,
    payload: object | None = None,
) -> tuple[int, dict[str, object]]:
    parsed = urllib.parse.urlparse(admin_url)
    host = parsed.netloc
    hdrs = {"Accept": "application/json", "Host": host, "Origin": f"{parsed.scheme}://{host}"}
    body = None
    if payload is not None:
        hdrs["Content-Type"] = "application/json"
        body = json_body(payload)
    st, _, raw = http(method, f"{admin_url}{path}", headers=hdrs, data=body)
    obj = json.loads(raw.decode("utf-8") or "{}")
    return st, obj if isinstance(obj, dict) else {"data": obj}


def wait_http_ok(url: str, seconds: float = 15.0) -> bool:
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            st, _, _ = http("GET", url, timeout=1.5)
            if st == 200:
                return True
        except Exception:
            pass
        time.sleep(0.15)
    return False


def wait_port(port: int, seconds: float = 10.0) -> bool:
    deadline = time.time() + seconds
    while time.time() < deadline:
        s = socket.socket()
        s.settimeout(0.3)
        try:
            s.connect(("127.0.0.1", port))
            return True
        except OSError:
            time.sleep(0.1)
        finally:
            s.close()
    return False


def posts_ok(obj: dict[str, object], where: str) -> bool:
    total = obj.get("total")
    items = obj.get("items")
    first_id = items[0].get("id") if isinstance(items, list) and items and isinstance(items[0], dict) else None
    ok = total == 1 and first_id == 18
    log(ok, where, f"total={total} id={first_id}")
    return ok


def gen_rsa_keypair(workdir: str) -> tuple[str, str]:
    priv_path = os.path.join(workdir, "rsa_priv.pem")
    subprocess.run(
        ["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", priv_path],
        check=True,
        capture_output=True,
    )
    pub_proc = subprocess.run(
        ["openssl", "rsa", "-in", priv_path, "-pubout"],
        check=True,
        capture_output=True,
    )
    return priv_path, pub_proc.stdout.decode("utf-8")


def rsa_sign_sha256(priv_path: str, msg: bytes) -> bytes:
    proc = subprocess.run(
        ["openssl", "dgst", "-sha256", "-sign", priv_path],
        input=msg,
        check=True,
        capture_output=True,
    )
    return proc.stdout


def seed_and_verify_preconditions(
    admin_url: str,
    port_base: int | None,
    workdir: str,
) -> tuple[dict[str, int], bytes, str]:
    st, status = admin_req(admin_url, "GET", "/api/v1/status")
    counts = status.get("counts") if isinstance(status.get("counts"), dict) else {}
    if counts.get("images") == 0 and counts.get("posts") == 0:
        for m in ("x_upt.photos.render", "x_upt.imgdef.compare", "x_upt.posts.search"):
            code, err = admin_req(admin_url, "POST", f"/api/v1/methods/{m}/enable")
            log(code == 409 and bool(err.get("missing")), f"0.precondition.block.{m}", f"status={code}")
        code_f, err_f = admin_req(admin_url, "POST", "/api/v1/fronts/rest/enable")
        log(code_f == 409 and bool(err_f.get("missing")), "0.precondition.block.rest", f"status={code_f}")

    cover_png = make_png(1)
    avatar_png = make_png(2)
    banner_png = make_png(3)
    st_img, _ = admin_req(
        admin_url,
        "POST",
        "/api/v1/images",
        {
            "items": [
                {"scene": "article_cover", "filename": "cover_01.png", "base64": base64.b64encode(cover_png).decode()},
                {"scene": "avatar", "filename": "avatar_01.png", "base64": base64.b64encode(avatar_png).decode()},
                {"scene": "banner", "filename": "banner_01.png", "base64": base64.b64encode(banner_png).decode()},
            ]
        },
    )
    log(st_img == 200, "0.seed.images", f"status={st_img}")

    st_posts, _ = admin_req(
        admin_url,
        "POST",
        "/api/v1/posts/import",
        {
            "items": [
                {"id": 18, "userId": 6, "title": "gRPC Unary Service", "body": "Deterministic gRPC test post"},
                {"id": 19, "userId": 7, "title": "Other Author", "body": "Filtered out by user_id"},
            ]
        },
    )
    log(st_posts == 200, "0.seed.posts", f"status={st_posts}")

    rsa_priv_path, rsa_pub_pem = gen_rsa_keypair(workdir)
    st_app, _ = admin_req(
        admin_url,
        "POST",
        "/api/v1/apps",
        {
            "client_id": CLIENT_ID,
            "client_secret": SECRET,
            "name": "Acceptance Client",
            "rsa_public_pem": rsa_pub_pem,
            "methods": ["x_upt.posts.search", "x_upt.photos.render", "x_upt.imgdef.compare"],
        },
    )
    log(st_app == 200, "0.seed.app", f"status={st_app}")

    # Lower simulated latency so multi-protocol suite runs briskly while still exercising latency codepath.
    admin_req(
        admin_url,
        "PUT",
        "/api/v1/settings",
        {
            "latency_min_ms": 1,
            "latency_max_ms": 5,
            "token_expire_minutes": 30,
            "token_cleanup_seconds": 300,
            "token_max_per_client": 0,
            "sign_window_seconds": 300,
            "sign_nonce_ttl_seconds": 600,
            "job_expire_seconds": 600,
        },
    )

    for m in ("x_upt.posts.search", "x_upt.photos.render", "x_upt.imgdef.compare"):
        code_m, _ = admin_req(admin_url, "POST", f"/api/v1/methods/{m}/enable")
        log(code_m == 200, f"0.enable.{m}", f"status={code_m}")

    core_fronts = [
        "oauth_gateway",
        "sign_gateway",
        "rest",
        "jsonrpc",
        "graphql",
        "grpc",
        "soap",
        "ws",
        "job",
    ]
    _, fronts_resp = admin_req(admin_url, "GET", "/api/v1/fronts")
    front_items = {f["name"]: f for f in fronts_resp.get("items", []) if isinstance(f, dict)}
    ports: dict[str, int] = {}

    for idx, fname in enumerate(core_fronts):
        cur = front_items[fname]
        port = (port_base + idx) if port_base is not None else int(cur["port"])
        if port != int(cur["port"]):
            admin_req(
                admin_url,
                "PUT",
                f"/api/v1/fronts/{fname}",
                {
                    "host": "127.0.0.1",
                    "port": port,
                    "http2": bool(cur["http2"]),
                    "h2c": bool(cur["h2c"]),
                    "expire_minutes": int(cur["expire_minutes"]),
                    "auth": list(cur["auth"]),
                    "tls_enabled": False,
                    "ws_max_conn": int(cur["ws_max_conn"]),
                    "ws_ping_secs": int(cur["ws_ping_secs"]),
                    "job_workers": int(cur["job_workers"]),
                },
            )
        code_en, view = admin_req(admin_url, "POST", f"/api/v1/fronts/{fname}/enable")
        ports[fname] = int(view.get("port", port))
        log(code_en == 200 and wait_port(ports[fname]), f"1.bind.{fname}", f"port={ports[fname]}")

    # Active-dependency lock check: deleting demo_client while core fronts are active MUST return 409.
    del_app_code, _ = admin_req(admin_url, "DELETE", f"/api/v1/apps/{CLIENT_ID}")
    log(del_app_code == 409, "0.lock.active_app_409", f"status={del_app_code}")

    return ports, cover_png, rsa_priv_path


def oauth_token(port: int, path: str) -> str:
    body = urllib.parse.urlencode(
        {"grant_type": "client_credentials", "client_id": CLIENT_ID, "client_secret": SECRET}
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
    return str(obj["access_token"])


def test_nine_core_fronts(ports: dict[str, int], cover_png: bytes, workdir: str) -> None:
    # 1. oauth_gateway
    tok0 = oauth_token(ports["oauth_gateway"], "/v3/oauth/token")
    st, _, raw = http(
        "POST",
        f"http://127.0.0.1:{ports['oauth_gateway']}/v3/gateway",
        headers={"Authorization": f"Bearer {tok0}", "Content-Type": "application/json"},
        data=json_body({"method": "x_upt.posts.search", "biz_content": POSTS_REQ}),
    )
    env = json.loads(raw.decode())
    posts_ok(env.get("data") or {}, "2.oauth.posts")

    # 2. sign_gateway (UTF-8 and utf-8)
    for tag, cs in (("UTF-8", "UTF-8"), ("utf-8", "utf-8")):
        fields = {
            "app_id": CLIENT_ID,
            "method": "x_upt.posts.search",
            "charset": cs,
            "version": "1.0",
            "timestamp": str(int(time.time())),
            "nonce": uuid.uuid4().hex[:16],
            "sign_type": "HMAC-SHA256",
            "biz_content": json.dumps(POSTS_REQ, separators=(",", ":")),
        }
        src = "&".join(f"{k}={v}" for k, v in sorted(fields.items()) if v)
        fields["sign"] = base64.b64encode(hmac.new(SECRET.encode(), src.encode(), hashlib.sha256).digest()).decode()
        st, _, raw = http(
            "POST",
            f"http://127.0.0.1:{ports['sign_gateway']}/gateway.do",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            data=urllib.parse.urlencode(fields).encode(),
        )
        obj = json.loads(raw.decode())
        posts_ok(obj.get("x_upt_posts_search_response") or {}, f"2.sign.posts.{tag}")

    # 3. rest shapes & photos & imgdef
    rest_p = ports["rest"]
    key_h = {"X-API-Key": SECRET}
    for label, ctype, body in (
        ("3.rest.json", "application/json", json_body(POSTS_REQ)),
        ("3.rest.vnd.api+json", "application/vnd.api+json", json_body(POSTS_REQ)),
        (
            "3.rest.form",
            "application/x-www-form-urlencoded",
            urllib.parse.urlencode({k: str(v) for k, v in POSTS_REQ.items()}).encode(),
        ),
    ):
        st, _, raw = http("POST", f"http://127.0.0.1:{rest_p}/posts/search", headers={**key_h, "Content-Type": ctype}, data=body)
        posts_ok(json.loads(raw.decode()), label)

    qs = urllib.parse.urlencode({k: str(v) for k, v in POSTS_REQ.items()})
    st, _, raw = http("GET", f"http://127.0.0.1:{rest_p}/posts/search?{qs}", headers=key_h)
    posts_ok(json.loads(raw.decode()), "3.rest.get")

    st, _, raw = http("HEAD", f"http://127.0.0.1:{rest_p}/posts/search", headers=key_h)
    log(st == 200 and len(raw) == 0, "3.rest.head", f"status={st} len={len(raw)}")

    st1, _, r1 = http("POST", f"http://127.0.0.1:{rest_p}/photos/render", headers={**key_h, "Content-Type": "application/json"}, data=json_body(PHOTO_REQ))
    st2, _, r2 = http("POST", f"http://127.0.0.1:{rest_p}/photos/render", headers={**key_h, "Content-Type": "application/json"}, data=json_body(PHOTO_REQ))
    fn1 = (json.loads(r1.decode()).get("image") or {}).get("filename")
    fn2 = (json.loads(r2.decode()).get("image") or {}).get("filename")
    log(st1 == 200 and st2 == 200 and fn1 == "cover_01.png" and fn1 == fn2, "3.rest.photos.deterministic", f"fn={fn1}")

    b64_img = base64.b64encode(cover_png).decode()
    st, _, raw = http(
        "POST",
        f"http://127.0.0.1:{rest_p}/imgdef/compare",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body({"filename": "cover_01.png", "image": b64_img, "threshold": 5}),
    )
    cmp_obj = json.loads(raw.decode())
    log(st == 200 and cmp_obj.get("matched") is True and cmp_obj.get("distance") == 0, "3.rest.imgdef.json", f"dist={cmp_obj.get('distance')}")

    boundary = "----xuptbound77"
    mp = (
        f'--{boundary}\r\nContent-Disposition: form-data; name="filename"\r\n\r\ncover_01.png\r\n'
        f'--{boundary}\r\nContent-Disposition: form-data; name="threshold"\r\n\r\n5\r\n'
        f'--{boundary}\r\nContent-Disposition: form-data; name="image"; filename="cover_01.png"\r\n\r\n'
    ).encode() + cover_png + f"\r\n--{boundary}--\r\n".encode()
    st, _, raw = http(
        "POST",
        f"http://127.0.0.1:{rest_p}/imgdef/compare",
        headers={**key_h, "Content-Type": f"multipart/form-data; boundary={boundary}"},
        data=mp,
    )
    log(st == 200 and json.loads(raw.decode()).get("matched") is True, "3.rest.imgdef.multipart")

    # 4. jsonrpc (single + notification)
    rpc_p = ports["jsonrpc"]
    st, _, raw = http(
        "POST",
        f"http://127.0.0.1:{rpc_p}/rpc",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body({"jsonrpc": "2.0", "method": "x_upt.posts.search", "params": POSTS_REQ, "id": 1}),
    )
    posts_ok(json.loads(raw.decode()).get("result") or {}, "4.jsonrpc.posts")
    st_n, _, _ = http(
        "POST",
        f"http://127.0.0.1:{rpc_p}/rpc",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body({"jsonrpc": "2.0", "method": "x_upt.posts.search", "params": POSTS_REQ}),
    )
    log(st_n == 204, "4.jsonrpc.notification.204", f"status={st_n}")

    # 5. graphql
    gql_p = ports["graphql"]
    q = (
        "query($keyword:String!,$userId:Int!,$page:Int!,$pageSize:Int!,$sort:PostSort!)"
        "{ postsSearch(keyword:$keyword,userId:$userId,page:$page,pageSize:$pageSize,sort:$sort)"
        "{ total items { id } } }"
    )
    st, _, raw = http(
        "POST",
        f"http://127.0.0.1:{gql_p}/graphql",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body({"query": q, "variables": {"keyword": "gRPC", "userId": 6, "page": 1, "pageSize": 10, "sort": "ID_DESC"}}),
    )
    gql_data = ((json.loads(raw.decode()).get("data") or {}).get("postsSearch")) or {}
    log(st == 200 and gql_data.get("total") == 1, "5.graphql.posts")
    st_405, _, _ = http("GET", f"http://127.0.0.1:{gql_p}/graphql")
    log(st_405 == 405, "5.graphql.get.405", f"status={st_405}")
    st_sdl, _, sdl_raw = http("GET", f"http://127.0.0.1:{gql_p}/sdl")
    log(st_sdl == 200 and b"postsSearch" in sdl_raw, "5.graphql.sdl")

    # 6. soap
    soap_p = ports["soap"]
    st_w, _, wsdl_raw = http("GET", f"http://127.0.0.1:{soap_p}/soap")
    log(st_w == 200 and f":{soap_p}/soap".encode() in wsdl_raw, "6.soap.wsdl")
    xml = (
        '<?xml version="1.0"?>'
        '<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">'
        "<s:Body><SearchPosts>"
        "<keyword>gRPC</keyword><user_id>6</user_id><page>1</page><page_size>10</page_size><sort>id_desc</sort>"
        "</SearchPosts></s:Body></s:Envelope>"
    )
    basic = "Basic " + base64.b64encode(f"{CLIENT_ID}:{SECRET}".encode()).decode()
    st_s, _, s_raw = http("POST", f"http://127.0.0.1:{soap_p}/soap", headers={"Content-Type": "text/xml", "Authorization": basic}, data=xml.encode())
    log(st_s == 200 and b"<id>18</id>" in s_raw and b"<userId>6</userId>" in s_raw, "6.soap.posts")

    # 7. grpc
    test_grpc(ports["grpc"], workdir)

    # 8. ws
    test_ws(ports["ws"])

    # 9. job
    job_p = ports["job"]
    st_j, _, j_raw = http(
        "POST",
        f"http://127.0.0.1:{job_p}/jobs",
        headers={**key_h, "Content-Type": "application/json"},
        data=json_body({"method": "x_upt.posts.search", "biz_content": POSTS_REQ}),
    )
    j_obj = json.loads(j_raw.decode())
    job_id = j_obj.get("job_id")
    log(st_j == 202 and j_obj.get("status") == "pending", "9.job.submit")
    polled = {}
    for _ in range(40):
        time.sleep(0.1)
        _, _, p_raw = http("GET", f"http://127.0.0.1:{job_p}/jobs/{job_id}", headers=key_h)
        view = json.loads(p_raw.decode())
        if view.get("status") == "succeeded":
            polled = view.get("result") or {}
            break
    posts_ok(polled, "9.job.poll.succeeded")


def test_auth_and_logs(admin_url: str, rest_port: int, rsa_priv_path: str) -> None:
    url = f"http://127.0.0.1:{rest_port}/posts/search"
    body = json_body(POSTS_REQ)

    # api_key & basic & oauth_cc & gettoken
    st, _, _ = http("POST", url, headers={"Content-Type": "application/json", "X-API-Key": SECRET}, data=body)
    log(st == 200, "10.auth.api_key")
    basic = "Basic " + base64.b64encode(f"{CLIENT_ID}:{SECRET}".encode()).decode()
    st, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": basic}, data=body)
    log(st == 200, "10.auth.basic")
    tok = oauth_token(rest_port, "/oauth/token")
    st, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": f"Bearer {tok}"}, data=body)
    log(st == 200, "10.auth.oauth_cc")
    gt_qs = urllib.parse.urlencode({"app_id": CLIENT_ID, "secret": SECRET})
    st_gt, _, gt_raw = http("GET", f"http://127.0.0.1:{rest_port}/gettoken?{gt_qs}")
    gt_tok = json.loads(gt_raw.decode()).get("access_token", "")
    st_gt2, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": f"Bearer {gt_tok}"}, data=body)
    log(st_gt == 200 and st_gt2 == 200, "10.auth.gettoken")

    # JWT HS256 & RS256
    now = int(time.time())
    claims_b64 = b64url(json.dumps({"iss": CLIENT_ID, "sub": CLIENT_ID, "iat": now, "exp": now + 300}).encode())
    hs_hdr = b64url(b'{"alg":"HS256","typ":"JWT"}')
    hs_msg = f"{hs_hdr}.{claims_b64}"
    hs_sig = b64url(hmac.new(SECRET.encode(), hs_msg.encode(), hashlib.sha256).digest())
    st_hs, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": f"Bearer {hs_msg}.{hs_sig}"}, data=body)
    log(st_hs == 200, "10.auth.jwt_hs256")

    rs_hdr = b64url(b'{"alg":"RS256","typ":"JWT"}')
    rs_msg = f"{rs_hdr}.{claims_b64}"
    rs_sig = b64url(rsa_sign_sha256(rsa_priv_path, rs_msg.encode()))
    st_rs, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": f"Bearer {rs_msg}.{rs_sig}"}, data=body)
    log(st_rs == 200, "10.auth.jwt_rs256")

    # Digest (MD5 + SHA-256 dual challenge)
    st_d, hdr_d, _ = http("POST", url, headers={"Content-Type": "application/json"}, data=body)
    wa = hdr_d.get("WWW-Authenticate", "")
    log(st_d == 401 and "MD5" in wa and "SHA-256" in wa, "10.auth.digest_challenge")
    nonce, realm = "", "x-upt"
    for chunk in wa.replace(", Digest", "\nDigest").split("\n"):
        if "Digest" in chunk and "MD5" in chunk:
            for part in chunk.split(","):
                p = part.strip()
                if p.lower().startswith("nonce="):
                    nonce = p.split("=", 1)[1].strip().strip('"')
                elif p.lower().startswith("realm="):
                    realm = p.split("=", 1)[1].strip().strip('"')
    ha1 = hashlib.md5(f"{CLIENT_ID}:{realm}:{SECRET}".encode()).hexdigest()
    ha2 = hashlib.md5(b"POST:/posts/search").hexdigest()
    cnonce, nc = "0a1b2c3d", "00000001"
    d_resp = hashlib.md5(f"{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}".encode()).hexdigest()
    d_hdr = f'Digest username="{CLIENT_ID}", realm="{realm}", nonce="{nonce}", uri="/posts/search", algorithm=MD5, response="{d_resp}", qop=auth, nc={nc}, cnonce="{cnonce}"'
    st_dv, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": d_hdr}, data=body)
    log(st_dv == 200, "10.auth.digest_verify")

    # Hawk
    ts = str(int(time.time()))
    hn = "hawk9911"
    ph = base64.b64encode(hashlib.sha256(f"hawk.1.payload\napplication/json\n{body.decode()}\n".encode()).digest()).decode()
    norm = f"hawk.1.header\n{ts}\n{hn}\nPOST\n/posts/search\n127.0.0.1\n{rest_port}\n{ph}\n\n"
    mac = base64.b64encode(hmac.new(SECRET.encode(), norm.encode(), hashlib.sha256).digest()).decode()
    hawk_h = f'Hawk id="{CLIENT_ID}", ts="{ts}", nonce="{hn}", mac="{mac}", hash="{ph}"'
    st_hk, _, _ = http("POST", url, headers={"Content-Type": "application/json", "Authorization": hawk_h, "Host": f"127.0.0.1:{rest_port}"}, data=body)
    log(st_hk == 200, "10.auth.hawk")

    # AWS SigV4
    gm = time.gmtime()
    amz_date = time.strftime("%Y%m%dT%H%M%SZ", gm)
    date_s = time.strftime("%Y%m%d", gm)
    p_hex = hashlib.sha256(body).hexdigest()
    can_h = f"host:127.0.0.1:{rest_port}\nx-amz-content-sha256:{p_hex}\nx-amz-date:{amz_date}\n"
    sh = "host;x-amz-content-sha256;x-amz-date"
    canon = f"POST\n/posts/search\n\n{can_h}\n{sh}\n{p_hex}"
    scope = f"{date_s}/us-east-1/execute-api/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{hashlib.sha256(canon.encode()).hexdigest()}"

    def h256(k: bytes, m: str) -> bytes:
        return hmac.new(k, m.encode(), hashlib.sha256).digest()

    k_sign = h256(h256(h256(h256(f"AWS4{SECRET}".encode(), date_s), "us-east-1"), "execute-api"), "aws4_request")
    sig_hex = hmac.new(k_sign, sts.encode(), hashlib.sha256).hexdigest()
    aws_h = f"AWS4-HMAC-SHA256 Credential={CLIENT_ID}/{scope}, SignedHeaders={sh}, Signature={sig_hex}"
    st_aws, _, _ = http(
        "POST",
        url,
        headers={
            "Content-Type": "application/json",
            "Authorization": aws_h,
            "X-Amz-Date": amz_date,
            "X-Amz-Content-Sha256": p_hex,
            "Host": f"127.0.0.1:{rest_port}",
        },
        data=body,
    )
    log(st_aws == 200, "10.auth.aws_sigv4")

    # Request ID echo + /api/v1/logs/recent verification
    rid = f"req-accept-{uuid.uuid4().hex[:12]}"
    st_r, hdr_r, _ = http(
        "POST",
        url,
        headers={"Content-Type": "application/json", "X-API-Key": SECRET, "X-Request-Id": rid},
        data=body,
    )
    echoed = hdr_r.get("X-Request-Id") or hdr_r.get("x-request-id")
    log(st_r == 200 and echoed == rid, "11.request_id.echo", f"got={echoed}")
    _, logs_resp = admin_req(admin_url, "GET", f"/api/v1/logs/recent?request_id={rid}")
    items = logs_resp.get("items", [])
    log(isinstance(items, list) and len(items) == 1, "11.logs.recent.recorded")


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


def read_varint(buf: bytes, i: int) -> tuple[int, int]:
    n, shift = 0, 0
    while i < len(buf):
        b = buf[i]
        i += 1
        n |= (b & 0x7F) << shift
        if b < 0x80:
            return n, i
        shift += 7
    raise ValueError("truncated varint")


def proto_fields(buf: bytes) -> dict[int, object]:
    i, out = 0, {}
    while i < len(buf):
        key, i = read_varint(buf, i)
        field, wire = key >> 3, key & 7
        if wire == 0:
            out[field], i = read_varint(buf, i)
        elif wire == 2:
            ln, i = read_varint(buf, i)
            out[field] = buf[i : i + ln]
            i += ln
        else:
            raise ValueError(f"unexpected wire {wire}")
    return out


def test_grpc(grpc_port: int, workdir: str) -> None:
    payload = proto_str(1, "gRPC") + proto_var(2, 6) + proto_var(3, 1) + proto_var(4, 10) + proto_str(5, "id_desc")
    frame = b"\x00" + struct.pack(">I", len(payload)) + payload
    hdr_file = os.path.join(workdir, "grpc.hdr")
    bin_file = os.path.join(workdir, "grpc.bin")
    rid = f"grpc-{uuid.uuid4().hex[:10]}"
    cmd = [
        "curl",
        "--http2-prior-knowledge",
        "-sS",
        "-D",
        hdr_file,
        "-o",
        bin_file,
        "-X",
        "POST",
        f"http://127.0.0.1:{grpc_port}/x_upt.v1.XUpt/SearchPosts",
        "-H",
        "content-type: application/grpc",
        "-H",
        "te: trailers",
        "-H",
        f"x-api-key: {SECRET}",
        "-H",
        f"x-request-id: {rid}",
        "--data-binary",
        "@-",
    ]
    proc = subprocess.run(cmd, input=frame, capture_output=True, check=False)
    if proc.returncode != 0:
        log(False, "7.grpc.search", proc.stderr.decode()[:200])
        return
    with open(bin_file, "rb") as f:
        raw = f.read()
    with open(hdr_file, encoding="utf-8", errors="replace") as f:
        hdr_txt = f.read()
    total = proto_fields(raw[5:]).get(2, 0) if len(raw) >= 5 else 0
    log(total == 1 and rid in hdr_txt, "7.grpc.search_and_rid", f"total={total}")


def test_ws(ws_port: int) -> None:
    ws_key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET /ws?api_key={urllib.parse.quote(SECRET)} HTTP/1.1\r\n"
        f"Host: 127.0.0.1:{ws_port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {ws_key}\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n"
    )
    s = socket.create_connection(("127.0.0.1", ws_port), timeout=10)
    try:
        s.sendall(req.encode())
        hs = b""
        while b"\r\n\r\n" not in hs:
            hs += s.recv(4096)
        payload = json_body({"id": "ws-1", "method": "x_upt.posts.search", "biz_content": POSTS_REQ})
        mask = os.urandom(4)
        ln = len(payload)
        hdr = bytes([0x81, 0x80 | ln]) if ln < 126 else bytes([0x81, 0xFE]) + struct.pack(">H", ln)
        s.sendall(hdr + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))
        s.settimeout(8)
        fh = s.recv(2)
        fln = fh[1] & 0x7F
        if fln == 126:
            fln = struct.unpack(">H", s.recv(2))[0]
        elif fln == 127:
            fln = struct.unpack(">Q", s.recv(8))[0]
        data = b""
        while len(data) < fln:
            chunk = s.recv(fln - len(data))
            if not chunk:
                break
            data += chunk
        posts_ok(json.loads(data.decode()).get("data") or {}, "8.ws.posts")
    finally:
        s.close()


def main() -> int:
    parser = argparse.ArgumentParser(description="x-upt core protocol acceptance suite")
    parser.add_argument("--admin-url", default=os.environ.get("X_UPT_ADMIN_URL", ""))
    parser.add_argument("--port-base", type=int, default=None)
    args = parser.parse_args()

    workdir = tempfile.mkdtemp(prefix="x-upt-accept-")
    spawned: subprocess.Popen[bytes] | None = None
    admin_url = args.admin_url.rstrip("/")

    try:
        if not admin_url:
            root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
            bin_path = os.path.join(root, "target/release/x-upt")
            if not os.path.isfile(bin_path):
                bin_path = os.path.join(root, "target/debug/x-upt")
            admin_port = 28768
            admin_url = f"http://127.0.0.1:{admin_port}"
            spawned = subprocess.Popen(
                [bin_path, "serve", "--admin-listen", f"127.0.0.1:{admin_port}", "--data-dir", os.path.join(workdir, "data")],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            if not wait_http_ok(f"{admin_url}/health"):
                raise RuntimeError("failed to start temporary x-upt server")
            if args.port_base is None:
                args.port_base = 26000

        ports, cover_png, rsa_priv_path = seed_and_verify_preconditions(admin_url, args.port_base, workdir)
        test_nine_core_fronts(ports, cover_png, workdir)
        test_auth_and_logs(admin_url, ports["rest"], rsa_priv_path)
    finally:
        if spawned is not None:
            spawned.terminate()
            try:
                spawned.wait(timeout=8)
            except subprocess.TimeoutExpired:
                spawned.kill()
        shutil.rmtree(workdir, ignore_errors=True)

    failed = sum(1 for line in RESULTS if line.startswith("FAIL"))
    print(f"\n{failed} failed, {len(RESULTS)} total.", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
