# The parity matrix

`matrix.csv` holds every Thistle Tea test, one row each. It records what in wvs checks the same behaviour. This is
Phase 1a of [FULL_PARITY_PLAN.md](../FULL_PARITY_PLAN.md): its headline, covered over applicable, measures how close
wvs is.

## Where it stands (2026-10-17, against wvs at 20461ca)

Every row is mapped. **1,781 of the 8,429 applicable Thistle Tea tests are covered: 21%.** Most gaps are behaviour wvs
has with no test asserting it, not behaviour it lacks. The rest are listed by feature in [FINDINGS.md](FINDINGS.md).

| Area | Covered or accepted / applicable | | The rest |
|---|---|---|---|
| game/core | 1091 / 5431 | 20.1% | gap 4339, not-applicable 242, wrong-for-1.12 1 |
| game/world | 510 / 2572 | 19.8% | gap 2062, not-applicable 149 |
| game/network | 123 / 264 | 46.6% | gap 140, not-applicable 3, wrong-for-1.12 1 |
| game/inbound | 42 / 120 | 35.0% | gap 78 |
| . | 14 / 20 | 70.0% | gap 6, not-applicable 1 |
| game | 1 / 12 | 8.3% | gap 11 |
| game/architecture | 0 / 0 | 100.0% | not-applicable 8 |
| telemetry | 0 / 0 | 100.0% | not-applicable 7 |
| auth | 0 / 6 | 0.0% | gap 6 |
| dev_seed | 0 / 0 | 100.0% | not-applicable 6 |
| db/mangos | 0 / 4 | 0.0% | gap 4 |
| native | 0 / 0 | 100.0% | not-applicable 3 |
| web/utils | 0 / 0 | 100.0% | not-applicable 2 |
| **all** | 1781 / 8429 | 21.1% | gap 6646, not-applicable 421, wrong-for-1.12 2 |

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
