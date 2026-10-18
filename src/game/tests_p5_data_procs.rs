//! Phase 5's proc damage, Thistle Tea's `ProcDamageTest` ported: a holy
//! aura that strikes back for 50 (Holy Shield's kind) on what it procs on,
//! with its chance, charges and cooldown; a block-only one; the damage
//! itself, which reflects not, procs no more, is absorbed and kept off by
//! immunity, and can miss only from a harmful aura.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{CUSTOM_NEGATIVE, ProcEvent, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::procs::{self, *};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, health, put, set};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

/// The striking auras: taken blows (Thistle Tea's), dealt blows, harmful
/// spells either way, a block-only one, one of no chance, one with a
/// second's cooldown, a harmful one, and the target's own proc on spells
/// taken; a mirror, a holy shield of 30 and a holy immunity.
const STRIKES: u32 = 96_850;
const STRIKES_OUT: u32 = 96_851;
const STRIKES_SPELLS: u32 = 96_852;
const STRIKES_BLOCKS: u32 = 96_853;
const STRIKES_NEVER: u32 = 96_854;
const STRIKES_COOLING: u32 = 96_855;
const STRIKES_HOSTILE: u32 = 96_856;
const TARGETS_PROC: u32 = 96_857;
const MIRROR: u32 = 96_858;
const HOLY_WARD: u32 = 96_859;
const HOLY_IMMUNE: u32 = 96_860;
const FIREBALL: u32 = 96_861;

fn strikes(id: u32, flags: u32) -> SpellTemplate {
    let mut s = aura_spell(
        id,
        Some(10_000),
        &[
            (AURA_MOD_BLOCK_PERCENT, 0, 0),
            (AURA_PROC_TRIGGER_DAMAGE, 0, 0),
        ],
    );
    s.school = 1;
    s.dmg_class = 1;
    s.proc_flags = flags;
    s.proc_chance = 100;
    s.proc_charges = 3;
    s.effects[1].base_points = 49;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut never = strikes(STRIKES_NEVER, PROC_TAKEN_MELEE);
    never.proc_chance = 0;
    let mut hostile = strikes(STRIKES_HOSTILE, PROC_TAKEN_MELEE);
    hostile.custom_flags = CUSTOM_NEGATIVE;
    hostile.proc_charges = 0;
    let mut targets = strikes(TARGETS_PROC, PROC_TAKEN_NEGATIVE_SPELL);
    targets.id = TARGETS_PROC;
    let mut fireball = SpellTemplate {
        id: FIREBALL,
        name: "Fireball".into(),
        school: 2,
        dmg_class: 1,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    fireball.effects[0] = crate::game::data::spells::SpellEffect {
        effect: crate::game::map::spells::EFFECT_SCHOOL_DAMAGE,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    add_spells(
        &mut d,
        [
            strikes(STRIKES, PROC_TAKEN_MELEE | PROC_TAKEN_MELEE_SPELL),
            strikes(STRIKES_OUT, PROC_DONE_MELEE | PROC_DONE_MELEE_SPELL),
            strikes(
                STRIKES_SPELLS,
                PROC_DONE_NEGATIVE_SPELL | PROC_TAKEN_NEGATIVE_SPELL,
            ),
            strikes(STRIKES_BLOCKS, PROC_TAKEN_MELEE),
            never,
            strikes(STRIKES_COOLING, PROC_TAKEN_MELEE),
            hostile,
            targets,
            aura_spell(MIRROR, None, &[(AURA_REFLECT_SPELLS, 0, 0)]),
            aura_spell(HOLY_WARD, None, &[(AURA_SCHOOL_ABSORB, 2, 0)]),
            aura_spell(HOLY_IMMUNE, None, &[(AURA_SCHOOL_IMMUNITY, 2, 0)]),
            fireball,
        ],
    );
    d.spells.proc_events.insert(
        STRIKES_BLOCKS,
        ProcEvent {
            ex: EX_BLOCK,
            ..Default::default()
        },
    );
    d.spells.proc_events.insert(
        STRIKES_COOLING,
        ProcEvent {
            cooldown: 1_000,
            ..Default::default()
        },
    );
    d
}

/// A blow from `actor` on `target` of `ex`, or a spell's.
fn blow(actor: Guid, target: Guid, flags: (u32, u32), ex: u32, spell: Option<u32>) -> Event {
    Event {
        actor,
        target,
        actor_flags: flags.0,
        target_flags: flags.1,
        ex,
        spell,
        damage: if ex & (EX_NORMAL_HIT | EX_CRITICAL_HIT) != 0 {
            30
        } else {
            0
        },
        school: if spell.is_some() { 4 } else { 1 },
        ..Default::default()
    }
}

const MELEE: (u32, u32) = (PROC_DONE_MELEE, PROC_TAKEN_MELEE);

fn charges(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    auras::auras(m, g).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.spell == spell)
            .map(|h| h.charges)
    })
}

/// Brute's health lost to what `f` does.
fn hurt(m: &mut Map, g: Guid, f: impl FnOnce(&mut Map)) -> u32 {
    set(m, g, crate::game::fields::UNIT_FIELD_HEALTH, 100_000);
    f(m);
    100_000 - health(m, g)
}

/// Thistle Tea's procs: one on blows dealt strikes the victim on a landed
/// blow (a charge spent), not on a miss, dodge, parry or block, nor on one
/// taken; one on blows taken goes by its chance (none: never), charges and
/// cooldown (a second: not at 999 ms, at a second on a crit, then its last);
/// a block-only one on a full block, not a hit; one on harmful spells both
/// ways. Ended (run out, cancelled, its bearer dead), none.
#[tokio::test]
async fn a_striking_aura_procs_by_its_flags_outcomes_chance_charges_and_cooldown() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let _ = put(m, ga, ga, STRIKES_OUT, &[0, 50]);
            let landed = hurt(m, brute, |m| {
                procs::trigger(m, blow(ga, brute, MELEE, EX_NORMAL_HIT, None))
            });
            let out_charges = charges(m, ga, STRIKES_OUT);
            let mut avoided = Vec::new();
            for ex in [EX_MISS, EX_DODGE, EX_PARRY, EX_BLOCK] {
                avoided.push(hurt(m, brute, |m| {
                    procs::trigger(m, blow(ga, brute, MELEE, ex, None))
                }));
            }
            let retaliates = hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None))
            });
            auras::remove_spell(m, ga, STRIKES_OUT, None);
            // Taken: chance, charges, cooldown.
            let _ = put(m, ga, ga, STRIKES_NEVER, &[0, 50]);
            let never = hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None))
            });
            auras::remove_spell(m, ga, STRIKES_NEVER, None);
            let _ = put(m, ga, ga, STRIKES_COOLING, &[0, 50]);
            let mut cooling = Vec::new();
            for (after, ex) in [
                (0, EX_NORMAL_HIT),
                (999, EX_NORMAL_HIT),
                (1, EX_CRITICAL_HIT),
                (1_000, EX_NORMAL_HIT),
            ] {
                m.now += Duration::from_millis(after);
                let dealt = hurt(m, brute, |m| {
                    procs::trigger(m, blow(brute, ga, MELEE, ex, None))
                });
                cooling.push((dealt, charges(m, ga, STRIKES_COOLING)));
            }
            // Block-only.
            let _ = put(m, ga, ga, STRIKES_BLOCKS, &[0, 50]);
            let hit = hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None))
            });
            let blocked = hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_BLOCK, None))
            });
            let block_charges = charges(m, ga, STRIKES_BLOCKS);
            auras::remove_spell(m, ga, STRIKES_BLOCKS, None);
            // Spells, taken and dealt.
            let _ = put(m, ga, ga, STRIKES_SPELLS, &[0, 50]);
            let spell_flags = (PROC_DONE_NEGATIVE_SPELL, PROC_TAKEN_NEGATIVE_SPELL);
            let taken = hurt(m, brute, |m| {
                procs::trigger(
                    m,
                    blow(brute, ga, spell_flags, EX_NORMAL_HIT, Some(FIREBALL)),
                )
            });
            let dealt = hurt(m, brute, |m| {
                procs::trigger(
                    m,
                    blow(ga, brute, spell_flags, EX_NORMAL_HIT, Some(FIREBALL)),
                )
            });
            let spell_charges = charges(m, ga, STRIKES_SPELLS);
            auras::remove_spell(m, ga, STRIKES_SPELLS, None);
            // Ended.
            let mut ended = Vec::new();
            let _ = put(m, ga, ga, STRIKES, &[0, 50]);
            m.now += Duration::from_secs(10);
            auras::update(m);
            ended.push(hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None))
            }));
            let _ = put(m, ga, ga, STRIKES, &[0, 50]);
            auras::remove_spell(m, ga, STRIKES, None);
            ended.push(hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None))
            }));
            let _ = put(m, ga, ga, STRIKES, &[0, 50]);
            crate::game::map::combat::deal_damage(m, brute, ga, 1_000_000);
            ended.push(hurt(m, brute, |m| {
                procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None))
            }));
            (
                (landed, out_charges, avoided, retaliates),
                (never, cooling),
                (hit, blocked, block_charges),
                (taken, dealt, spell_charges),
                ended,
            )
        })
        .await
        .unwrap();
    let ((landed, out_charges, avoided, retaliates), (never, cooling), blocks, spells, ended) = got;
    assert_eq!(
        (landed, out_charges),
        (50, Some(2)),
        "a landed blow, a charge"
    );
    assert_eq!(avoided, vec![0; 4], "nothing for what didn't land");
    assert_eq!(retaliates, 0, "not on a blow taken");
    assert_eq!(never, 0, "no chance, never");
    assert_eq!(
        cooling,
        vec![(50, Some(2)), (0, Some(2)), (50, Some(1)), (50, None)],
        "a second's cooldown, its charges spent"
    );
    assert_eq!(blocks, (0, 50, Some(2)), "a full block, not a hit");
    assert_eq!(spells, (50, 50, Some(1)), "spells either way");
    assert_eq!(ended, vec![0; 3], "run out, cancelled, dead");
}

/// Thistle Tea's damage: the carrier's, of its effect's points (50); it
/// isn't reflected and procs nothing of its target's (its charges kept);
/// a holy ward takes 30 of it; a holy immunity keeps it off; a corpse
/// takes none. A helpful aura's never misses; a harmful one's can.
#[tokio::test]
async fn proc_damage_reflects_not_procs_not_is_absorbed_and_misses_only_from_harm() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let struck =
                |m: &mut Map| procs::trigger(m, blow(brute, ga, MELEE, EX_NORMAL_HIT, None));
            let _ = put(m, ga, ga, STRIKES, &[0, 50]);
            let _ = put(m, brute, brute, MIRROR, &[100]);
            let _ = put(m, brute, brute, TARGETS_PROC, &[0, 50]);
            let mine = health(m, ga);
            let plain = hurt(m, brute, struck);
            let unreflected = health(m, ga) == mine;
            let theirs = charges(m, brute, TARGETS_PROC);
            auras::remove_spell(m, brute, MIRROR, None);
            auras::remove_spell(m, brute, TARGETS_PROC, None);
            let _ = put(m, brute, brute, HOLY_WARD, &[30]);
            let warded = hurt(m, brute, struck);
            auras::remove_spell(m, brute, HOLY_WARD, None);
            let _ = put(m, brute, brute, HOLY_IMMUNE, &[0]);
            let immune = hurt(m, brute, struck);
            auras::remove_spell(m, brute, HOLY_IMMUNE, None);
            auras::remove_spell(m, ga, STRIKES, None);
            // Rolled, against a brute far above: a helpful aura's always
            // lands, a harmful one's not.
            m.spells.rolls = Some(true);
            set(m, brute, crate::game::fields::UNIT_FIELD_LEVEL, 63);
            let mut misses = [0, 0];
            for (i, spell) in [STRIKES_COOLING, STRIKES_HOSTILE].into_iter().enumerate() {
                for _ in 0..40 {
                    auras::remove_spell(m, ga, spell, None);
                    let _ = put(m, ga, ga, spell, &[0, 50]);
                    if hurt(m, brute, struck) == 0 {
                        misses[i] += 1;
                    }
                }
                auras::remove_spell(m, ga, spell, None);
            }
            m.spells.rolls = Some(false);
            // A corpse.
            let _ = put(m, ga, ga, STRIKES, &[0, 50]);
            crate::game::map::combat::deal_damage(m, ga, brute, 10_000_000);
            let before = health(m, brute);
            struck(m);
            let corpse = before - health(m, brute);
            (plain, unreflected, theirs, warded, immune, misses, corpse)
        })
        .await
        .unwrap();
    let (plain, unreflected, theirs, warded, immune, misses, corpse) = got;
    assert_eq!(plain, 50, "the effect's points");
    assert!(unreflected, "not thrown back");
    assert_eq!(theirs, Some(3), "it procs nothing of the target's");
    assert_eq!(warded, 20, "30 absorbed");
    assert_eq!(immune, 0);
    assert_eq!(misses[0], 0, "a helpful aura's never misses");
    assert!(misses[1] > 0, "a harmful one's can");
    assert_eq!(corpse, 0);
}

/// Pyroclasm's ranks; warlock spells of Rain of Fire's flag, Hellfire's,
/// Soul Fire's look, another flag, and a mage's of Hellfire's flag.
const PYROCLASM: [(u32, f32); 2] = [(18_096, 13.0), (18_073, 26.0)];
const RAIN: u32 = 96_870;
const HELLFIRE: u32 = 96_871;
const SOUL_FIRE: u32 = 96_872;
const SHADOW_BOLT: u32 = 96_873;
const MAGES: u32 = 96_874;
const MAGES_LOOK: u32 = 96_875;

fn pyro_data() -> GameData {
    let mut d = data();
    let warlock = |id: u32, family: u32, flags: u64, icon: u32, visual: u32| {
        let mut s = aura_spell(id, Some(1_000), &[(AURA_DUMMY, 0, 0)]);
        s.family_name = family;
        s.family_flags = flags;
        s.spell_icon = icon;
        s.spell_visual = visual;
        s
    };
    add_spells(
        &mut d,
        [
            warlock(RAIN, 5, 0x20, 0, 0),
            warlock(HELLFIRE, 5, 0x40, 0, 0),
            warlock(SOUL_FIRE, 5, 0x40, 184, 2253),
            warlock(SHADOW_BOLT, 5, 0x1, 0, 0),
            warlock(MAGES, 3, 0x40, 0, 0),
            warlock(MAGES_LOOK, 3, 0, 184, 2253),
        ],
    );
    for (id, _) in PYROCLASM {
        add_spells(&mut d, [aura_spell(id, None, &[(AURA_DUMMY, 0, 0)])]);
    }
    d
}

/// Thistle Tea's Pyroclasm: each rank's chance (13%, 26%) spread over
/// Rain of Fire's four ticks, Hellfire's fifteen, Soul Fire's one; none
/// for oneself, the dead, no spell, another warlock spell or a mage's.
#[tokio::test]
async fn pyroclasms_chance_is_spread_over_its_spells_ticks() {
    use crate::game::map::proc_scripts::pyroclasm_chance;
    let (_realm, _a, ga, map) = setup_with(pyro_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let mut chances = Vec::new();
            for (id, _) in PYROCLASM {
                for spell in [RAIN, HELLFIRE, SOUL_FIRE] {
                    chances.push(pyroclasm_chance(m, ga, brute, id, Some(spell)));
                }
            }
            let mut none = vec![
                pyroclasm_chance(m, ga, ga, 18_073, Some(HELLFIRE)),
                pyroclasm_chance(m, ga, brute, 18_073, None),
            ];
            for spell in [SHADOW_BOLT, MAGES, MAGES_LOOK] {
                none.push(pyroclasm_chance(m, ga, brute, 18_073, Some(spell)));
            }
            crate::game::map::combat::deal_damage(m, ga, brute, 10_000_000);
            none.push(pyroclasm_chance(m, ga, brute, 18_073, Some(HELLFIRE)));
            (chances, none)
        })
        .await
        .unwrap();
    let (chances, none) = got;
    let mut want = Vec::new();
    for (_, pct) in PYROCLASM {
        for ticks in [4.0, 15.0, 1.0] {
            want.push(Some(pct / ticks));
        }
    }
    assert_eq!(chances, want);
    assert!(none.iter().all(Option::is_none), "{none:?}");
}

/// A proc on what an absorb took (`procEx` 0x400) on blows and spells
/// taken; physical shields of 40 and of everything; a mark.
const ON_ABSORB: u32 = 96_880;
const ABSORB_MARK: u32 = 96_881;
const SHIELD_40: u32 = 96_882;
const SHIELD_ALL: u32 = 96_883;
const STRIKE: u32 = 96_884;

fn absorb_data() -> GameData {
    let mut d = data();
    let mut proc = crate::game::tests_p3_aura2::proc_spell(ON_ABSORB, 0x2_02A8, ABSORB_MARK);
    proc.proc_charges = 3;
    let mut strike = SpellTemplate {
        id: STRIKE,
        name: "Strike".into(),
        school: 0,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    strike.effects[0] = crate::game::data::spells::SpellEffect {
        effect: crate::game::map::spells::EFFECT_SCHOOL_DAMAGE,
        base_points: 99,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    add_spells(
        &mut d,
        [
            proc,
            aura_spell(ABSORB_MARK, Some(10_000), &[(AURA_DUMMY, 0, 0)]),
            aura_spell(SHIELD_40, None, &[(AURA_SCHOOL_ABSORB, 1, 0)]),
            aura_spell(SHIELD_ALL, None, &[(AURA_SCHOOL_ABSORB, 1, 0)]),
            strike,
        ],
    );
    d.spells.proc_events.insert(
        ON_ABSORB,
        ProcEvent {
            ex: EX_ABSORB,
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's absorbed hits: a physical spell, ability or shot of 100
/// that a shield takes some or all of (a shield of 40 used up, one of 100
/// used up) procs what wants an absorb (a charge spent); one no shield
/// touched doesn't.
#[tokio::test]
async fn an_absorbed_spell_or_ability_keeps_its_absorb_for_procs() {
    let mut d = absorb_data();
    let (_realm, _a, ga, map) = {
        let base = d.spells.spells.get(&STRIKE).cloned().unwrap();
        for (i, class) in [1u32, 2, 3].into_iter().enumerate() {
            let mut s = base.clone();
            s.id = STRIKE + 10 + i as u32;
            s.dmg_class = class;
            d.spells.spells.insert(s.id, s);
        }
        setup_with(d, Class::Warrior).await
    };
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let mut out = Vec::new();
            for i in 0..3 {
                let spell = STRIKE + 10 + i;
                for (shield, amount) in [(None, 0), (Some(SHIELD_40), 40), (Some(SHIELD_ALL), 100)]
                {
                    let _ = put(m, ga, ga, ON_ABSORB, &[0]);
                    if let Some(s) = shield {
                        let _ = put(m, ga, ga, s, &[amount]);
                    }
                    set(m, ga, crate::game::fields::UNIT_FIELD_MAXHEALTH, 5_000);
                    set(m, ga, crate::game::fields::UNIT_FIELD_HEALTH, 5_000);
                    set(m, ga, crate::game::fields::UNIT_FIELD_RESISTANCES, 0);
                    crate::game::tests_p3_spells_kit::fresh(m, brute);
                    crate::game::map::spells::cast_triggered(m, brute, spell, Some(ga));
                    let marked = auras::auras(m, ga).is_some_and(|a| a.has_spell(ABSORB_MARK));
                    // What the shield has left (its whole hundred, or the forty, taken).
                    let left = shield.and_then(|s| {
                        auras::auras(m, ga)
                            .and_then(|a| a.holders.iter().find(|h| h.spell == s).cloned())
                            .map(|h| h.effects[0].left)
                    });
                    out.push((i, amount, left, marked, charges(m, ga, ON_ABSORB)));
                    for s in [ON_ABSORB, ABSORB_MARK, SHIELD_40, SHIELD_ALL] {
                        auras::remove_spell(m, ga, s, None);
                    }
                }
            }
            out
        })
        .await
        .unwrap();
    for (i, amount, left, marked, charges) in got {
        assert!(left.is_none_or(|l| l == 0), "class {i}: {amount} absorbed");
        assert_eq!(marked, amount > 0, "class {i}: {amount}");
        assert_eq!(charges, Some(if amount > 0 { 2 } else { 3 }), "class {i}");
    }
}
