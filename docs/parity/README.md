# The parity matrix

`matrix.csv` holds every Thistle Tea test, one row each. It records what in wvs checks the same behaviour. This is
Phase 1a of [FULL_PARITY_PLAN.md](../FULL_PARITY_PLAN.md): its headline, covered over applicable, measures how close
wvs is.

## Where it stands (2026-10-18, after Phase 3's first round)

Every row is mapped. Phase 1a found 1,781 of the 8,429 applicable tests covered (21%). Phase 3's first round (3a to
3f) settled 1,377 more rows, most as covered, with a fix where a test showed wvs differed. The open gaps are listed by
feature in [FINDINGS.md](FINDINGS.md).

| Area | Covered or accepted / applicable | | The rest |
|---|---|---|---|
| game/core | 1966 / 5423 | 36.3% | gap 3436, not-applicable 250, wrong-for-1.12 21 |
| game/world | 686 / 2571 | 26.7% | gap 1884, not-applicable 150, wrong-for-1.12 1 |
| game/network | 222 / 261 | 85.1% | gap 38, not-applicable 6, wrong-for-1.12 1 |
| game/inbound | 101 / 119 | 84.9% | gap 16, not-applicable 1, wrong-for-1.12 2 |
| . | 17 / 20 | 85.0% | gap 3, not-applicable 1 |
| game | 1 / 12 | 8.3% | gap 11 |
| game/architecture | 0 / 0 | 100.0% | not-applicable 8 |
| telemetry | 0 / 0 | 100.0% | not-applicable 7 |
| auth | 6 / 6 | 100.0% |  |
| dev_seed | 0 / 0 | 100.0% | not-applicable 6 |
| db/mangos | 0 / 4 | 0.0% | gap 4 |
| native | 0 / 0 | 100.0% | not-applicable 3 |
| web/utils | 0 / 0 | 100.0% | not-applicable 2 |
| **all** | 2999 / 8416 | 35.6% | gap 5392, not-applicable 434, wrong-for-1.12 25 |

## The columns

| Column | What it holds |
|---|---|
| `area` | the test's directory under Thistle Tea's `test/` (`game/core/spell`) |
| `module`, `file`, `line` | where the test is |
| `describe` | its enclosing `describe` blocks, joined by ` / ` |
| `test` | its name |
| `status` | one of the statuses below |
| `wvs_test` | the wvs tests that check the same behaviour, `module::name`, separated by `;` |
| `note` | why, where it isn't plain |

| Status | Meaning |
|---|---|
| `unmapped` | not looked at yet |
| `covered` | a wvs test checks the same behaviour, and passes |
| `gap` | wvs doesn't do it, or no test shows it does |
| `accepted` | wvs differs on purpose, as the user accepted ([THISTLE_TEA_DIFFERENCES.md](../THISTLE_TEA_DIFFERENCES.md)) |
| `wrong-for-1.12` | Thistle Tea's behaviour isn't 1.12's (mangos and the client say otherwise); waiting on the user |
| `not-applicable` | nothing in a game server answers to it: the web server, Elixir's telemetry and code layering |

A test of Thistle Tea's internals, such as a module's data structure, counts as `covered` when wvs has the behaviour
the structure serves. The `note` says which behaviour.

## The tools

- **`list_tt_tests.py <thistle tea checkout> matrix.csv matrix.csv`** lists the tests again. It keeps every row's
  status, `wvs_test` and note, matched by file, `describe` and name. Tests that are new come in as `unmapped`, and
  tests that are gone drop out.
- **`summary.py [matrix.csv] [depth]`** prints the headline per area.

## Keeping up with Thistle Tea (Phase 1c)

The pin is `0426f0d6` (2026-10-11). To move it:
1. **Diff its game code:** `git diff 0426f0d6..<new> -- lib/game test` in the Thistle Tea checkout. Note what
   changed in behaviour, beyond the tests.
2. **Regenerate the imported scripts:** `tt_scripts.json.gz`, with `tools/dump_tt_scripts.exs`.
3. **List the tests again** with `list_tt_tests.py`, and map the new `unmapped` rows.
4. **Record the new pin** here and in [THISTLE_TEA_DIFFERENCES.md](../THISTLE_TEA_DIFFERENCES.md).
