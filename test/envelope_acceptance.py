#!/usr/bin/env python3
"""End-to-end envelope protocol acceptance suite for x-upt (6 envelope fronts via /api/v1/*)."""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

RESULTS: list[str] = []


def log(ok: bool, name: str, detail: str = "") -> None:
    line = ("PASS " if ok else "FAIL ") + name + (f" :: {detail}" if detail else "")
    RESULTS.append(line)
    print(line, flush=True)


def http(
    method: str,
    url: str,
    headers: dict[str, str] | None = None,
    data: bytes | None = None,
) -> tuple[int, bytes]:
    req = urllib.request.Request(url, data=data, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as err:
        return err.code, err.read()


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
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    st, raw = http(method, f"{admin_url}{path}", headers=hdrs, data=body)
    obj = json.loads(raw.decode("utf-8") or "{}")
    return st, obj if isinstance(obj, dict) else {"data": obj}


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


def openssl_cipher(alg: str, key: bytes, data: bytes, *, decrypt: bool = False, iv: bytes | None = None) -> bytes:
    cmd = ["openssl", "enc", f"-{alg}", "-K", key.hex()]
    if iv is not None:
        cmd.extend(["-iv", iv.hex()])
    if decrypt:
        cmd.append("-d")
    proc = subprocess.run(cmd, input=data, check=True, capture_output=True)
    return proc.stdout


def sm4_ecb(key: bytes, plain: bytes) -> bytes:
    return openssl_cipher("sm4-ecb", key, plain)


def sm4_ecb_dec(key: bytes, cipher: bytes) -> bytes:
    return openssl_cipher("sm4-ecb", key, cipher, decrypt=True)


def aes_ecb(key: bytes, plain: bytes) -> bytes:
    return openssl_cipher("aes-128-ecb", key, plain)


def aes_ecb_dec(key: bytes, cipher: bytes) -> bytes:
    return openssl_cipher("aes-128-ecb", key, cipher, decrypt=True)


def sm4_cbc(key: bytes, iv: bytes, plain: bytes) -> bytes:
    return openssl_cipher("sm4-cbc", key, plain, iv=iv)


def sm4_cbc_dec(key: bytes, iv: bytes, cipher: bytes) -> bytes:
    return openssl_cipher("sm4-cbc", key, cipher, decrypt=True, iv=iv)


def hmac_sm3(key: bytes, data: bytes) -> str:
    proc = subprocess.run(
        ["openssl", "dgst", "-sm3", "-mac", "HMAC", "-macopt", f"hexkey:{key.hex()}"],
        input=data,
        check=True,
        capture_output=True,
    )
    return proc.stdout.decode("ascii").strip().split()[-1]


def digest_of(obj: dict[str, object]) -> str:
    raw = json.dumps(obj, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def expect_digest(body: dict[str, object], biz: dict[str, object], name: str) -> None:
    ok = body.get("ok") is True and body.get("hit") is True and body.get("digest") == digest_of(biz)
    log(ok, name, json.dumps(body, ensure_ascii=False)[:220])


def seed_envelopes(
    admin_url: str,
    port_base: int | None,
) -> tuple[dict[str, int], dict[str, bytes], dict[str, str]]:
    _, status = admin_req(admin_url, "GET", "/api/v1/status")
    counts = status.get("counts") if isinstance(status.get("counts"), dict) else {}
    if counts.get("envelope_profiles") == 0:
        code_pre, err_pre = admin_req(admin_url, "POST", "/api/v1/fronts/envelope_token_plain/enable")
        log(code_pre == 409 and bool(err_pre.get("missing")), "env.precondition.block", f"status={code_pre}")

    text_secrets = {
        "issue_username": "env_user_alpha",
        "issue_password": "env_pass_alpha_99",
        "issue_id": "env_id_beta",
        "issue_secret": "env_sec_beta_88",
        "company_id": "acme_corp_01",
        "appcode": "APPCODE_X991",
        "cred_username": "des_caller_01",
    }
    raw_keys = {
        "sm4_key": os.urandom(16),
        "md5_key": b"md5_salt_secret_16",
        "aes_key": os.urandom(16),
        "hmac_key": os.urandom(32),
        "hmac_sm3_key": os.urandom(32),
        "des_key": os.urandom(8),
    }

    for name, txt in text_secrets.items():
        st, _ = admin_req(
            admin_url,
            "POST",
            "/api/v1/envelopes/secrets",
            {"name": name, "secret_kind": "text", "value": txt},
        )
        if st != 200:
            raise RuntimeError(f"failed to upload text secret {name}: {st}")

    for name, kb in raw_keys.items():
        st, _ = admin_req(
            admin_url,
            "POST",
            "/api/v1/envelopes/secrets",
            {"name": name, "secret_kind": "bytes_hex", "value": kb.hex()},
        )
        if st != 200:
            raise RuntimeError(f"failed to upload hex key {name}: {st}")

    profiles_by_front: dict[str, list[dict[str, object]]] = {
        "envelope_token_plain": [
            {
                "name": "issue",
                "kind": "issue",
                "methods": ["POST"],
                "path": "/issue",
                "body_kind": "form",
                "body_encoding": "identity",
                "fields": [
                    {"name": "username", "slot": "form", "role": "auth"},
                    {"name": "password", "slot": "form", "role": "auth"},
                ],
                "biz_rest": False,
                "token_response_field": "access_token",
                "secrets": {"username": "issue_username", "password": "issue_password"},
                "fail_in": "http",
            },
            {
                "name": "call",
                "kind": "call",
                "methods": ["POST"],
                "path": "/call",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [{"name": "authorization", "slot": "header", "role": "token"}],
                "biz_rest": True,
                "token_prefix": "Bearer ",
                "fail_in": "http",
            },
        ],
        "envelope_token_crypt": [
            {
                "name": "issue",
                "kind": "issue",
                "methods": ["POST"],
                "path": "/issue",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [
                    {"name": "id", "slot": "json", "role": "auth"},
                    {"name": "secret", "slot": "json", "role": "auth"},
                ],
                "biz_rest": False,
                "token_response_field": "access_token",
                "secrets": {"id": "issue_id", "secret": "issue_secret"},
                "fail_in": "http",
            },
            {
                "name": "call",
                "kind": "call",
                "methods": ["POST"],
                "path": "/call",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [
                    {"name": "authorization", "slot": "header", "role": "token"},
                    {"name": "data", "slot": "json", "role": "cipher"},
                ],
                "biz_rest": False,
                "token_prefix": "Bearer ",
                "crypt_alg": "sm4_ecb_pkcs7",
                "cipher_scope": "fields",
                "cipher_fields": ["data"],
                "crypt_encoding": "base64",
                "key": "sm4_key",
                "fail_in": "http",
            },
        ],
        "envelope_sign_plain": [
            {
                "name": "call",
                "kind": "call",
                "methods": ["POST"],
                "path": "/call",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [{"name": "sign", "slot": "json", "role": "sign"}],
                "biz_rest": True,
                "canon": "sorted_kv",
                "canon_sep": "&",
                "canon_suffix_key": True,
                "sign_alg": "md5",
                "hex_case": "lower",
                "sign_encoding": "hex",
                "sign_key": "md5_key",
                "fail_in": "http",
            }
        ],
        "envelope_sign_crypt": [
            {
                "name": "aes",
                "kind": "call",
                "methods": ["POST"],
                "path": "/aes",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [
                    {"name": "payload", "slot": "json", "role": "cipher"},
                    {"name": "sign", "slot": "json", "role": "sign"},
                ],
                "biz_rest": False,
                "canon": "raw",
                "canon_fields": ["payload"],
                "sign_alg": "hmac_sha256",
                "hex_case": "lower",
                "sign_encoding": "hex",
                "sign_key": "hmac_key",
                "crypt_alg": "aes_ecb_pkcs5",
                "cipher_scope": "fields",
                "cipher_fields": ["payload"],
                "crypt_encoding": "base64",
                "key": "aes_key",
                "order": "sign_then_encrypt",
                "fail_in": "envelope",
            },
            {
                "name": "sm4",
                "kind": "call",
                "methods": ["POST"],
                "path": "/sm4",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [
                    {"name": "payload", "slot": "json", "role": "cipher"},
                    {"name": "sign", "slot": "json", "role": "sign"},
                ],
                "biz_rest": False,
                "canon": "raw",
                "sign_alg": "hmac_sm3",
                "hex_case": "lower",
                "sign_encoding": "hex",
                "sign_key": "hmac_sm3_key",
                "crypt_alg": "sm4_cbc_pkcs7",
                "cipher_scope": "fields",
                "cipher_fields": ["payload"],
                "crypt_encoding": "base64",
                "iv_from": "random_prefix",
                "key": "sm4_key",
                "order": "encrypt_then_sign",
                "fail_in": "http",
            },
        ],
        "envelope_cred_plain": [
            {
                "name": "query_call",
                "kind": "call",
                "methods": ["GET"],
                "path": "/call",
                "body_kind": "empty",
                "body_encoding": "identity",
                "fields": [{"name": "company_id", "slot": "query", "role": "auth"}],
                "biz_rest": True,
                "secrets": {"company_id": "company_id"},
                "fail_in": "http",
            },
            {
                "name": "header_call",
                "kind": "call",
                "methods": ["POST"],
                "path": "/header",
                "body_kind": "json",
                "body_encoding": "identity",
                "fields": [{"name": "appcode", "slot": "header", "role": "auth"}],
                "biz_rest": True,
                "secrets": {"appcode": "appcode"},
                "fail_in": "http",
            },
        ],
        "envelope_cred_crypt": [
            {
                "name": "des_call",
                "kind": "call",
                "methods": ["POST"],
                "path": "/call",
                "body_kind": "form",
                "body_encoding": "identity",
                "fields": [
                    {"name": "username", "slot": "form", "role": "auth"},
                    {"name": "params", "slot": "form", "role": "cipher"},
                ],
                "biz_rest": False,
                "secrets": {"username": "cred_username"},
                "crypt_alg": "des_cbc_pkcs5",
                "cipher_scope": "fields",
                "cipher_fields": ["params"],
                "crypt_encoding": "base64",
                "iv_from": "key",
                "key": "des_key",
                "fail_in": "http",
                "response_crypt_alg": "none",
            }
        ],
    }

    _, fronts_resp = admin_req(admin_url, "GET", "/api/v1/fronts")
    front_items = {f["name"]: f for f in fronts_resp.get("items", []) if isinstance(f, dict)}
    ports: dict[str, int] = {}

    for idx, (fname, specs) in enumerate(profiles_by_front.items()):
        st_p, _ = admin_req(admin_url, "PUT", f"/api/v1/envelopes/{fname}/profiles", {"profiles": specs})
        if st_p != 200:
            raise RuntimeError(f"replace profiles failed for {fname}: {st_p}")
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
                    "http2": False,
                    "h2c": False,
                    "expire_minutes": 30,
                    "auth": [],
                    "tls_enabled": False,
                    "ws_max_conn": 1024,
                    "ws_ping_secs": 30,
                    "job_workers": 4,
                },
            )
        st_en, view = admin_req(admin_url, "POST", f"/api/v1/fronts/{fname}/enable")
        ports[fname] = int(view.get("port", port))
        log(st_en == 200 and wait_port(ports[fname]), f"env.bind.{fname}", f"port={ports[fname]}")

    # Active-dependency lock check: deleting sm4_key while envelope_token_crypt is active MUST return 409.
    del_code, _ = admin_req(admin_url, "DELETE", "/api/v1/envelopes/secrets/sm4_key")
    log(del_code == 409, "env.lock.active_secret_409", f"status={del_code}")

    return ports, raw_keys, text_secrets


def run_assertions(ports: dict[str, int], raw_keys: dict[str, bytes], txt: dict[str, str]) -> None:
    # 1. envelope_token_plain
    p0 = ports["envelope_token_plain"]
    st, raw = http(
        "POST",
        f"http://127.0.0.1:{p0}/issue",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": txt["issue_username"], "password": "wrong"}).encode(),
    )
    log(st == 401 and b"bad_credential" in raw, "token-plain-fail")
    st, raw = http(
        "POST",
        f"http://127.0.0.1:{p0}/issue",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": txt["issue_username"], "password": txt["issue_password"]}).encode(),
    )
    tok = json.loads(raw)["access_token"]
    _, raw = http(
        "POST",
        f"http://127.0.0.1:{p0}/call",
        {"Content-Type": "application/json", "Authorization": f"Bearer {tok}"},
        b'{"city":"hz"}',
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "token-plain-ok")

    # 2. envelope_token_crypt
    p1 = ports["envelope_token_crypt"]
    st, raw = http(
        "POST",
        f"http://127.0.0.1:{p1}/issue",
        {"Content-Type": "application/json"},
        json.dumps({"id": txt["issue_id"], "secret": "wrong"}).encode(),
    )
    log(st == 401 and b"bad_credential" in raw, "token-crypt-fail")
    _, raw = http(
        "POST",
        f"http://127.0.0.1:{p1}/issue",
        {"Content-Type": "application/json"},
        json.dumps({"id": txt["issue_id"], "secret": txt["issue_secret"]}).encode(),
    )
    tok_c = json.loads(raw)["access_token"]
    sm4_k = raw_keys["sm4_key"]
    enc_b64 = base64.b64encode(sm4_ecb(sm4_k, b'{"city":"hz"}')).decode()
    _, raw = http(
        "POST",
        f"http://127.0.0.1:{p1}/call",
        {"Content-Type": "application/json", "Authorization": f"Bearer {tok_c}"},
        json.dumps({"data": enc_b64}).encode(),
    )
    outer = json.loads(raw)
    inner = json.loads(sm4_ecb_dec(sm4_k, base64.b64decode(outer["data"])))
    expect_digest(inner, {"city": "hz"}, "token-crypt-ok")

    # 3. envelope_sign_plain
    p2 = ports["envelope_sign_plain"]
    md5_sig = hashlib.md5(b"city=hz&" + raw_keys["md5_key"]).hexdigest()
    st, raw = http("POST", f"http://127.0.0.1:{p2}/call", {"Content-Type": "application/json"}, b'{"city":"hz","sign":"00"}')
    log(st == 401 and b"bad_sign" in raw, "sign-plain-fail")
    _, raw = http(
        "POST",
        f"http://127.0.0.1:{p2}/call",
        {"Content-Type": "application/json"},
        json.dumps({"city": "hz", "sign": md5_sig}).encode(),
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "sign-plain-ok")

    # 4. envelope_sign_crypt (AES fail_in=envelope + SM4-CBC + HMAC-SM3)
    p3 = ports["envelope_sign_crypt"]
    aes_k, mac_k = raw_keys["aes_key"], raw_keys["hmac_key"]
    aes_payload = base64.b64encode(aes_ecb(aes_k, b'{"city":"hz"}')).decode()
    aes_sig = hmac.new(mac_k, aes_payload.encode(), hashlib.sha256).hexdigest()
    st, raw = http(
        "POST",
        f"http://127.0.0.1:{p3}/aes",
        {"Content-Type": "application/json"},
        json.dumps({"payload": aes_payload, "sign": "00"}).encode(),
    )
    fail_inner = json.loads(aes_ecb_dec(aes_k, base64.b64decode(json.loads(raw)["payload"])))
    log(st == 200 and fail_inner.get("reason") == "bad_sign", "sign-crypt-aes-fail")
    _, raw = http(
        "POST",
        f"http://127.0.0.1:{p3}/aes",
        {"Content-Type": "application/json"},
        json.dumps({"payload": aes_payload, "sign": aes_sig}).encode(),
    )
    ok_inner = json.loads(aes_ecb_dec(aes_k, base64.b64decode(json.loads(raw)["payload"])))
    expect_digest(ok_inner, {"city": "hz"}, "sign-crypt-aes-ok")

    iv = os.urandom(16)
    plain = b'{"city":"hz"}'
    sm4_blob = base64.b64encode(iv + sm4_cbc(sm4_k, iv, plain)).decode()
    sm3_sig = hmac_sm3(raw_keys["hmac_sm3_key"], plain)
    _, raw = http(
        "POST",
        f"http://127.0.0.1:{p3}/sm4",
        {"Content-Type": "application/json"},
        json.dumps({"payload": sm4_blob, "sign": sm3_sig}).encode(),
    )
    packed = base64.b64decode(json.loads(raw)["payload"])
    expect_digest(json.loads(sm4_cbc_dec(sm4_k, packed[:16], packed[16:])), {"city": "hz"}, "sign-crypt-sm4-ok")

    # 5. envelope_cred_plain
    p4 = ports["envelope_cred_plain"]
    st, raw = http("GET", f"http://127.0.0.1:{p4}/call?company_id=wrong&city=hz")
    log(st == 401 and b"bad_credential" in raw, "cred-plain-fail")
    _, raw = http("GET", f"http://127.0.0.1:{p4}/call?" + urllib.parse.urlencode({"company_id": txt["company_id"], "city": "hz"}))
    expect_digest(json.loads(raw), {"city": "hz"}, "cred-plain-ok")
    _, raw = http("POST", f"http://127.0.0.1:{p4}/header", {"Content-Type": "application/json", "AppCode": txt["appcode"]}, b'{"city":"hz"}')
    expect_digest(json.loads(raw), {"city": "hz"}, "cred-header-ok")

    # 6. envelope_cred_crypt (DES-CBC via openssl)
    p5 = ports["envelope_cred_crypt"]
    des_k = raw_keys["des_key"]
    proc = subprocess.run(
        ["openssl", "enc", "-des-cbc", "-provider", "legacy", "-provider", "default", "-K", des_k.hex(), "-iv", des_k.hex(), "-nosalt"],
        input=b'{"city":"hz"}',
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        log(False, "cred-crypt-ok", proc.stderr.decode()[:180])
        return
    params_b64 = base64.b64encode(proc.stdout).decode()
    st, raw = http(
        "POST",
        f"http://127.0.0.1:{p5}/call",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": "wrong", "params": params_b64}).encode(),
    )
    log(st == 401 and b"bad_credential" in raw, "cred-crypt-fail")
    st, raw = http(
        "POST",
        f"http://127.0.0.1:{p5}/call",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": txt["cred_username"], "params": params_b64}).encode(),
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "cred-crypt-ok")


def main() -> int:
    parser = argparse.ArgumentParser(description="x-upt envelope protocol acceptance suite")
    parser.add_argument("--admin-url", default=os.environ.get("X_UPT_ADMIN_URL", ""))
    parser.add_argument("--port-base", type=int, default=None)
    args = parser.parse_args()

    workdir = tempfile.mkdtemp(prefix="x-upt-env-accept-")
    spawned: subprocess.Popen[bytes] | None = None
    admin_url = args.admin_url.rstrip("/")

    try:
        if not admin_url:
            root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
            bin_path = os.path.join(root, "target/release/x-upt")
            if not os.path.isfile(bin_path):
                bin_path = os.path.join(root, "target/debug/x-upt")
            admin_port = 28769
            admin_url = f"http://127.0.0.1:{admin_port}"
            spawned = subprocess.Popen(
                [bin_path, "serve", "--admin-listen", f"127.0.0.1:{admin_port}", "--data-dir", os.path.join(workdir, "data")],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            deadline = time.time() + 15
            while time.time() < deadline:
                try:
                    st, _ = http("GET", f"{admin_url}/health")
                    if st == 200:
                        break
                except Exception:
                    pass
                time.sleep(0.15)
            if args.port_base is None:
                args.port_base = 26010

        ports, raw_keys, txt = seed_envelopes(admin_url, args.port_base)
        run_assertions(ports, raw_keys, txt)
    finally:
        if spawned is not None:
            spawned.terminate()
            try:
                spawned.wait(timeout=8)
            except subprocess.TimeoutExpired:
                spawned.kill()
        shutil.rmtree(workdir, ignore_errors=True)

    failed = [line for line in RESULTS if line.startswith("FAIL")]
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
