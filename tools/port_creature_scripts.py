"""Thistle Tea's creature scripts' simple events (timed casts, health
thresholds, aggro calls for help) as a Rust table for wvs.
Usage: uv run --python 3.14 python -I tools/port_creature_scripts.py <creature_script dir> <out.rs> <skip entries,...>"""
import re, sys, pathlib

src = pathlib.Path(sys.argv[1]); out = pathlib.Path(sys.argv[2])
skip = {int(x) for x in sys.argv[3].split(",") if x}
TARGETS = {"self": "SELF", "victim": "1", "hostile_random": "4", "hostile_random_not_top": "5",
           "hostile_nearest": "6", "hostile_farthest": "7", "hostile_second_aggro": "2",
           "hostile_last_aggro": "3", "provided": "0"}
rows, entries, skipped = [], set(), 0

def num(tok, consts):
    tok = tok.strip()
    if tok.startswith("@"):
        return consts.get(tok[1:])
    tok = tok.replace("_", "")
    try:
        return int(tok)
    except ValueError:
        return None

def rng(tok, consts):
    tok = tok.strip()
    m = re.fullmatch(r"\{\s*([^,]+),\s*([^}]+)\}", tok)
    if m:
        a, b = num(m.group(1), consts), num(m.group(2), consts)
        return None if a is None or b is None else (a, b)
    v = num(tok, consts)
    return None if v is None else (v, v)

def flags_of(tok, consts):
    total = 0
    for part in tok.split("+"):
        part = part.strip()
        v = num(part, consts)
        if v is None:
            return None
        total += v
    return total

def split_args(s):
    depth, cur, out = 0, "", []
    for ch in s:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip()); cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out

def cast_of(expr, consts):
    m = re.fullmatch(r"Combat\.cast\((.*)\)", expr.strip(), re.S)
    if not m:
        return None
    a = split_args(m.group(1))
    spell = num(a[0], consts)
    target = "1"
    flags = 0
    if len(a) > 1:
        t = a[1].strip().lstrip(":")
        if t not in TARGETS:
            return None
        target = TARGETS[t]
    if len(a) > 2:
        flags = flags_of(a[2], consts)
        if flags is None:
            return None
    if spell is None:
        return None
    return spell, target, flags

for f in sorted(src.glob("*.ex")):
    text = f.read_text()
    consts = {}
    for name, val in re.findall(r"^\s*@(\w+)\s+([0-9_]+)\s*$", text, re.M):
        consts[name] = int(val.replace("_", ""))
    for m in re.finditer(r"def events\(@(\w+) = entry\) do\n(.*?)\n  end\n", text, re.S):
        entry = consts.get(m.group(1))
        if entry is None or entry in skip:
            continue
        body = m.group(2)
        # Each top-level call in the list.
        calls = re.findall(r"(Combat\.every\(entry,.*?\)|Combat\.below_health\(entry,.*?\)|CreatureScript\.event\(entry,.*?\))(?=,\s*\n\s*(?:Combat|CreatureScript|\])|\s*\n\s*\])", body, re.S)
        got = 0
        for c in calls:
            c = " ".join(c.split())
            if c.startswith("Combat.every("):
                args = split_args(c[len("Combat.every("):-1])
                if len(args) != 5:
                    skipped += 1; continue
                cast = cast_of(args[2], consts)
                first, rep = rng(args[3], consts), rng(args[4], consts)
                if not cast or not first or not rep:
                    skipped += 1; continue
                rows.append((entry, "TIMER", first[0], first[1], rep[0], rep[1], *cast, 0)); got += 1
            elif c.startswith("Combat.below_health("):
                args = split_args(c[len("Combat.below_health("):-1])
                if len(args) != 4:
                    skipped += 1; continue
                cast = cast_of(args[2], consts)
                pct = num(args[3], consts)
                if not cast or pct is None:
                    skipped += 1; continue
                rows.append((entry, "BELOW", 0, 0, 0, 0, *cast, pct)); got += 1
            elif c.startswith("CreatureScript.event("):
                args = split_args(c[len("CreatureScript.event("):-1])
                if len(args) == 4 and args[2] == ":aggro" and args[3].replace(" ", "") in ("[zone_combat()]", "[zone_pulse()]"):
                    rows.append((entry, "AGGRO_ZONE", 0, 0, 0, 0, 0, "1", 0, 0)); got += 1
                elif len(args) == 4 and args[2] == ":aggro":
                    cast = cast_of(args[3].strip()[1:-1], consts)
                    if cast:
                        rows.append((entry, "AGGRO_CAST", 0, 0, 0, 0, *cast, 0)); got += 1
                    else:
                        skipped += 1
                else:
                    skipped += 1
            else:
                skipped += 1
        if got:
            entries.add(entry)

lines = ["//! Generated from Thistle Tea's creature scripts by tools/port_creature_scripts.py:",
         "//! their timed casts, health thresholds and aggro calls. Do not edit by hand.", "",
         "use super::creature_scripts::{Kind, Row, SELF};", "",
         "pub const ROWS: &[Row] = &["]
for (entry, kind, f0, f1, r0, r1, spell, target, flags, pct) in rows:
    lines.append(f"    Row {{ entry: {entry}, kind: Kind::{kind.title().replace('_', '')}, first: ({f0}, {f1}), repeat: ({r0}, {r1}), spell: {spell}, target: {target}, flags: {flags}, pct: {pct} }},")
lines.append("];")
out.write_text("\n".join(lines) + "\n")
print(f"{len(rows)} events for {len(entries)} creatures; {skipped} left out")
