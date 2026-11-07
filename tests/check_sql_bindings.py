"""
Static guard: verify that every rusqlite `params![...]` supplies exactly as many
values as its SQL statement expects.

Why this exists
---------------
On 2026-11-07 the Risk dashboard crashed with:
    Wrong number of parameters passed to query. Got 10, needed 9
Two `INSERT INTO risk_events` statements reused a numbered placeholder
(`?9, ?9` / `?5, ?5, ?5`) but `params!` appended one extra `now` for each
reused slot. Numbered placeholders are positional: reusing ?N does NOT consume
another parameter. The required count is the LARGEST index, not the number of
`?` occurrences or the number of columns.

`cargo check` cannot catch this (it is a runtime binding error), and the
project's Rust unit-test target currently fails to compile for unrelated
historical reasons, so this dependency-free script is the regression guard.

Usage
-----
    python tests/check_sql_bindings.py            # scan src-tauri/src
    python tests/check_sql_bindings.py --selftest # verify the detector itself
Exit code 1 if any mismatch (or if the self-test fails).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "src-tauri" / "src"


def split_top_level(body: str) -> list[str]:
    """Split a macro body on commas that are outside strings/nested delimiters."""
    parts: list[str] = []
    buf: list[str] = []
    depth = 0
    in_string = False
    in_char = False
    escaped = False
    i = 0
    while i < len(body):
        ch = body[i]
        if escaped:
            buf.append(ch)
            escaped = False
            i += 1
            continue
        if ch == "\\" and (in_string or in_char):
            buf.append(ch)
            escaped = True
            i += 1
            continue
        if in_string:
            buf.append(ch)
            if ch == '"':
                in_string = False
            i += 1
            continue
        if in_char:
            buf.append(ch)
            if ch == "'":
                in_char = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            buf.append(ch)
        elif ch == "'":
            # Rust lifetime ('a) is not a char literal; only treat as a char
            # literal when a closing quote follows shortly.
            if re.match(r"'(\\.|[^'\\])'", body[i:]):
                in_char = True
            buf.append(ch)
        elif ch in "([{":
            depth += 1
            buf.append(ch)
        elif ch in ")]}":
            depth -= 1
            buf.append(ch)
        elif ch == "," and depth == 0:
            parts.append("".join(buf).strip())
            buf = []
        else:
            buf.append(ch)
        i += 1
    tail = "".join(buf).strip()
    if tail:
        parts.append(tail)
    return parts


def find_macro_end(text: str, open_pos: int) -> int:
    """Return the index just after the ] / ) matching the macro at open_pos."""
    opener = text[open_pos]
    closer = "]" if opener == "[" else ")"
    depth = 0
    in_string = False
    escaped = False
    i = open_pos
    while i < len(text):
        ch = text[i]
        if escaped:
            escaped = False
            i += 1
            continue
        if ch == "\\" and in_string:
            escaped = True
            i += 1
            continue
        if ch == '"':
            in_string = not in_string
        elif not in_string:
            if ch == opener:
                depth += 1
            elif ch == closer:
                depth -= 1
                if depth == 0:
                    return i + 1
        i += 1
    return -1


def string_literals(text: str) -> list[tuple[int, int, str]]:
    """All normal Rust string literals as (start, end, content) — supports newlines."""
    out: list[tuple[int, int, str]] = []
    i = 0
    n = len(text)
    while i < n:
        if text[i] == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    break
                j += 1
            j = min(j, n - 1)
            out.append((i, j, text[i + 1 : j]))
            i = j + 1
        else:
            i += 1
    return out


def expected_count(sql: str) -> int | None:
    """Needed parameter count, or None when it cannot be determined statically."""
    numbered = [int(x) for x in re.findall(r"\?(\d+)", sql)]
    bare = len(re.findall(r"\?(?!\d)", sql))
    if numbered:
        return max(numbered) if bare == 0 else None
    if bare:
        return bare
    # No placeholder in this literal -> SQL is likely built via format!() with
    # a constant. Cannot verify statically; skip rather than emit false alarms.
    return None


CONST_SQL_DEF = re.compile(
    r"\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*\"((?:[^\"\\]|\\.)*)\"",
    re.S,
)


def check_file(path: Path) -> tuple[int, int, list[str]]:
    text = path.read_text(encoding="utf-8")
    literals = string_literals(text)
    const_sql = {m.group(1): m.group(2) for m in CONST_SQL_DEF.finditer(text)}
    # A SQL statement is often pulled into a named `const &str` and then
    # referenced as `conn.execute(MEMO_BY_ID, params![...])`. Register each
    # such reference as a virtual literal at the *call site*, otherwise the
    # guard would inspect the previous (unrelated) statement's SQL.
    if const_sql:
        names = "|".join(re.escape(n) for n in sorted(const_sql, key=len, reverse=True))
        for m in re.finditer(r"[(,]\s*(" + names + r")\s*[,)]", text):
            literals.append((m.start(1), m.end(1), const_sql[m.group(1)]))
        literals.sort(key=lambda lit: lit[0])
    problems: list[str] = []
    checked = 0
    skipped = 0

    for match in re.finditer(r"params!\s*([\[(])", text):
        open_pos = match.end() - 1
        end_pos = find_macro_end(text, open_pos)
        if end_pos < 0:
            continue
        body = text[open_pos + 1 : end_pos - 1]
        parts = split_top_level(body)
        got = len(parts)

        # Pick the closest preceding literal that DECLARES parameters. This
        # skips log strings, parameter-free SELECTs and the placeholder-free
        # branch of a `let sql = match ...`, while still handling dynamic
        # `format!("{} WHERE ... ?1", base_query)` fragments and const &str
        # references (registered as virtual literals above).
        prior = [lit for lit in literals if lit[1] < match.start()]
        chosen = next(
            (lit for lit in reversed(prior) if expected_count(lit[2]) is not None),
            None,
        )
        if chosen is None:
            skipped += 1
            continue
        sql = chosen[2]
        needed = expected_count(sql)

        line = text.count("\n", 0, match.start()) + 1
        checked += 1
        if got != needed:
            problems.append(
                f"  MISMATCH {path.relative_to(ROOT)}:{line}  "
                f"params! supplies {got}, SQL needs {needed}\n"
                f"    SQL: {' '.join(sql.split())[:130]}"
            )
    return checked, skipped, problems


def selftest() -> bool:
    cases = [
        # (sql, params body, expected mismatch?)
        ("INSERT INTO t VALUES (?1, ?2, ?3, ?3)", "a, b, c, d", True),
        ("INSERT INTO t VALUES (?1, ?2, ?3, ?3)", "a, b, c", False),
        ("SELECT * FROM t WHERE id = ?1", "id", False),
        ("UPDATE t SET a = ?2 WHERE id = ?1", "id, now_utc_str()", False),
        (
            "INSERT INTO t (a, b) VALUES (?1, ?2, ?3)",
            'id, format!("x, y", n), json!({"k": 1, "v": 2})',
            None,  # exercise nested-comma splitting; no mismatch assertion
        ),
    ]
    ok = True
    for sql, body, expect_bad in cases:
        needed = expected_count(sql)
        got = len(split_top_level(body))
        if expect_bad is None:
            print(f"  [splitter] nested comma handled: got {got} args -> {split_top_level(body)}")
            continue
        bad = needed is not None and got != needed
        status = "OK" if bad == expect_bad else "FAIL"
        if status == "FAIL":
            ok = False
        print(f"  [{status}] needed={needed} got={got} expected_mismatch={expect_bad}")
    return ok


def main() -> int:
    if "--selftest" in sys.argv:
        print("=== self-test ===")
        return 0 if selftest() else 1

    print("=== SQL binding check (params! vs SQL placeholders) ===")
    total_checked = 0
    total_skipped = 0
    all_problems: list[str] = []
    for path in sorted(SRC_DIR.rglob("*.rs")):
        checked, skipped, problems = check_file(path)
        total_checked += checked
        total_skipped += skipped
        all_problems.extend(problems)

    if all_problems:
        print("\n".join(all_problems))
        print(f"\nFAILED: {len(all_problems)} mismatch(es) / {total_checked} checked, {total_skipped} skipped")
        return 1
    print(f"OK: {total_checked} binding(s) checked, {total_skipped} skipped (dynamic SQL)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
