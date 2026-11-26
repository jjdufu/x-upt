#!/usr/bin/env python3
"""G2 guard: Embedded web/dist purity, size budget, and build.rs freshness lock."""

from __future__ import annotations

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DIST = ROOT / "web/dist"
BUILD_RS = ROOT / "crates/x-upt/build.rs"
MAX_SINGLE_BYTES = 5 * 1024 * 1024
MAX_TOTAL_BYTES = 15 * 1024 * 1024


def main() -> int:
    errors: list[str] = []
    index_html = DIST / "index.html"
    if not index_html.is_file() or index_html.stat().st_size == 0:
        errors.append("missing or empty web/dist/index.html")
    else:
        html = index_html.read_text(encoding="utf-8")
        if "/assets/" not in html:
            errors.append("web/dist/index.html does not reference fingerprinted /assets/ bundle")

    total = 0
    if DIST.is_dir():
        for p in sorted(DIST.rglob("*")):
            if not p.is_file():
                continue
            rel = p.relative_to(DIST)
            size = p.stat().st_size
            total += size
            if p.suffix == ".map":
                errors.append(f"forbidden sourcemap in web/dist: {rel}")
            if p.name.startswith(".env"):
                errors.append(f"forbidden .env file in web/dist: {rel}")
            if "node_modules" in rel.parts:
                errors.append(f"forbidden node_modules fragment in web/dist: {rel}")
            if size > MAX_SINGLE_BYTES:
                errors.append(f"asset exceeds {MAX_SINGLE_BYTES} bytes: {rel} ({size})")

    if total > MAX_TOTAL_BYTES:
        errors.append(f"total web/dist size {total} exceeds {MAX_TOTAL_BYTES} bytes")

    if not BUILD_RS.is_file():
        errors.append("missing crates/x-upt/build.rs")
    else:
        build_src = BUILD_RS.read_text(encoding="utf-8")
        if "cargo:rerun-if-changed=../../web/dist" not in build_src:
            errors.append("crates/x-upt/build.rs missing rerun-if-changed for web/dist")
        if "frontend_embedded" not in build_src:
            errors.append("crates/x-upt/build.rs missing frontend_embedded cfg gate")

    if errors:
        for err in errors:
            print(f"EMBED-GUARD FAIL: {err}", file=sys.stderr)
        return 1
    print(f"EMBED-GUARD PASS ({total} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
