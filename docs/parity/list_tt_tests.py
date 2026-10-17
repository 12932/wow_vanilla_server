"""Lists every Thistle Tea test (ExUnit `test "..."`) with its area, file, line and enclosing `describe`.

Usage: list_tt_tests.py <thistle_tea checkout> <out.csv> [<existing matrix.csv to carry mappings from>]
"""
import csv, re, sys
from pathlib import Path

root, out = Path(sys.argv[1]), Path(sys.argv[2])
old = {}
if len(sys.argv) > 3 and Path(sys.argv[3]).exists():
    with open(sys.argv[3], newline="") as f:
        for r in csv.DictReader(f):
            old[(r["file"], r["describe"], r["test"])] = r

# Areas with no counterpart in a game server: the web server, Elixir's telemetry and internals.
NOT_APPLICABLE = [
    ("web", "Thistle Tea's web server"),
    ("telemetry", "Elixir telemetry"),
    ("dev_seed", "Thistle Tea's developer seed characters"),
    ("native", "namigator's Elixir NIF"),
    ("game/architecture", "Elixir code-layering rules"),
]

describe_re = re.compile(r'^(\s*)describe\s+"((?:[^"\\]|\\.)*)"')
test_re = re.compile(r'^(\s*)test\s+"((?:[^"\\]|\\.)*)"')
module_re = re.compile(r'^\s*defmodule\s+([\w.]+)')
rows = []
for path in sorted(root.glob("test/**/*_test.exs")):
    rel = path.relative_to(root).as_posix()
    area = "/".join(rel.split("/")[1:-1]) or "."
    module, describes = "", []  # (indent, name)
    for n, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
        if m := module_re.match(line):
            module = module or m.group(1)
        if m := describe_re.match(line):
            ind = len(m.group(1))
            describes = [d for d in describes if d[0] < ind] + [(ind, m.group(2))]
            continue
        if m := test_re.match(line):
            ind = len(m.group(1))
            describes = [d for d in describes if d[0] < ind]
            desc = " / ".join(d[1] for d in describes)
            key = (rel, desc, m.group(2))
            prev = old.get(key, {})
            na = next((why for pre, why in NOT_APPLICABLE if area.startswith(pre)), None)
            if na and prev.get("status", "unmapped") == "unmapped":
                prev = {"status": "not-applicable", "note": na}
            rows.append({
                "area": area, "module": module, "file": rel, "line": n, "describe": desc, "test": m.group(2),
                "status": prev.get("status", "unmapped"), "wvs_test": prev.get("wvs_test", ""),
                "note": prev.get("note", ""),
            })
out.parent.mkdir(parents=True, exist_ok=True)
with open(out, "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
    w.writeheader()
    w.writerows(rows)
print(len(rows), "tests")
