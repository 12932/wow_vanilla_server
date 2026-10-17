"""Merges mapping results (file,line,status,wvs_test,note) into the matrix, by file and line.

Usage: merge_map.py matrix.csv map-a.csv [map-b.csv ...]
"""
import csv, sys
from pathlib import Path

STATUSES = {"unmapped", "covered", "gap", "accepted", "wrong-for-1.12", "not-applicable"}
matrix = Path(sys.argv[1])
with open(matrix, newline="") as f:
    rows = list(csv.DictReader(f))
index = {(r["file"], r["line"]): r for r in rows}
for part in sys.argv[2:]:
    n = bad = 0
    with open(part, newline="") as f:
        for m in csv.DictReader(f):
            r = index.get((m["file"], str(m["line"]).strip()))
            status = m["status"].strip()
            if r is None or status not in STATUSES:
                bad += 1
                continue
            r["status"], r["wvs_test"], r["note"] = status, m.get("wvs_test", "").strip(), m.get("note", "").strip()
            n += 1
    print(f"{part}: {n} rows merged, {bad} skipped")
with open(matrix, "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
    w.writeheader()
    w.writerows(rows)
