"""The matrix's headline: covered over applicable, overall and per area.

Usage: summary.py [matrix.csv] [depth]   (depth: how many path parts of the area to group by, default 2)
"""
import collections, csv, sys
from pathlib import Path

path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).with_name("matrix.csv")
depth = int(sys.argv[2]) if len(sys.argv) > 2 else 2
by = collections.defaultdict(collections.Counter)
for r in csv.DictReader(open(path, newline="")):
    by["/".join(r["area"].split("/")[:depth])][r["status"]] += 1
total = collections.Counter()
for c in by.values():
    total.update(c)

def line(name, c):
    applicable = sum(c.values()) - c["not-applicable"]
    done = c["covered"] + c["accepted"]
    pct = 100 * done / applicable if applicable else 100
    rest = ", ".join(f"{k} {v}" for k, v in sorted(c.items()) if k not in ("covered", "accepted"))
    return f"| {name} | {done} / {applicable} | {pct:.1f}% | {rest} |"

print("| Area | Covered or accepted / applicable | | The rest |")
print("|---|---|---|---|")
for name in sorted(by, key=lambda n: -sum(by[n].values())):
    print(line(name, by[name]))
print(line("**all**", total))
