#!/usr/bin/env python3
"""G1 guard: Rust toolchain, workspace lints, clippy thresholds, zero-exemption & zero-preset checks."""

from __future__ import annotations

import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent

REQUIRED_CLIPPY_DENY = {
    "allow_attributes",
    "missing_errors_doc",
    "missing_panics_doc",
    "unwrap_used",
    "expect_used",
    "get_unwrap",
    "panic",
    "todo",
    "unimplemented",
    "unreachable",
    "indexing_slicing",
    "as_conversions",
    "as_underscore",
    "lossy_float_literal",
    "let_underscore_must_use",
    "unused_result_ok",
    "mem_forget",
    "dbg_macro",
    "print_stdout",
    "print_stderr",
    "exit",
    "rc_buffer",
    "rc_mutex",
    "infinite_loop",
    "significant_drop_in_scrutinee",
    "non_send_fields_in_send_ty",
}

BANNED_RUST_PATTERNS = [
    ("unsafe_keyword", re.compile(r"\bunsafe\b")),
    ("allow_attr", re.compile(r"#\s*!?\s*\[\s*allow\s*\(")),
    ("expect_attr", re.compile(r"#\s*!?\s*\[\s*expect\s*\(")),
    ("warn_attr", re.compile(r"#\s*!?\s*\[\s*warn\s*\(")),
    ("cfg_attr_lint", re.compile(r"cfg_attr\s*\([^)]*\b(?:allow|expect|warn)\s*\(")),
    ("doc_hidden", re.compile(r"#\s*\[\s*doc\s*\(\s*hidden\s*\)\s*\]")),
    ("rustfmt_skip", re.compile(r"#\s*\[\s*rustfmt\s*::\s*skip\s*\]")),
    ("cfg_false", re.compile(r"#\s*\[\s*cfg\s*\(\s*(?:false|any\s*\(\s*\))\s*\)\s*\]")),
    ("dynamic_include", re.compile(r"\binclude!\s*\(")),
]

COUPLED_RE = re.compile(r"\b(?:x-loadgen|mazbri|upupt|jwczjzl)\b", re.IGNORECASE)


def strip_rust_comments_and_strings(src: str) -> str:
    out: list[str] = []
    i = 0
    n = len(src)
    while i < n:
        ch = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if ch == "/" and nxt == "/":
            while i < n and src[i] != "\n":
                i += 1
            continue
        if ch == "/" and nxt == "*":
            depth = 1
            i += 2
            while i < n and depth > 0:
                if src[i : i + 2] == "/*":
                    depth += 1
                    i += 2
                elif src[i : i + 2] == "*/":
                    depth -= 1
                    i += 2
                else:
                    if src[i] == "\n":
                        out.append("\n")
                    i += 1
            continue
        if ch in ("r", "b") and i + 1 < n and src[i + 1] in ('"', "#", "r"):
            m = re.match(r'b?r(#*)"', src[i:])
            if m:
                hashes = m.group(1)
                closing = '"' + hashes
                end = src.find(closing, i + len(m.group(0)))
                if end == -1:
                    break
                segment = src[i : end + len(closing)]
                out.append('""' + "\n" * segment.count("\n"))
                i = end + len(closing)
                continue
        if ch == '"':
            i += 1
            lines = 0
            while i < n:
                if src[i] == "\\":
                    if i + 1 < n and src[i + 1] == "\n":
                        lines += 1
                    i += 2
                    continue
                if src[i] == "\n":
                    lines += 1
                if src[i] == '"':
                    i += 1
                    break
                i += 1
            out.append('""' + "\n" * lines)
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def check_workspace() -> list[str]:
    errors: list[str] = []
    tc_path = ROOT / "rust-toolchain.toml"
    if not tc_path.is_file():
        errors.append("missing rust-toolchain.toml")
    else:
        tc = tomllib.loads(tc_path.read_text(encoding="utf-8")).get("toolchain", {})
        if tc.get("channel") != "1.100.0":
            errors.append(f"rust-toolchain channel must be 1.100.0, got {tc.get('channel')}")
        comps = set(tc.get("components", []))
        if not {"clippy", "rustfmt"}.issubset(comps):
            errors.append("rust-toolchain components must include clippy and rustfmt")

    if not (ROOT / "Cargo.lock").is_file():
        errors.append("Cargo.lock must be tracked")

    for bad_cfg in (ROOT / ".cargo/config", ROOT / ".cargo/config.toml"):
        if bad_cfg.exists():
            errors.append(f"forbidden repo-local cargo config: {bad_cfg}")

    for preset in (ROOT / "imgs", ROOT / "config/keys"):
        if preset.exists():
            errors.append(f"forbidden pre-baked asset directory present: {preset}")

    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    ws = cargo.get("workspace", {})
    lints = ws.get("lints", {})
    if lints.get("rust", {}).get("unsafe_code") != "forbid":
        errors.append("[workspace.lints.rust] unsafe_code must be 'forbid'")
    if lints.get("rust", {}).get("missing_docs") != "deny":
        errors.append("[workspace.lints.rust] missing_docs must be 'deny'")
    if lints.get("rust", {}).get("trivial_casts") != "deny":
        errors.append("[workspace.lints.rust] trivial_casts must be 'deny'")
    clippy_lints = lints.get("clippy", {})
    for key in sorted(REQUIRED_CLIPPY_DENY):
        val = clippy_lints.get(key)
        level = val.get("level") if isinstance(val, dict) else val
        if level != "deny":
            errors.append(f"[workspace.lints.clippy] {key} must be 'deny'")

    for member in ws.get("members", []):
        m_toml = ROOT / member / "Cargo.toml"
        data = tomllib.loads(m_toml.read_text(encoding="utf-8"))
        if data.get("lints", {}).get("workspace") is not True:
            errors.append(f"{member}/Cargo.toml must set [lints] workspace = true")
        lib_rs = ROOT / member / "src/lib.rs"
        if lib_rs.is_file():
            head = lib_rs.read_text(encoding="utf-8")[:300]
            if "#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]" not in head:
                errors.append(f"{member}/src/lib.rs missing doctest forbid/deny attribute")

    clippy_cfg = tomllib.loads((ROOT / "clippy.toml").read_text(encoding="utf-8"))
    limits = {
        "too-many-lines-threshold": 100,
        "cognitive-complexity-threshold": 25,
        "too-many-arguments-threshold": 7,
        "type-complexity-threshold": 250,
        "max-fn-params-bools": 3,
        "max-struct-bools": 3,
        "enum-variant-size-threshold": 200,
    }
    for k, max_v in limits.items():
        got = clippy_cfg.get(k)
        if got is None or got > max_v:
            errors.append(f"clippy.toml {k} must be <= {max_v}, got {got}")

    return errors


def check_sources() -> list[str]:
    errors: list[str] = []
    crates_dir = ROOT / "crates"
    for rs in sorted(crates_dir.rglob("*.rs")):
        raw = rs.read_text(encoding="utf-8")
        m_coupled = COUPLED_RE.search(raw)
        if m_coupled:
            errors.append(f"{rs.relative_to(ROOT)}: coupled term {m_coupled.group(0)!r}")
        stripped = strip_rust_comments_and_strings(raw)
        for name, regex in BANNED_RUST_PATTERNS:
            m = regex.search(stripped)
            if m:
                lineno = stripped[: m.start()].count("\n") + 1
                errors.append(f"{rs.relative_to(ROOT)}:{lineno}: forbidden {name} ({m.group(0)!r})")
    return errors


def main() -> int:
    errors = check_workspace() + check_sources()
    if errors:
        for err in errors:
            print(f"LINT-GUARD FAIL: {err}", file=sys.stderr)
        return 1
    print("LINT-GUARD PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
