#!/usr/bin/env python3
"""Hit envelope ports 3010-3015. Keys come from config/keys/envelope."""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import subprocess
import urllib.parse
import urllib.request

from cryptography.hazmat.primitives import hashes, padding
from cryptography.hazmat.primitives.hmac import HMAC
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
KEYS = os.path.join(ROOT, "config/keys/envelope")
RESULTS: list[str] = []


def log(ok: bool, name: str, detail: str = "") -> None:
    line = ("PASS " if ok else "FAIL ") + name + (f" :: {detail}" if detail else "")
    RESULTS.append(line)
    print(line, flush=True)


def read_bytes(name: str) -> bytes:
    with open(os.path.join(KEYS, name), "rb") as handle:
        return handle.read()


def read_line(name: str) -> str:
    return read_bytes(name).decode().rstrip("\r\n")


def http(method: str, url: str, headers: dict | None = None, data: bytes | None = None):
    req = urllib.request.Request(url, data=data, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as err:
        return err.code, err.read()


def pkcs7(data: bytes, block: int) -> bytes:
    padder = padding.PKCS7(block * 8).padder()
    return padder.update(data) + padder.finalize()


def unpad(data: bytes, block: int) -> bytes:
    unpadder = padding.PKCS7(block * 8).unpadder()
    return unpadder.update(data) + unpadder.finalize()


def sm4_ecb(key: bytes, plain: bytes) -> bytes:
    enc = Cipher(algorithms.SM4(key), modes.ECB()).encryptor()
    raw = pkcs7(plain, 16)
    return enc.update(raw) + enc.finalize()


def sm4_ecb_dec(key: bytes, cipher: bytes) -> bytes:
    dec = Cipher(algorithms.SM4(key), modes.ECB()).decryptor()
    return unpad(dec.update(cipher) + dec.finalize(), 16)


def aes_ecb(key: bytes, plain: bytes) -> bytes:
    enc = Cipher(algorithms.AES(key), modes.ECB()).encryptor()
    return enc.update(pkcs7(plain, 16)) + enc.finalize()


def aes_ecb_dec(key: bytes, cipher: bytes) -> bytes:
    dec = Cipher(algorithms.AES(key), modes.ECB()).decryptor()
    return unpad(dec.update(cipher) + dec.finalize(), 16)


def sm4_cbc(key: bytes, iv: bytes, plain: bytes) -> bytes:
    enc = Cipher(algorithms.SM4(key), modes.CBC(iv)).encryptor()
    return enc.update(pkcs7(plain, 16)) + enc.finalize()


def sm4_cbc_dec(key: bytes, iv: bytes, cipher: bytes) -> bytes:
    dec = Cipher(algorithms.SM4(key), modes.CBC(iv)).decryptor()
    return unpad(dec.update(cipher) + dec.finalize(), 16)


def hmac_sm3(key: bytes, data: bytes) -> str:
    mac = HMAC(key, hashes.SM3())
    mac.update(data)
    return mac.finalize().hex()


def digest_of(obj: dict) -> str:
    raw = json.dumps(obj, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()
    return hashlib.sha256(raw).hexdigest()


def expect_digest(body: dict, biz: dict, name: str) -> None:
    ok = body.get("ok") is True and body.get("hit") is True and body.get("digest") == digest_of(biz)
    log(ok, name, json.dumps(body, ensure_ascii=False)[:240])


def token_plain() -> None:
    user = read_line("issue_username")
    password = read_line("issue_password")
    status, raw = http(
        "POST",
        "http://127.0.0.1:3010/issue",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": user, "password": "nope"}).encode(),
    )
    log(status == 401 and b"bad_credential" in raw, "token-plain-fail", raw[:120].decode())
    status, raw = http(
        "POST",
        "http://127.0.0.1:3010/issue",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": user, "password": password}).encode(),
    )
    token = json.loads(raw)["access_token"]
    status, raw = http(
        "POST",
        "http://127.0.0.1:3010/call",
        {"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        b'{"city":"hz"}',
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "token-plain-ok")


def token_crypt() -> None:
    ident = read_line("issue_id")
    secret = read_line("issue_secret")
    status, raw = http(
        "POST",
        "http://127.0.0.1:3011/issue",
        {"Content-Type": "application/json"},
        json.dumps({"id": ident, "secret": "nope"}).encode(),
    )
    log(status == 401 and b"bad_credential" in raw, "token-crypt-fail", raw[:120].decode())
    status, raw = http(
        "POST",
        "http://127.0.0.1:3011/issue",
        {"Content-Type": "application/json"},
        json.dumps({"id": ident, "secret": secret}).encode(),
    )
    token = json.loads(raw)["access_token"]
    key = read_bytes("sm4.key")
    payload = base64.b64encode(sm4_ecb(key, b'{"city":"hz"}')).decode()
    status, raw = http(
        "POST",
        "http://127.0.0.1:3011/call",
        {"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        json.dumps({"data": payload}).encode(),
    )
    outer = json.loads(raw)
    inner = json.loads(sm4_ecb_dec(key, base64.b64decode(outer["data"])))
    expect_digest(inner, {"city": "hz"}, "token-crypt-ok")


def sign_plain() -> None:
    key = read_bytes("md5.key")
    material = b"city=hz&" + key
    sign = hashlib.md5(material).hexdigest()
    status, raw = http(
        "POST",
        "http://127.0.0.1:3012/call",
        {"Content-Type": "application/json"},
        json.dumps({"city": "hz", "sign": "00"}).encode(),
    )
    log(status == 401 and b"bad_sign" in raw, "sign-plain-fail", raw[:120].decode())
    status, raw = http(
        "POST",
        "http://127.0.0.1:3012/call",
        {"Content-Type": "application/json"},
        json.dumps({"city": "hz", "sign": sign}).encode(),
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "sign-plain-ok")


def sign_crypt() -> None:
    aes_key = read_bytes("aes.key")
    mac_key = read_bytes("hmac.key")
    payload = base64.b64encode(aes_ecb(aes_key, b'{"city":"hz"}')).decode()
    sign = hmac.new(mac_key, payload.encode(), hashlib.sha256).hexdigest()
    status, raw = http(
        "POST",
        "http://127.0.0.1:3013/aes",
        {"Content-Type": "application/json"},
        json.dumps({"payload": payload, "sign": "00"}).encode(),
    )
    failed = json.loads(raw)
    inner_fail = json.loads(aes_ecb_dec(aes_key, base64.b64decode(failed["payload"])))
    log(
        status == 200 and inner_fail.get("reason") == "bad_sign",
        "sign-crypt-aes-fail",
        json.dumps(inner_fail, ensure_ascii=False)[:160],
    )
    status, raw = http(
        "POST",
        "http://127.0.0.1:3013/aes",
        {"Content-Type": "application/json"},
        json.dumps({"payload": payload, "sign": sign}).encode(),
    )
    outer = json.loads(raw)
    inner = json.loads(aes_ecb_dec(aes_key, base64.b64decode(outer["payload"])))
    expect_digest(inner, {"city": "hz"}, "sign-crypt-aes-ok")

    sm4_key = read_bytes("sm4.key")
    sm3_key = read_bytes("hmac_sm3.key")
    plain = b'{"city":"hz"}'
    iv = os.urandom(16)
    blob = base64.b64encode(iv + sm4_cbc(sm4_key, iv, plain)).decode()
    sign = hmac_sm3(sm3_key, plain)
    status, raw = http(
        "POST",
        "http://127.0.0.1:3013/sm4",
        {"Content-Type": "application/json"},
        json.dumps({"payload": blob, "sign": sign}).encode(),
    )
    outer = json.loads(raw)
    packed = base64.b64decode(outer["payload"])
    inner = json.loads(sm4_cbc_dec(sm4_key, packed[:16], packed[16:]))
    expect_digest(inner, {"city": "hz"}, "sign-crypt-sm4-ok")


def cred_plain() -> None:
    company = read_line("company_id")
    status, raw = http("GET", "http://127.0.0.1:3014/call?company_id=nope&city=hz")
    log(status == 401 and b"bad_credential" in raw, "cred-plain-fail", raw[:120].decode())
    status, raw = http(
        "GET",
        "http://127.0.0.1:3014/call?" + urllib.parse.urlencode({"company_id": company, "city": "hz"}),
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "cred-plain-ok")
    appcode = read_line("appcode")
    status, raw = http(
        "POST",
        "http://127.0.0.1:3014/header",
        {"Content-Type": "application/json", "AppCode": appcode},
        b'{"city":"hz"}',
    )
    expect_digest(json.loads(raw), {"city": "hz"}, "cred-header-ok")


def des_crypt() -> None:
    user = read_line("cred_username")
    key = read_bytes("des.key")
    proc = subprocess.run(
        [
            "openssl",
            "enc",
            "-des-cbc",
            "-provider",
            "legacy",
            "-provider",
            "default",
            "-K",
            key.hex(),
            "-iv",
            key.hex(),
            "-nosalt",
        ],
        input=b'{"city":"hz"}',
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        log(False, "cred-crypt-ok", proc.stderr.decode()[:200])
        return
    params = base64.b64encode(proc.stdout).decode()
    status, raw = http(
        "POST",
        "http://127.0.0.1:3015/call",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": "nope", "params": params}).encode(),
    )
    log(status == 401 and b"bad_credential" in raw, "cred-crypt-fail", raw[:120].decode())
    status, raw = http(
        "POST",
        "http://127.0.0.1:3015/call",
        {"Content-Type": "application/x-www-form-urlencoded"},
        urllib.parse.urlencode({"username": user, "params": params}).encode(),
    )
    body = json.loads(raw)
    visible = b'"hit"' in raw
    log(status == 200 and visible and body.get("digest") == digest_of({"city": "hz"}), "cred-crypt-ok", raw[:180].decode())


def main() -> None:
    token_plain()
    token_crypt()
    sign_plain()
    sign_crypt()
    cred_plain()
    des_crypt()
    failed = [line for line in RESULTS if line.startswith("FAIL")]
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
