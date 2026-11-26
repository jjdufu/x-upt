#!/usr/bin/env python3
"""G1 guard: Markdown relative links, rule file byte budgets, coupled terms, and .gitignore."""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LINK_RE = re.compile(r"\[[^\]]+\]\(([^)#\s]+)(?:#[^)]+)?\)")
COUPLED_RE = re.compile(r"\b(?:x-loadgen|mazbri|upupt|jwczjzl)\b", re.IGNORECASE)


def strip_fenced_code(md: str) -> str:
    out: list[str] = []
    in_fence = False
    for line in md.splitlines():
        if line.lstrip().startswith("```") or line.lstrip().startswith("~~~"):
            in_fence = not in_fence
            continue
        if not in_fence:
            out.append(line)
    return "\n".join(out)


def main() -> int:
    errors: list[str] = []

    for rule_name in ("AGENTS.md", "IRON_RULES.md", "GATE_BOOTSTRAP.md"):
        p = ROOT / rule_name
        if not p.is_file():
            errors.append(f"missing {rule_name}")
            continue
        size = p.stat().st_size
        if size >= 24_000:
            errors.append(f"{rule_name} exceeds 24,000 bytes ({size})")

    skip_dirs = {"target", "node_modules", ".git", "changelog"}
    for md in sorted(ROOT.rglob("*.md")):
        parts = set(md.relative_to(ROOT).parts)
        if parts & skip_dirs:
            continue
        raw = md.read_text(encoding="utf-8")
        if md.name != "CHANGELOG.md":
            m_c = COUPLED_RE.search(raw)
            if m_c:
                errors.append(f"{md.relative_to(ROOT)}: coupled term {m_c.group(0)!r}")
        cleaned = strip_fenced_code(raw)
        for m in LINK_RE.finditer(cleaned):
            target = m.group(1)
            if "://" in target or target.startswith("mailto:"):
                continue
            resolved = (md.parent / target).resolve()
            if not resolved.exists():
                errors.append(f"{md.relative_to(ROOT)}: broken link -> {target}")

    gitignore = (ROOT / ".gitignore").read_text(encoding="utf-8")
    for required in ("target/", "node_modules/", "web/dist/", ".env", "logs/", "data/", "*.pid"):
        if required not in gitignore:
            errors.append(f".gitignore missing pattern {required!r}")

    if errors:
        for err in errors:
            print(f"DOCS-GUARD FAIL: {err}", file=sys.stderr)
        return 1
    print("DOCS-GUARD PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
