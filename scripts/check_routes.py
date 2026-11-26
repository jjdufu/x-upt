#!/usr/bin/env python3
"""G1 guard: Bidirectional parity between crates/x-upt router and admin-openapi.json, plus upstream isolation."""

from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OPENAPI = ROOT / "crates/x-upt/admin-openapi.json"
API_DIR = ROOT / "crates/x-upt/src"
FRONTS_DIR = ROOT / "crates/x-upt-fronts/src"

ROUTE_CALL_RE = re.compile(r'\.route\(\s*"([^"]+)"\s*,\s*([^()]+(?:\([^()]*\)[^()]*)*)\)', re.DOTALL)
METHOD_RE = re.compile(r"\b(get|post|put|delete|patch)\s*\(")


def extract_router_routes() -> set[tuple[str, str]]:
    found: set[tuple[str, str]] = set()
    for rs in sorted(API_DIR.rglob("*.rs")):
        text = rs.read_text(encoding="utf-8")
        for m in ROUTE_CALL_RE.finditer(text):
            path = m.group(1)
            chain = m.group(2)
            for mm in METHOD_RE.finditer(chain):
                found.add((mm.group(1).upper(), path))
    return found


def extract_openapi_routes() -> set[tuple[str, str]]:
    spec = json.loads(OPENAPI.read_text(encoding="utf-8"))
    found: set[tuple[str, str]] = set()
    for path, ops in spec.get("paths", {}).items():
        for method in ops:
            if method.lower() in {"get", "post", "put", "delete", "patch"}:
                found.add((method.upper(), path))
    return found


def check_upstream_isolation() -> list[str]:
    errors: list[str] = []
    for rs in sorted(FRONTS_DIR.rglob("*.rs")):
        text = rs.read_text(encoding="utf-8")
        for m in ROUTE_CALL_RE.finditer(text):
            path = m.group(1)
            if path == "/health" or path.startswith("/api/v1"):
                errors.append(f"{rs.relative_to(ROOT)} leaks control-plane path {path!r} onto upstream front")
    return errors


def main() -> int:
    if not OPENAPI.is_file():
        print("ROUTES-GUARD FAIL: missing crates/x-upt/admin-openapi.json", file=sys.stderr)
        return 1

    router_routes = extract_router_routes()
    spec_routes = extract_openapi_routes()
    errors = check_upstream_isolation()

    undocumented = sorted(router_routes - spec_routes)
    unimplemented = sorted(spec_routes - router_routes)
    for method, path in undocumented:
        errors.append(f"route in axum::Router missing from admin-openapi.json: {method} {path}")
    for method, path in unimplemented:
        errors.append(f"route in admin-openapi.json missing from axum::Router: {method} {path}")

    if errors:
        for err in errors:
            print(f"ROUTES-GUARD FAIL: {err}", file=sys.stderr)
        return 1
    print(f"ROUTES-GUARD PASS ({len(spec_routes)} operations)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
