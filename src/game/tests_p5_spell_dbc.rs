//! Phase 3, round 5, against Thistle Tea's DBC proc tests on the real
//! data: Pyroclasm (`pyroclasm_dbc_test.exs`), Blessed Recovery and the
//! Persistent Shield (`proc_spell_dbc_test.exs`), each through
//! `procs::trigger` with the event its spell's landing raises.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{self as kit, has, holder};
use crate::game::map::procs::{self, EX_CRITICAL_HIT, EX_NORMAL_HIT, EX_RESIST, Event};
use crate::game::map::{Map, auras};

const PYROCLASM: [u32; 2] = [18096, 18073];
const PYROCLASM_STUN: u32 = 18093;
const SOUL_FIRE: u32 = 6353;
const HELLFIRE_EFFECT: u32 = 5857;
const RAIN_OF_FIRE: u32 = 5740;
const SHADOW_BOLT: u32 = 686;
const HELLFIRE: [u32; 3] = [1949, 11683, 11684];
const RAINS: [u32; 4] = [5740, 6219, 11677, 11678];

/// A harmful spell's landing on `target` from `actor`, or a tick of one.
fn landed(m: &mut Map, actor: Guid, target: Guid, spell: u32, periodic: bool, ex: u32) {
    let (done, taken) = if periodic {
        (procs::PROC_DONE_PERIODIC, procs::PROC_TAKEN_PERIODIC)
    } else {
        (
            procs::PROC_DONE_NEGATIVE_SPELL,
            procs::PROC_TAKEN_NEGATIVE_SPELL,
        )
    };
    procs::trigger(
        m,
        Event {
            actor,
            target,
            actor_flags: done,
            target_flags: taken | procs::PROC_TAKEN_DAMAGE,
            ex,
            spell: Some(spell),
            damage: 100,
            school: 4,
            ..Default::default()
        },
    );
}

/// How often `n` landings of `spell` stun the kobold.
fn stuns(m: &mut Map, w: Guid, kobold: Guid, spell: u32, periodic: bool, ex: u32, n: u32) -> u32 {
    (0..n)
        .filter(|_| {
            auras::remove_spell(m, kobold, PYROCLASM_STUN, None);
            if let Some(c) = m.caster_mut(kobold) {
                c.diminishing.clear();
            }
            landed(m, w, kobold, spell, periodic, ex);
            has(m, kobold, PYROCLASM_STUN)
        })
        .count() as u32
}

/// Both ranks of Pyroclasm stun from Soul Fire (by its icon and visual),
/// a Hellfire Effect's hit and a Rain of Fire's tick, by their chance over
/// the whole channel; nothing from a resist, a dead target, one's own
/// tick, or a Shadow Bolt; the second rank takes the first's rule; the
/// channels tick as the rule counts (Thistle Tea's Pyroclasm tests).
#[tokio::test]
#[ignore = "extended"]
async fn pyroclasm_stuns_from_its_fires_by_their_share_of_its_chance() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_a, w, map) = kit::player(&realm, "P5DA", "Pyro", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[]);
            let kobold = kit::dummy(m, w, 60, 5.0);
            let data = m.realm.data.clone();
            let mut rates = Vec::new();
            for talent in PYROCLASM {
                auras::remove_where(m, w, |h| PYROCLASM.contains(&h.spell));
                kit::apply(m, w, w, talent);
                let soul_fire = stuns(m, w, kobold, SOUL_FIRE, false, EX_NORMAL_HIT, 2_000);
                let hellfire = stuns(m, w, kobold, HELLFIRE_EFFECT, false, EX_NORMAL_HIT, 6_000);
                let rain = stuns(m, w, kobold, RAIN_OF_FIRE, true, EX_NORMAL_HIT, 4_000);
                let resisted = stuns(m, w, kobold, SOUL_FIRE, false, EX_RESIST, 200);
                let bolt = stuns(m, w, kobold, SHADOW_BOLT, false, EX_NORMAL_HIT, 200);
                rates.push((talent, soul_fire, hellfire, rain, resisted, bolt));
            }
            // Its own tick, and a dead target.
            let own = stuns(m, w, w, RAIN_OF_FIRE, true, EX_NORMAL_HIT, 200);
            crate::game::map::combat::deal_damage(m, w, kobold, 2_000_000);
            let dead = stuns(m, w, kobold, SOUL_FIRE, false, EX_NORMAL_HIT, 200);
            let rules = PYROCLASM.map(|t| {
                data.spells
                    .proc_events
                    .get(&t)
                    .copied()
                    .map(|p| (p.flags, p.family_mask[0]))
            });
            let channels: Vec<(u32, Option<i32>, u32)> = HELLFIRE
                .iter()
                .chain(RAINS.iter())
                .map(|id| {
                    let s = data.spells.get(*id).unwrap();
                    let periodic = s
                        .effects
                        .iter()
                        .find(|e| e.amplitude > 0)
                        .map_or(0, |e| e.amplitude);
                    (
                        *id,
                        data.spells.durations.get(&s.duration_index).map(|d| d.base),
                        periodic,
                    )
                })
                .collect();
            (rates, own, dead, rules, channels)
        })
        .await
        .unwrap();
    let (rates, own, dead, rules, channels) = out;
    for (talent, soul_fire, hellfire, rain, resisted, bolt) in rates {
        let pct = if talent == PYROCLASM[0] { 13.0 } else { 26.0 };
        let near = |seen: u32, n: f32, share: f32| {
            let expected = n * pct / 100.0 / share;
            (seen as f32 - expected).abs() <= expected * 0.35 + 5.0
        };
        assert!(
            near(soul_fire, 2_000.0, 1.0),
            "{talent} Soul Fire: {soul_fire}"
        );
        assert!(
            near(hellfire, 6_000.0, 15.0),
            "{talent} Hellfire: {hellfire}"
        );
        assert!(near(rain, 4_000.0, 4.0), "{talent} Rain of Fire: {rain}");
        assert_eq!(
            (resisted, bolt),
            (0, 0),
            "{talent}: a resist, a Shadow Bolt"
        );
    }
    assert_eq!((own, dead), (0, 0), "its own tick, a dead target");
    assert_eq!(rules[0], rules[1], "the second rank takes the first's rule");
    assert_eq!(rules[0].map(|r| r.0), Some(0x50400));
    for (id, duration, amplitude) in channels {
        if HELLFIRE.contains(&id) {
            assert_eq!((duration, amplitude), (Some(15_000), 1_000), "{id}");
        } else {
            assert_eq!((duration, amplitude), (Some(8_000), 2_000), "{id}");
        }
    }
}

const BLESSED_RECOVERY: [(u32, u32); 3] = [(27811, 27813), (27815, 27817), (27816, 27818)];
const PERSISTENT_SHIELD: u32 = 26467;
const PERSISTENT_SHIELD_ABSORB: u32 = 26470;

/// Every rank of Blessed Recovery heals its priest over three ticks after
/// a crit taken, its share by rank, and a new crit replaces it rather than
/// adding; the Persistent Shield puts a share of a heal on its recipient
/// as an absorb, a new heal's replacing the old (Thistle Tea's
/// `proc_spell_dbc_test.exs`).
#[tokio::test]
#[ignore = "extended"]
async fn blessed_recovery_and_the_persistent_shield() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_a, p, map) = kit::player(&realm, "P5DB", "Blessed", Race::Human, Class::Priest).await;
    let (_b, f, _) = kit::player(&realm, "P5DC", "Shielded", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, p, 60, &[]);
            kit::ready(m, f, 60, &[]);
            let kobold = kit::dummy(m, p, 60, 3.0);
            let mut recovery = Vec::new();
            for (talent, heal) in BLESSED_RECOVERY {
                auras::remove_where(m, p, |h| {
                    BLESSED_RECOVERY
                        .iter()
                        .any(|(t, x)| *t == h.spell || *x == h.spell)
                });
                kit::apply(m, p, p, talent);
                let crit = |m: &mut Map, damage: u32| {
                    procs::trigger(
                        m,
                        Event {
                            actor: kobold,
                            target: p,
                            actor_flags: procs::PROC_DONE_MELEE,
                            target_flags: procs::PROC_TAKEN_MELEE | procs::PROC_TAKEN_DAMAGE,
                            ex: EX_CRITICAL_HIT,
                            damage,
                            school: 1,
                            ..Default::default()
                        },
                    );
                };
                crit(m, 300);
                let first = holder(m, p, heal).map(|h| (h.effects[0].amount, h.duration));
                crit(m, 600);
                let second = holder(m, p, heal).map(|h| h.effects[0].amount);
                let holders = auras::auras(m, p)
                    .map_or(0, |a| a.holders.iter().filter(|h| h.spell == heal).count());
                recovery.push((talent, first, second, holders));
            }
            // The shield, from two heals' landings on the friend.
            kit::apply(m, p, p, PERSISTENT_SHIELD);
            let healed = |m: &mut Map, amount: u32| {
                procs::trigger(
                    m,
                    Event {
                        actor: p,
                        target: f,
                        actor_flags: procs::PROC_DONE_POSITIVE_SPELL,
                        target_flags: procs::PROC_TAKEN_POSITIVE_SPELL,
                        ex: EX_NORMAL_HIT,
                        spell: Some(2050),
                        damage: amount,
                        school: 2,
                        ..Default::default()
                    },
                );
                holder(m, f, PERSISTENT_SHIELD_ABSORB).map(|h| h.effects[0].amount)
            };
            let shields = (
                healed(m, 1_000),
                healed(m, 400),
                has(m, p, PERSISTENT_SHIELD_ABSORB),
            );
            (recovery, shields)
        })
        .await
        .unwrap();
    let (recovery, (first_shield, second_shield, on_priest)) = out;
    for (i, (talent, first, second, holders)) in recovery.into_iter().enumerate() {
        let pct = [8, 16, 25][i];
        let (amount, duration) = first.unwrap_or_else(|| panic!("{talent}: no heal"));
        assert!((amount - 300 * pct / 300).abs() <= 1, "{talent}: {amount}");
        assert_eq!(
            duration,
            Some(6_000),
            "{talent}: three ticks over six seconds"
        );
        let second = second.unwrap();
        assert!(
            (second - 600 * pct / 300).abs() <= 1,
            "{talent}: replaced, not added: {second}"
        );
        assert_eq!(holders, 1, "{talent}: one heal");
    }
    assert_eq!(first_shield, Some(150), "15% of the heal");
    assert_eq!(second_shield, Some(60), "replaced by the next");
    assert!(!on_priest, "on its recipient");
}
