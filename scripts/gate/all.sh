#!/usr/bin/env bash
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${PATH}"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

echo "=== [G1] Static Guards & Release Script Self-Test ==="
python3 scripts/lint-guard.py
python3 scripts/web-guard.py
python3 scripts/docs-and-plans-guard.py
python3 scripts/check_routes.py
bash .github/scripts/release-test.sh

if command -v shellcheck >/dev/null 2>&1; then
    find scripts .github/scripts -name '*.sh' -print0 | xargs -0 shellcheck
fi

echo "=== [G2] Frontend & Rust Build / Lint / Test / Doc Audit ==="
pnpm --dir web install --frozen-lockfile
pnpm --dir web lint
pnpm --dir web typecheck
pnpm --dir web test
pnpm --dir web build
python3 scripts/embed-guard.py

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo build --release --locked
test -x "$ROOT/target/release/x-upt"

echo "=== [G3] Isolated Single-Binary End-to-End Black-Box Verification ==="
TMP_WORK="$(mktemp -d -t x-upt-g3-XXXXXX)"
SERVER_PID=""
cleanup() {
    if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill -TERM "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP_WORK"
}
trap cleanup EXIT

cp "$ROOT/target/release/x-upt" "$TMP_WORK/x-upt"
chmod +x "$TMP_WORK/x-upt"

read -r E2E_PORT ADMIN_PORT CORE_BASE ENV_BASE < <(python3 - <<'PY'
import socket

def free_block(count: int) -> int:
    for base in range(31000, 48000, 50):
        socks = []
        ok = True
        for offset in range(count):
            s = socket.socket()
            try:
                s.bind(("127.0.0.1", base + offset))
                socks.append(s)
            except OSError:
                ok = False
                break
        for s in socks:
            s.close()
        if ok:
            return base
    raise RuntimeError("no free port block")

b1 = free_block(20)
print(b1, b1 + 1, b1 + 2, b1 + 12)
PY
)

# 1. Verify `x-upt check` in isolated empty directory
(
    cd "$TMP_WORK"
    ./x-upt check --data-dir "$TMP_WORK/e2e-data"
)

# 2. Launch isolated instance #1 for Playwright E2E + SPA/MIME/Cache/404 boundary checks
(
    cd "$TMP_WORK"
    exec ./x-upt serve --admin-listen "127.0.0.1:${E2E_PORT}" --data-dir "$TMP_WORK/e2e-data" >"$TMP_WORK/e2e.log" 2>&1
) &
SERVER_PID="$!"

python3 - "http://127.0.0.1:${E2E_PORT}" <<'PY'
import re, sys, time, urllib.error, urllib.request

base = sys.argv[1]
deadline = time.time() + 15
while time.time() < deadline:
    try:
        with urllib.request.urlopen(f"{base}/health", timeout=1.5) as r:
            if r.status == 200:
                break
    except Exception:
        time.sleep(0.15)
else:
    raise SystemExit("E2E server failed /health readiness")

# Verify SPA index.html no-cache + ETag + 304 Not Modified
with urllib.request.urlopen(f"{base}/fronts", timeout=5) as r:
    assert r.status == 200
    assert "text/html" in (r.headers.get("Content-Type") or "")
    assert "no-cache" in (r.headers.get("Cache-Control") or "")
    etag = r.headers.get("ETag")
    assert etag, "missing ETag on index.html"
    html = r.read().decode("utf-8")

req_304 = urllib.request.Request(f"{base}/fronts", headers={"If-None-Match": etag})
try:
    urllib.request.urlopen(req_304, timeout=5)
    raise AssertionError("expected 304 Not Modified")
except urllib.error.HTTPError as e:
    assert e.code == 304, f"expected 304, got {e.code}"

# Verify fingerprinted /assets/*.js immutable + nosniff
m = re.search(r'src="(/assets/[^"]+\.js)"', html)
assert m, "missing fingerprinted /assets/*.js in index.html"
with urllib.request.urlopen(f"{base}{m.group(1)}", timeout=5) as r:
    assert r.status == 200
    assert "javascript" in (r.headers.get("Content-Type") or "")
    assert "immutable" in (r.headers.get("Cache-Control") or "")
    assert r.headers.get("X-Content-Type-Options") == "nosniff"

# Verify missing /assets/* -> 404 (not SPA fallback) & /api/* -> 404 JSON
for bad_path, want_json in (("/assets/missing-chunk.js", False), ("/api/v1/unknown-route", True)):
    try:
        urllib.request.urlopen(f"{base}{bad_path}", timeout=5)
        raise AssertionError(f"expected 404 for {bad_path}")
    except urllib.error.HTTPError as e:
        assert e.code == 404, f"expected 404 for {bad_path}, got {e.code}"
        if want_json:
            assert "application/json" in (e.headers.get("Content-Type") or "")
print("SPA-BOUNDARY PASS")
PY

X_UPT_BASE_URL="http://127.0.0.1:${E2E_PORT}" X_UPT_E2E_REST_PORT="$((E2E_PORT + 19))" pnpm --dir "$ROOT/web" test:e2e

kill -TERM "$SERVER_PID"
wait "$SERVER_PID"
SERVER_PID=""

# 3. Launch isolated instance #2 for full 15-front protocol acceptance, hot-disable & restart persistence
(
    cd "$TMP_WORK"
    exec ./x-upt serve --admin-listen "127.0.0.1:${ADMIN_PORT}" --data-dir "$TMP_WORK/full-data" >"$TMP_WORK/full.log" 2>&1
) &
SERVER_PID="$!"

python3 - "http://127.0.0.1:${ADMIN_PORT}" <<'PY'
import sys, time, urllib.request
base = sys.argv[1]
for _ in range(100):
    try:
        with urllib.request.urlopen(f"{base}/health", timeout=1.5) as r:
            if r.status == 200:
                sys.exit(0)
    except Exception:
        time.sleep(0.15)
sys.exit("Full acceptance server failed /health")
PY

python3 "$ROOT/test/acceptance.py" --admin-url "http://127.0.0.1:${ADMIN_PORT}" --port-base "${CORE_BASE}"
python3 "$ROOT/test/envelope_acceptance.py" --admin-url "http://127.0.0.1:${ADMIN_PORT}" --port-base "${ENV_BASE}"

# 4. Verify hot-disable immediately closes & releases TCP port, then re-enable restores it
python3 - "http://127.0.0.1:${ADMIN_PORT}" <<'PY'
import json, socket, sys, time, urllib.parse, urllib.request

admin = sys.argv[1]
parsed = urllib.parse.urlparse(admin)
hdrs = {"Accept": "application/json", "Host": parsed.netloc, "Origin": f"{parsed.scheme}://{parsed.netloc}"}

def call(method: str, path: str) -> dict:
    req = urllib.request.Request(f"{admin}{path}", method=method, headers=hdrs)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode("utf-8"))

off = call("POST", "/api/v1/fronts/soap/disable")
soap_port = int(off["port"])
assert off["enabled"] is False and off["running"] is False

closed = False
for _ in range(20):
    s = socket.socket()
    s.settimeout(0.2)
    try:
        s.connect(("127.0.0.1", soap_port))
        s.close()
        time.sleep(0.1)
    except OSError:
        closed = True
        break
    finally:
        s.close()
assert closed, f"soap port {soap_port} still open after disable"

on = call("POST", "/api/v1/fronts/soap/enable")
assert on["enabled"] is True and on["running"] is True
print("HOT-REBIND PASS")
PY

# 5. Verify SIGTERM clean exit (<= 10s, exit code 0) + SQLite restart persistence
T_STOP_START="$(date +%s)"
kill -TERM "$SERVER_PID"
set +e
wait "$SERVER_PID"
EXIT_CODE="$?"
set -e
SERVER_PID=""
T_STOP_ELAPSED="$(( $(date +%s) - T_STOP_START ))"
if [[ "$EXIT_CODE" -ne 0 || "$T_STOP_ELAPSED" -gt 10 ]]; then
    echo "SIGTERM check failed: exit=${EXIT_CODE} elapsed=${T_STOP_ELAPSED}s" >&2
    exit 1
fi
echo "SIGTERM-DRAIN PASS (${T_STOP_ELAPSED}s, exit 0)"

(
    cd "$TMP_WORK"
    exec ./x-upt serve --admin-listen "127.0.0.1:${ADMIN_PORT}" --data-dir "$TMP_WORK/full-data" >>"$TMP_WORK/full.log" 2>&1
) &
SERVER_PID="$!"

python3 - "http://127.0.0.1:${ADMIN_PORT}" "$((CORE_BASE + 2))" <<'PY'
import json, sys, time, urllib.request

admin, rest_port = sys.argv[1], int(sys.argv[2])
for _ in range(100):
    try:
        with urllib.request.urlopen(f"{admin}/health", timeout=1.5) as r:
            if r.status == 200:
                break
    except Exception:
        time.sleep(0.15)
else:
    raise SystemExit("restarted server failed /health")

with urllib.request.urlopen(f"{admin}/api/v1/status", timeout=5) as r:
    st = json.loads(r.read().decode("utf-8"))

assert st["counts"]["images"] == 3
assert st["counts"]["posts"] == 2
assert all(m["enabled"] for m in st["methods"]), "methods not persisted"
assert all(f["enabled"] and f["running"] for f in st["fronts"]), "fronts not restored on restart"

req = urllib.request.Request(
    f"http://127.0.0.1:{rest_port}/posts/search",
    data=b'{"keyword":"gRPC","user_id":6,"page":1,"page_size":10,"sort":"id_desc"}',
    headers={"Content-Type": "application/json", "X-API-Key": "DemoSecret_9xK2mQp8Lz4Vw7"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=5) as r:
    out = json.loads(r.read().decode("utf-8"))
    assert out["total"] == 1
print("RESTART-PERSISTENCE PASS")
PY

kill -TERM "$SERVER_PID"
wait "$SERVER_PID"
SERVER_PID=""

echo "ALL GATES PASS"
