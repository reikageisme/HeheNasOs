#!/usr/bin/env python3
"""Turn `cargo --message-format=json` output (stdin) into GitHub annotations.

Annotations are visible on the run page to anyone who can see the repo, unlike
raw job logs. Exits 1 if any error-level diagnostic was found.
"""
import json
import sys

errors = 0
seen = set()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if msg.get("reason") != "compiler-message":
        continue
    m = msg["message"]
    level = m.get("level")
    if level not in ("error", "warning"):
        continue
    spans = [s for s in m.get("spans", []) if s.get("is_primary")] or [{}]
    s = spans[0]
    path = s.get("file_name", "core")
    if not path.startswith("/"):
        path = "core/" + path
    text = (m.get("rendered") or m.get("message", "")).strip()
    key = (path, s.get("line_start"), m.get("message"))
    if key in seen:
        continue
    seen.add(key)
    body = text.replace("%", "%25").replace("\r", "").replace("\n", "%0A")
    kind = "error" if level == "error" else "warning"
    print(f"::{kind} file={path},line={s.get('line_start', 1)},col={s.get('column_start', 1)}::{body}")
    if level == "error":
        errors += 1

print(f"{errors} error(s)", file=sys.stderr)
sys.exit(1 if errors else 0)
