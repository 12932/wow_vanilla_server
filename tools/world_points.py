"""Observer points that cover every creature spawn of Eastern Kingdoms and Kalimdor.

A greedy set cover: each point is a creature spawn, chosen as the one whose
radius takes in the most spawns not yet covered (ties to the lowest spawn
guid), until every spawn of maps 0 and 1 is within the radius of a point. The
spawns are the server's: vmangos's `creature` rows in patch 10.

    uv run --python 3.14 python -I tools/world_points.py \
        $WVS_DATA/db/vmangos.sqlite docs/world_points.csv

writes `map,x,y,z` a line, in a fixed, seeded random order (so the first N
points are a fair sample of the world), and prints the point counts and the
share of spawns the first 10%, 25%, 50% and 100% of the points cover.
Standard library only; the same database, radius and seed give the same file.
"""

import argparse
import heapq
import math
import random
import sqlite3
from collections import defaultdict

MAPS = (0, 1)
NAMES = {0: "Eastern Kingdoms", 1: "Kalimdor"}
FRACTIONS = (0.10, 0.25, 0.50, 1.00)


def spawns(db):
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute(
        "SELECT guid, map, position_x, position_y, position_z FROM creature "
        "WHERE patch_min <= 10 AND patch_max >= 10 AND map IN (0, 1) ORDER BY guid"
    ).fetchall()
    by_map = defaultdict(list)
    for _guid, m, x, y, z in rows:
        by_map[m].append((x, y, z))
    return by_map


def grid(points, cell):
    g = defaultdict(list)
    for i, (x, y, _) in enumerate(points):
        g[(math.floor(x / cell), math.floor(y / cell))].append(i)
    return g


def neighbours(points, radius):
    """For each spawn, the spawns within `radius` of it (itself included)."""
    g = grid(points, radius)
    r2 = radius * radius
    out = []
    for x, y, _ in points:
        cx, cy = math.floor(x / radius), math.floor(y / radius)
        near = []
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for j in g.get((cx + dx, cy + dy), ()):
                    px, py, _ = points[j]
                    if (px - x) ** 2 + (py - y) ** 2 <= r2:
                        near.append(j)
        out.append(near)
    return out


def cover(points, near):
    """Greedy set cover, lazily: a spawn's gain is only recounted when popped."""
    covered = [False] * len(points)
    heap = [(-len(n), i) for i, n in enumerate(near)]
    heapq.heapify(heap)
    chosen = []
    left = len(points)
    while left:
        neg, i = heapq.heappop(heap)
        gain = sum(1 for j in near[i] if not covered[j])
        if gain == 0:
            continue
        if gain != -neg:
            heapq.heappush(heap, (-gain, i))
            continue
        chosen.append(i)
        for j in near[i]:
            if not covered[j]:
                covered[j] = True
                left -= 1
    return chosen


def coverage(n_points, order, near):
    """The share of spawns covered by the first k of `order`, at each fraction of it."""
    covered = set()
    shares = {}
    marks = {max(1, round(f * len(order))): f for f in FRACTIONS}
    for k, i in enumerate(order, 1):
        covered.update(near[i])
        if k in marks:
            shares[marks[k]] = len(covered) / n_points
    return shares


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("db", help="vmangos.sqlite")
    ap.add_argument("out", help="the CSV to write")
    ap.add_argument("--radius", type=float, default=90.0, help="yards (default 90)")
    ap.add_argument("--seed", type=int, default=1, help="the order's seed (default 1)")
    a = ap.parse_args()

    by_map = spawns(a.db)
    print(f"radius {a.radius:g} yd, seed {a.seed}")
    near = {}
    order = []  # (map, spawn index), both continents
    for m in MAPS:
        pts = by_map[m]
        near[m] = neighbours(pts, a.radius)
        chosen = cover(pts, near[m])
        cells = {(math.floor(x / 100), math.floor(y / 100)) for x, y, _ in pts}
        print(
            f"map {m} ({NAMES[m]}): {len(pts)} spawns in {len(cells)} 100-yard cells, "
            f"{len(chosen)} points"
        )
        order.extend((m, i) for i in chosen)
    # One seeded order over both continents, so the first N points of the
    # file are a fair sample of the world: the benchmark and peon take them.
    random.Random(a.seed).shuffle(order)
    total = len(order)
    print("the first N points of the file cover, of each continent's spawns:")
    for f in FRACTIONS:
        k = max(1, round(f * total))
        parts = []
        for m in MAPS:
            mine = [i for mm, i in order[:k] if mm == m]
            share = coverage(len(by_map[m]), mine, near[m])[1.0] if mine else 0.0
            parts.append(f"map {m} {len(mine)} points {share * 100:.1f}%")
        print(f"  {int(f * 100):3}% ({k} points): " + ", ".join(parts))
    with open(a.out, "w") as out:
        out.write("map,x,y,z\n")
        for m, i in order:
            x, y, z = by_map[m][i]
            out.write(f"{m},{x:.2f},{y:.2f},{z:.2f}\n")
    print(f"wrote {total} points to {a.out}")


if __name__ == "__main__":
    main()
