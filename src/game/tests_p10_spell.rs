//! Round 10, against Thistle Tea's spell tests where wvs differed from
//! both it and mangos-classic (`trap_proc_dbc_test.exs`,
//! `radius_test.exs`, `spell_effect_test.exs`, `dream_dragon_test.exs`,
//! `cast_context_test.exs`): Frost Trap's pulse sets off Entrapment; a
//! totem's party aura reaches by its shaman's radius modifiers as they
//! were when it was set down; a spawn-in ends its caster's spawning; an
//! aura that lasts through death goes on a corpse; a summoned demon comes
//! down where it was aimed, for an hour; and which abilities roll with
//! their weapon's skill.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, cast_fresh, pass};
use super::tests_p6_wspell::put;
use crate::game::data::spells::{self as sd, ProcEvent, SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::creature::Life;
use crate::game::map::spells::{self, Targets, mods, outcome};
use crate::game::map::{Map, auras, pets, procs};

/// Entrapment's kind, sure to go off on a trap's activation of the
/// hunters' traps' family, and the root it gives (of five stacks, to count
/// its procs); Frost Trap's area, its pulse every two seconds Frost Trap
/// Aura's script, and the dummy it pulses.
const ENTRAP: u32 = 998_400;
const ROOT: u32 = 998_401;
const FROST: u32 = 998_402;
const PULSE: u32 = 998_403;
/// A totem's summon, its party aura of 20 yards (the shamans' family's
/// first flag), half again to that radius, and the totem.
const SUMMON_TOTEM: u32 = 998_404;
const TOTEM_AURA: u32 = 998_405;
const WIDER: u32 = 998_406;
const TOTEM: u32 = 9_960;
/// The spawn-ins mangos ends the spawning of (Rookery Whelp's, Birth), and
/// another of the effect.
const ROOKERY_SPAWN: u32 = 15_750;
const BIRTH: u32 = 26_262;
const OTHER_SPAWN: u32 = 998_407;
/// Marks at a unit: one lasting through death, one that may be cast at
/// the dead, one of neither.
const DEATH_PERSISTENT: u32 = 998_408;
const AT_THE_DEAD: u32 = 998_409;
const PLAIN_MARK: u32 = 998_410;
/// Inferno's kind: a demon summoned at the destination.
const SUMMON_DEMON: u32 = 998_411;
const DEMON: u32 = 9_961;
const R10: u32 = 13;
const R20: u32 = 920;
/// `SPELL_EFFECT_SPAWN`, and `UNIT_FLAG_SPAWNING`.
const EFFECT_SPAWN: u32 = 46;
const UNIT_FLAG_SPAWNING: u32 = 0x2;

fn data(d: &mut GameData) {
    d.spells.radii.insert(R10, 10.0);
    d.spells.radii.insert(R20, 20.0);
    for (entry, name) in [(TOTEM, "Made-up Totem"), (DEMON, "Made-up Infernal")] {
        d.creature_templates.insert(
            entry,
            CreatureTemplate {
                entry,
                name: name.into(),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                level_min: 60,
                level_max: 60,
                faction: 7,
                ..Default::default()
            },
        );
    }
    // Entrapment, its root, Frost Trap's area and its pulse.
    let mut entrap = bare(ENTRAP, 0);
    entrap.family_name = 9;
    entrap.duration_index = LASTS_LONG;
    entrap.proc_flags = procs::PROC_TRAP;
    entrap.proc_chance = 100;
    effect(
        &mut entrap,
        0,
        SpellEffect {
            trigger_spell: ROOT,
            ..aura(auras::AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    d.spells.proc_events.insert(
        ENTRAP,
        ProcEvent {
            family: 9,
            family_mask: [0x14, 0, 0],
            ..Default::default()
        },
    );
    let mut root = bare(ROOT, 0);
    root.duration_index = LASTS_LONG;
    root.stack_amount = 5;
    effect(&mut root, 0, aura(auras::AURA_MOD_ROOT, 0, 0, 6));
    let mut frost = bare(FROST, 4);
    frost.family_name = 9;
    frost.family_flags = 0x10;
    frost.script_name = "spell_hunter_frost_trap_aura".into();
    frost.duration_index = LASTS_15S;
    effect(
        &mut frost,
        0,
        SpellEffect {
            effect: spells::EFFECT_PERSISTENT_AREA_AURA,
            aura: auras::AURA_PERIODIC_TRIGGER_SPELL,
            amplitude: 2_000,
            trigger_spell: PULSE,
            target_a: 28,
            radius_index: R10,
            ..Default::default()
        },
    );
    let mut pulse = bare(PULSE, 0);
    effect(
        &mut pulse,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUMMY,
            target_a: 25,
            ..Default::default()
        },
    );
    // The totem, its aura and the shaman's radius.
    let mut summon = bare(SUMMON_TOTEM, 0);
    summon.duration_index = LASTS_LONG;
    effect(
        &mut summon,
        0,
        SpellEffect {
            effect: pets::EFFECT_SUMMON_TOTEM_SLOT1,
            misc_value: TOTEM as i32,
            base_points: 4,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut totem_aura = bare(TOTEM_AURA, 0);
    totem_aura.family_name = 11;
    totem_aura.family_flags = 1;
    effect(
        &mut totem_aura,
        0,
        SpellEffect {
            effect: 35,
            radius_index: R20,
            ..aura(auras::AURA_MOD_STAT, 0, 10, 1)
        },
    );
    d.pets.totem_spells.insert(TOTEM, TOTEM_AURA);
    let mut wider = bare(WIDER, 0);
    wider.family_name = 11;
    wider.duration_index = LASTS_LONG;
    effect(
        &mut wider,
        0,
        SpellEffect {
            item_type: 1,
            ..aura(auras::AURA_ADD_PCT_MODIFIER, mods::OP_RADIUS as i32, 50, 1)
        },
    );
    // Spawn-ins.
    let spawn_in = |id: u32| {
        let mut s = bare(id, 0);
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: EFFECT_SPAWN,
                target_a: 1,
                ..Default::default()
            },
        );
        s
    };
    // Marks.
    let mark = |id: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 6));
        s
    };
    let mut persistent = mark(DEATH_PERSISTENT);
    persistent.attributes[3] |= sd::ATTR_EX3_DEATH_PERSISTENT;
    let mut at_the_dead = mark(AT_THE_DEAD);
    at_the_dead.attributes[2] |= sd::ATTR_EX2_ALLOW_DEAD_TARGET;
    // The demon.
    let mut demon = bare(SUMMON_DEMON, 0);
    demon.duration_index = LASTS_15S;
    effect(
        &mut demon,
        0,
        SpellEffect {
            effect: pets::EFFECT_SUMMON_DEMON,
            misc_value: DEMON as i32,
            radius_index: R10,
            ..Default::default()
        },
    );
    insert(
        d,
        [
            entrap,
            root,
            frost,
            pulse,
            summon,
            totem_aura,
            wider,
            spawn_in(ROOKERY_SPAWN),
            spawn_in(BIRTH),
            spawn_in(OTHER_SPAWN),
            persistent,
            at_the_dead,
            mark(PLAIN_MARK),
            demon,
        ],
    );
}

fn stacks(m: &Map, g: Guid, spell: u32) -> u32 {
    kit::holder(m, g, spell).map_or(0, |h| h.stacks)
}

fn kill(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m.creature_mut(g) {
        c.life = Life::Dead {
            corpse_until: now + Duration::from_secs(600),
            respawn_at: now + Duration::from_secs(600),
        };
    }
    kit::set_health(m, g, 0);
}

/// A Frost Trap's area about a kobold pulses every two seconds, and each
/// pulse is the trap's activation for its hunter's Entrapment: nothing at
/// 1.999 s, a root at 2 s; the trap laid again at 2.25 s rolls nothing of
/// its own, and the next root comes at 4 s, one only (Thistle Tea's "Frost
/// Trap checks every two seconds without rerolling on area refreshes";
/// mangos-classic's dummy 18350, its periodic trigger, rolling
/// Entrapment).
#[tokio::test]
async fn a_frost_traps_pulse_sets_off_entrapment_each_period() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P10S1", "Trapper", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            kit::apply(m, g, g, ENTRAP);
            let k = kit::dummy(m, g, 60, 3.0);
            let at = m.position_of(k).unwrap();
            spells::cast_triggered_at(m, g, FROST, at);
            let mut seen = Vec::new();
            pass(m, 1_999);
            seen.push(stacks(m, k, ROOT));
            pass(m, 1);
            seen.push(stacks(m, k, ROOT));
            pass(m, 250);
            spells::cast_triggered_at(m, g, FROST, at);
            seen.push(stacks(m, k, ROOT));
            pass(m, 1_749);
            seen.push(stacks(m, k, ROOT));
            pass(m, 1);
            seen.push(stacks(m, k, ROOT));
            pass(m, 250);
            seen.push(stacks(m, k, ROOT));
            seen
        })
        .await
        .unwrap();
    assert_eq!(out, vec![0, 1, 1, 1, 2, 2]);
}

/// A totem set down under its shaman's half again to its aura's radius
/// reaches 30 yards: the shaman 25 yards off has its aura, and keeps it
/// once the modifier is gone. Set down again without it, it reaches 20,
/// and the shaman there has none (Thistle Tea's "totems snapshot owner
/// modifiers and refresh using their retained radius"; mangos-classic's
/// `AreaAura` taking `SPELLMOD_RADIUS` as the totem casts it).
#[tokio::test]
async fn a_totems_aura_reaches_by_its_shamans_radius_as_it_was_set_down() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P10S2", "Totemic", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[SUMMON_TOTEM]);
            kit::apply(m, g, g, WIDER);
            let set_down = |m: &mut Map| {
                cast_fresh(m, g, SUMMON_TOTEM, Targets::default());
                let totem = m.players[&g].owned.totems[0].expect("the totem");
                let at = m.position_of(totem).unwrap();
                put(
                    m,
                    g,
                    Vector3d {
                        x: at.x + 25.0,
                        ..at
                    },
                );
            };
            let think = |m: &mut Map| {
                m.now += Duration::from_millis(1_100);
                pets::update(m);
                kit::has(m, g, TOTEM_AURA)
            };
            set_down(m);
            let wide = think(m);
            auras::remove_spell(m, g, WIDER, None);
            let kept = think(m);
            set_down(m);
            let narrow = think(m);
            (wide, kept, narrow)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, false), "30 yards, kept, then 20");
}

/// Rookery Whelp's spawn-in and Birth end their caster's spawning: the
/// creature may be fought. Another spell of the effect leaves it be, as
/// Thistle Tea's spawn reveals a concealed creature and mangos-classic's
/// `EffectSpawn` does only for these two (`UNIT_FLAG_SPAWNING`).
#[tokio::test]
async fn a_spawn_in_ends_its_casters_spawning() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P10S3", "Watcher", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let flag = UNIT_FLAG_SPAWNING;
            [ROOKERY_SPAWN, BIRTH, OTHER_SPAWN].map(|spell| {
                let k = kit::dummy(m, g, 60, 5.0);
                if let Some(f) = m.unit_fields_mut(k) {
                    f.set_flag(UNIT_FIELD_FLAGS, flag, true);
                }
                spells::cast_triggered(m, k, spell, Some(k));
                m.unit_fields(k)
                    .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, flag))
            })
        })
        .await
        .unwrap();
    assert_eq!(out, [false, false, true]);
}

/// At a kobold's corpse, a mark that lasts through death goes on, and so
/// does one that may be cast at the dead; one of neither doesn't (Thistle
/// Tea's "a corpse keeps a mark that persists through death and sheds any
/// other aura"; mangos-classic's `AddSpellAuraHolder`).
#[tokio::test]
async fn a_corpse_takes_only_an_aura_for_the_dead() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P10S4", "Marker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            let k = kit::dummy(m, g, 60, 5.0);
            kill(m, k);
            [DEATH_PERSISTENT, AT_THE_DEAD, PLAIN_MARK].map(|spell| {
                spells::cast_triggered(m, g, spell, Some(k));
                kit::has(m, k, spell)
            })
        })
        .await
        .unwrap();
    assert_eq!(out, [true, true, false]);
}

/// A summoned demon (Inferno's kind) comes down where its cast was aimed,
/// ten yards off, not beside its caster, and is the caster's for an hour
/// whatever the spell's own time (Thistle Tea's "summon-demon effects
/// create a temporary owned summon at the destination"; mangos-classic's
/// `EffectSummonDemon`: the destination, 3,600,000 ms).
#[tokio::test]
async fn a_summoned_demon_comes_down_where_aimed_for_an_hour() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P10S5", "Infernal", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let here = m.players[&g].position();
            let at = Vector3d {
                x: here.x + 10.0,
                ..here
            };
            spells::cast_triggered_at(m, g, SUMMON_DEMON, at);
            let now = m.now;
            m.creatures
                .iter()
                .find(|(_, c)| c.entry == DEMON)
                .map(|(_, c)| {
                    (
                        c.owner == Some(g),
                        crate::game::map::combat::dist(c.position, at) < 0.01,
                        c.pet
                            .as_deref()
                            .and_then(|p| p.expires)
                            .map(|t| t.duration_since(now)),
                    )
                })
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        Some((true, true, Some(pets::DEMON_LASTS))),
        "the caster's, at the place, an hour"
    );
}

fn ability(dmg_class: u32, ranged_slot: bool, weapon: bool) -> SpellTemplate {
    let mut s = bare(1, 0);
    s.dmg_class = dmg_class;
    if ranged_slot {
        s.attributes[0] |= sd::ATTR_RANGED;
    }
    if weapon {
        s.equipped_item_class = 2;
    }
    s
}

/// Which abilities roll with their weapon's skill, and which with the most
/// their caster's level allows: a melee one always, and one wanting a
/// weapon; a ranged one only of the ranged slot (Thistle Tea's
/// "nonweapon abilities use level skill while weapon attacks retain
/// learned skill"; mangos-classic's `IsSpellUseWeaponSkill`).
#[test]
fn only_a_weapons_abilities_roll_with_its_skill() {
    let cases = [
        (ability(2, false, false), true),
        (ability(2, false, true), true),
        (ability(3, true, false), true),
        (ability(3, false, true), true),
        (ability(3, false, false), false),
        (ability(1, false, false), false),
    ];
    for (s, weapon) in cases {
        assert_eq!(
            outcome::uses_weapon_skill(&s),
            weapon,
            "class {}, ranged slot {}, weapon {}",
            s.dmg_class,
            s.attr(0, sd::ATTR_RANGED),
            s.equipped_item_class
        );
    }
}
