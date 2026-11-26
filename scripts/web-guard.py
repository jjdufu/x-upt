#!/usr/bin/env python3
"""G1 guard: TypeScript strictness, zero any/ts-ignore/eslint-disable, zero CDN, UI primitives & cleanup."""

from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WEB = ROOT / "web"

BANNED_TS_RE = [
    ("explicit_any", re.compile(r"(?::\s*any\b|\bas\s+any\b|<\s*any\s*>)")),
    ("ts_ignore", re.compile(r"@ts-(?:ignore|expect-error|nocheck)")),
    ("eslint_disable", re.compile(r"eslint-disable")),
    ("external_cdn", re.compile(r"https?://(?:cdn\.|unpkg\.com|jsdelivr\.net|fonts\.googleapis\.com|cdnjs\.)")),
]


def main() -> int:
    errors: list[str] = []
    tsconfig_path = WEB / "tsconfig.json"
    if not tsconfig_path.is_file():
        print("WEB-GUARD FAIL: missing web/tsconfig.json", file=sys.stderr)
        return 1

    tsconfig = json.loads(tsconfig_path.read_text(encoding="utf-8"))
    opts = tsconfig.get("compilerOptions", {})
    for flag in ("strict", "noUnusedLocals", "noUnusedParameters", "noUncheckedIndexedAccess"):
        if opts.get(flag) is not True:
            errors.append(f"web/tsconfig.json compilerOptions.{flag} must be true")

    ui_dir = WEB / "src/components/ui"
    if not ui_dir.is_dir() or not list(ui_dir.glob("Ui*.vue")):
        errors.append("missing reusable UI primitives under web/src/components/ui/Ui*.vue")

    for path in sorted((WEB / "src").rglob("*")):
        if not path.is_file() or path.suffix not in {".ts", ".tsx", ".vue"}:
            continue
        text = path.read_text(encoding="utf-8")
        for name, regex in BANNED_TS_RE:
            m = regex.search(text)
            if m:
                lineno = text[: m.start()].count("\n") + 1
                errors.append(f"{path.relative_to(ROOT)}:{lineno}: forbidden {name} ({m.group(0)!r})")
        if path.suffix == ".vue" and "views" in path.parts and "<select" in text:
            errors.append(f"{path.relative_to(ROOT)}: raw <select> in view; use UiSelect.vue")
        if ("setInterval(" in text or "new EventSource(" in text or "new WebSocket(" in text) and "onUnmounted" not in text:
            errors.append(f"{path.relative_to(ROOT)}: timer/stream created without onUnmounted cleanup")

    index_html = WEB / "index.html"
    if index_html.is_file():
        html = index_html.read_text(encoding="utf-8")
        if re.search(r"https?://", html):
            errors.append("web/index.html must not reference external http(s) resources")

    if errors:
        for err in errors:
            print(f"WEB-GUARD FAIL: {err}", file=sys.stderr)
        return 1
    print("WEB-GUARD PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
