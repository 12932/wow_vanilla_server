//! Round 9, against Thistle Tea's spell tests of the world where they are
//! 1.12's (`transporter_dbc_test.exs`, `trap_proc_dbc_test.exs`,
//! `wild_object_test.exs`, `spell_caster_locations_test.exs`,
//! `spell_objects_test.exs`, `spell_reception_threat_test.exs` and
//! `spell_units_test.exs`): the Gadgetzan transporter's chain; a trap's
//! one proc and a resisted trap's none; a ground trap's pulse; wild
//! objects; an ownerless trap's cast; an object worked by a spell; the
//! reagents a trigger wears; a missile's threat; one spell from two
//! items; and the creatures and corpses a script's spell names.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, has, pass};
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::scripts::{Condition, ScriptStep};
use crate::game::data::spells::{ProcEvent, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::auras::*;
use crate::game::map::combat::dist;
use crate::game::map::creature::Life;
use crate::game::map::spells::{self, Targets, checks, mods, targets};
use crate::game::map::{Map, death, objects};
use crate::game::tests_p3_combat as p3;

/// The Gadgetzan transporter's outcomes, by the ids `item_scripts` knows:
/// a good arrival, a bad one, the bad one's own outcome, and what that
/// may give.
const GADGETZAN: u32 = 23_441;
const FAILURE: u32 = 23_446;
const ARRIVAL: u32 = 23_448;
const MALFUNCTION: u32 = 23_444;
const EVIL_TWIN: u32 = 23_445;
/// Where each arrival puts its traveller: two places on the eastern
/// kingdoms' map, so the player stays on it.
const LANDING: Vector3d = Vector3d {
    x: -8_900.0,
    y: -100.0,
    z: 82.0,
};
const BAD_LANDING: Vector3d = Vector3d {
    x: -8_850.0,
    y: -150.0,
    z: 82.0,
};
/// Entrapment of rank 5's flags and rule, sure to go off, and the root it
/// gives (of five stacks, to count its procs); a blast of 100 fire about
/// its caster (Explosive Trap's); a burn over time (Immolation Trap's);
/// a ward that resists all.
const ENTRAP: u32 = 997_210;
const TRAP: u32 = 997_211;
const ROOT: u32 = 997_212;
const IMMOLATION: u32 = 997_213;
const WARD: u32 = 997_214;
/// Entrapment's `procFlags` at rank 5: a trap's activation, periodic,
/// harmful spells and areas.
const RANK_5_FLAGS: u32 = 0x25_1000;
/// A ground trap of ten yards checking every two seconds, and its pulse
/// of 10.
const FROST: u32 = 997_215;
const PULSE: u32 = 997_216;
/// Wild objects: two of no end at the caster's place, one 3 yards ahead,
/// one where the caster lies; the objects.
const TWO_WILD: u32 = 997_217;
const AHEAD: u32 = 997_218;
const ONE_WILD: u32 = 997_219;
const WILD_1: u32 = 997_220;
const WILD_2: u32 = 997_221;
/// A trap nobody set, and its spell: a heal at whoever springs it and a
/// wild object at its place.
const TRAP_OBJECT: u32 = 997_222;
const TRAP_SPELL: u32 = 997_223;
/// A spell working the object its script targets name (its custom
/// animation 0), and the object.
const WORK: u32 = 997_224;
const WORKED: u32 = 997_225;
/// A periodic trigger with no reagent, one with a reagent of its own, the
/// spell they trigger (its reagents a mantle, a cowl and a robe), and
/// the reagent.
const DRAIN: u32 = 997_226;
const DRAIN_PAID: u32 = 997_227;
const PAID_FOR: u32 = 997_228;
const MANTLE: u32 = 997_229;
const COWL: u32 = 997_230;
const ROBE: u32 = 997_231;
const TOKEN: u32 = 997_232;
/// A missile of 100 at 20 yards a second; 30 percent less threat.
const MISSILE: u32 = 997_233;
const LESS_THREAT: u32 = 997_234;
/// A damage over time an item casts.
const FROM_ITEMS: u32 = 997_235;
/// A heal at a script's creature near the caster, below 10 percent of
/// its health; the condition.
const MEND: u32 = 997_236;
const BELOW_TENTH: u32 = 997_237;
/// A dummy at a kobold's corpse, and at another entry's; the spell its
/// script casts.
const AT_CORPSE: u32 = 997_238;
const AT_OTHER: u32 = 997_239;
const SECOND: u32 = 997_240;
const OTHER_ENTRY: u32 = 9_978;
/// A script effect at the unit, and the mark its script puts on.
const SCRIPTED: u32 = 997_241;
const MARK: u32 = 997_242;
/// A heal at a script's kobold alone, and with a heal at an ally too.
const SCRIPT_HEAL: u32 = 997_243;
const SCRIPT_HEAL_ALLY: u32 = 997_244;
const NO_END: u32 = 920;
const R10: u32 = 13;
const R3: u32 = 920;
const FIRE: u32 = 2;
const FROST_SCHOOL: u32 = 4;
const GO_GENERIC: u8 = 5;
/// Equipment slots: head, shoulders, chest.
const HEAD: u8 = 0;
const SHOULDERS: u8 = 2;
const CHEST: u8 = 4;
/// `CONDITION_HEALTH_PERCENT`; `SCRIPT_COMMAND_CAST_SPELL`, triggered.
const HEALTH_PERCENT: i32 = 41;
const CAST_SPELL: u32 = 15;
const CAST_TRIGGERED: u32 = 0x2;

/// A spell of one aura on its caster, lasting `duration`.
fn self_aura(id: u32, kind: u32, misc: i32, amount: i32, duration: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = duration;
    effect(&mut s, 0, aura(kind, misc, amount, 1));
    s
}

fn hunters(id: u32, school: u32) -> SpellTemplate {
    let mut s = bare(id, school);
    s.family_name = 9;
    s.family_flags = 4;
    s
}

fn wild(entry: u32, target: u32, radius_index: u32) -> SpellEffect {
    SpellEffect {
        effect: objects::EFFECT_SUMMON_OBJECT_WILD,
        misc_value: entry as i32,
        target_a: target,
        radius_index,
        ..Default::default()
    }
}

fn garment(entry: u32, inventory_type: u32, name: &str) -> ItemTemplate {
    ItemTemplate {
        entry,
        class: 4,
        subclass: 1,
        name: name.into(),
        display_id: 1000,
        inventory_type,
        stackable: 1,
        ..Default::default()
    }
}

/// A script step casting `spell`, triggered, from its source at its
/// target.
fn cast_step(spell: u32) -> ScriptStep {
    ScriptStep {
        command: CAST_SPELL,
        datalong: [spell, CAST_TRIGGERED, 0, 0],
        ..Default::default()
    }
}

/// An effect of `effect` at a script's creature near the caster (38).
fn near_caster(id: u32, effect_id: u32) -> SpellTemplate {
    let mut s = bare(id, 1);
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: effect_id,
            base_points: 9,
            base_dice: 1,
            target_a: 38,
            ..Default::default()
        },
    );
    s
}

fn data(d: &mut GameData) {
    d.spells.durations.insert(
        NO_END,
        SpellDuration {
            base: -1,
            per_level: 0,
            max: -1,
        },
    );
    d.spells.radii.insert(R10, 10.0);
    d.spells.radii.insert(R3, 3.0);
    // The transporter: each arrival at its place, the bad one's own
    // outcome with it, and that one's marker.
    let teleport = SpellEffect {
        effect: spells::EFFECT_TELEPORT_UNITS,
        target_a: 1,
        target_b: 17,
        ..Default::default()
    };
    let mut good = bare(GADGETZAN, 0);
    effect(&mut good, 0, teleport);
    let mut bad = bare(FAILURE, 0);
    effect(&mut bad, 0, teleport);
    effect(
        &mut bad,
        1,
        SpellEffect {
            effect: spells::EFFECT_TRIGGER_SPELL,
            target_a: 1,
            trigger_spell: ARRIVAL,
            ..Default::default()
        },
    );
    let mut arrival = bare(ARRIVAL, 0);
    arrival.duration_index = LASTS_LONG;
    effect(
        &mut arrival,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUMMY,
            target_a: 1,
            ..Default::default()
        },
    );
    effect(&mut arrival, 1, aura(AURA_DUMMY, 0, 1, 1));
    for (spell, to) in [(GADGETZAN, LANDING), (FAILURE, BAD_LANDING)] {
        d.spells
            .target_positions
            .insert(spell, (0, to.x, to.y, to.z, 0.0));
    }
    // Entrapment, its root, the traps and the ward.
    let mut entrap = bare(ENTRAP, 0);
    entrap.family_name = 9;
    entrap.duration_index = LASTS_LONG;
    entrap.proc_flags = RANK_5_FLAGS;
    entrap.proc_chance = 100;
    effect(
        &mut entrap,
        0,
        SpellEffect {
            trigger_spell: ROOT,
            ..aura(AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
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
    effect(&mut root, 0, aura(AURA_MOD_ROOT, 0, 0, 6));
    let mut blast = hunters(TRAP, FIRE);
    effect(
        &mut blast,
        0,
        SpellEffect {
            target_a: 22,
            target_b: 15,
            radius_index: R10,
            ..damage(100)
        },
    );
    let mut burn = hunters(IMMOLATION, FIRE);
    burn.duration_index = LASTS_15S;
    effect(
        &mut burn,
        0,
        SpellEffect {
            amplitude: 3_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 6)
        },
    );
    let ward = self_aura(WARD, AURA_MOD_RESIST_CHANCE, 0x7F, 10_000, LASTS_LONG);
    // The ground trap and its pulse.
    let mut frost = hunters(FROST, FROST_SCHOOL);
    frost.family_flags = 16;
    frost.duration_index = LASTS_15S;
    effect(
        &mut frost,
        0,
        SpellEffect {
            effect: spells::EFFECT_PERSISTENT_AREA_AURA,
            aura: AURA_PERIODIC_TRIGGER_SPELL,
            amplitude: 2_000,
            trigger_spell: PULSE,
            target_a: 28,
            radius_index: R10,
            ..Default::default()
        },
    );
    let mut pulse = bare(PULSE, FROST_SCHOOL);
    effect(&mut pulse, 0, damage(10));
    // Wild objects.
    for entry in [WILD_1, WILD_2] {
        d.gameobject_templates.insert(
            entry,
            crate::game::tests_p3_entity_objects::object(entry, GO_GENERIC, "Wild", &[]),
        );
    }
    let mut two = bare(TWO_WILD, 0);
    two.duration_index = NO_END;
    effect(&mut two, 0, wild(WILD_1, 18, 0));
    effect(&mut two, 1, wild(WILD_2, 18, 0));
    let mut ahead = bare(AHEAD, 0);
    effect(&mut ahead, 0, wild(WILD_1, 47, R3));
    let mut one = bare(ONE_WILD, 0);
    effect(&mut one, 0, wild(WILD_1, 18, 0));
    // The ownerless trap: five yards, its spell, three seconds between,
    // armed at once.
    d.gameobject_templates.insert(
        TRAP_OBJECT,
        crate::game::tests_p3_entity_objects::object(
            TRAP_OBJECT,
            objects::GO_TRAP,
            "Trap",
            &[(2, 5), (3, TRAP_SPELL), (5, 3), (7, 0)],
        ),
    );
    let mut trap_spell = bare(TRAP_SPELL, 1);
    effect(
        &mut trap_spell,
        0,
        SpellEffect {
            effect: spells::EFFECT_HEAL,
            base_points: 99,
            base_dice: 1,
            target_a: 25,
            ..Default::default()
        },
    );
    effect(&mut trap_spell, 1, wild(WILD_1, 0, 0));
    // The object worked, by its script target's row.
    d.gameobject_templates.insert(
        WORKED,
        crate::game::tests_p3_entity_objects::object(WORKED, GO_GENERIC, "Worked", &[]),
    );
    let mut work = bare(WORK, 0);
    effect(
        &mut work,
        0,
        SpellEffect {
            effect: objects::EFFECT_ACTIVATE_OBJECT,
            misc_value: 1,
            target_a: 40,
            ..Default::default()
        },
    );
    d.spells.script_targets.insert(WORK, vec![(0, WORKED)]);
    // The periodic triggers and the spell paid for with worn gear.
    for (entry, kind, name) in [
        (MANTLE, 3, "Mantle"),
        (COWL, 1, "Cowl"),
        (ROBE, 5, "Robe"),
        (TOKEN, 0, "Token"),
    ] {
        d.items.insert(entry, garment(entry, kind, name));
    }
    let trigger = |id: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            SpellEffect {
                amplitude: 1_000,
                trigger_spell: PAID_FOR,
                ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
            },
        );
        s
    };
    let drain = trigger(DRAIN);
    let mut drain_paid = trigger(DRAIN_PAID);
    drain_paid.reagents[0] = (TOKEN, 1);
    let mut paid_for = self_aura(PAID_FOR, AURA_DUMMY, 0, 1, LASTS_LONG);
    paid_for.reagents[..3].copy_from_slice(&[(MANTLE, 1), (COWL, 1), (ROBE, 1)]);
    // The missile and the threat modifier.
    let mut missile = bare(MISSILE, 0);
    missile.speed = 20.0;
    effect(&mut missile, 0, damage(100));
    let less_threat = self_aura(LESS_THREAT, AURA_MOD_THREAT, 127, -30, LASTS_LONG);
    // A damage over time from an item.
    let mut from_items = bare(FROM_ITEMS, 0);
    from_items.duration_index = LASTS_5S;
    effect(
        &mut from_items,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 100, 6)
        },
    );
    // The heal at a kobold below a tenth of its health.
    let mend = near_caster(MEND, spells::EFFECT_HEAL);
    d.spells.script_targets.insert(MEND, vec![(1, KOBOLD)]);
    d.scripts.conditions.insert(
        BELOW_TENTH,
        Condition {
            entry: BELOW_TENTH,
            kind: HEALTH_PERCENT,
            values: [10, 2, 0, 0],
            flags: 0,
        },
    );
    d.spells
        .script_target_conditions
        .insert((MEND, 1, KOBOLD), BELOW_TENTH);
    // The dummies at a corpse, and the script they run.
    let at_corpse = near_caster(AT_CORPSE, spells::EFFECT_DUMMY);
    let at_other = near_caster(AT_OTHER, spells::EFFECT_DUMMY);
    d.spells.script_targets.insert(AT_CORPSE, vec![(2, KOBOLD)]);
    d.spells
        .script_targets
        .insert(AT_OTHER, vec![(2, OTHER_ENTRY)]);
    for id in [AT_CORPSE, AT_OTHER] {
        d.scripts.spell.insert(id, vec![cast_step(SECOND)]);
    }
    let second = self_aura(SECOND, AURA_DUMMY, 0, 1, LASTS_LONG);
    // The script effect and its mark.
    let mut scripted = bare(SCRIPTED, 0);
    effect(
        &mut scripted,
        0,
        SpellEffect {
            effect: spells::EFFECT_SCRIPT_EFFECT,
            target_a: 6,
            ..Default::default()
        },
    );
    d.scripts.spell.insert(SCRIPTED, vec![cast_step(MARK)]);
    let mut mark = bare(MARK, 0);
    mark.duration_index = LASTS_LONG;
    effect(&mut mark, 0, aura(AURA_DUMMY, 0, 1, 6));
    // A heal at a script's kobold, alone and with one at an ally.
    let script_heal = near_caster(SCRIPT_HEAL, spells::EFFECT_HEAL);
    let mut script_heal_ally = near_caster(SCRIPT_HEAL_ALLY, spells::EFFECT_HEAL);
    effect(
        &mut script_heal_ally,
        1,
        SpellEffect {
            effect: spells::EFFECT_HEAL,
            base_points: 9,
            base_dice: 1,
            target_a: 21,
            ..Default::default()
        },
    );
    for id in [SCRIPT_HEAL, SCRIPT_HEAL_ALLY] {
        d.spells.script_targets.insert(id, vec![(1, KOBOLD)]);
    }
    insert(
        d,
        [
            good,
            bad,
            arrival,
            self_aura(MALFUNCTION, AURA_DUMMY, 0, 1, LASTS_LONG),
            self_aura(EVIL_TWIN, AURA_DUMMY, 0, 1, LASTS_LONG),
            entrap,
            root,
            blast,
            burn,
            ward,
            frost,
            pulse,
            two,
            ahead,
            one,
            trap_spell,
            work,
            drain,
            drain_paid,
            paid_for,
            missile,
            less_threat,
            from_items,
            mend,
            at_corpse,
            at_other,
            second,
            scripted,
            mark,
            script_heal,
            script_heal_ally,
        ],
    );
}

/// A creature dead, its corpse lying where it fell.
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

/// `g` at `health` of `max`.
fn hurt(m: &mut Map, g: Guid, health: u32, max: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, max);
        f.set_u32(UNIT_FIELD_HEALTH, health);
    }
}

/// The objects `spell` raised: each one's entry, owner, whether it ends,
/// and place, by entry.
fn raised(m: &Map, spell: u32) -> Vec<(u32, Option<Guid>, bool, Vector3d)> {
    let mut out: Vec<_> = m
        .objects
        .summoned
        .iter()
        .filter(|s| s.spell == spell)
        .filter_map(|s| {
            let o = m.gameobject(s.guid)?;
            Some((o.entry, s.owner, s.expires.is_some(), o.position))
        })
        .collect();
    out.sort_by_key(|x| x.0);
    out
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    dist(a, b) < 0.01
}

/// An object of `entry` a yard before `g`, nobody's, once `c` is told of
/// it.
async fn worked_object(
    c: &mut crate::game::harness::Client,
    map: &crate::game::map::MapHandle,
    g: Guid,
) -> Guid {
    let obj = map
        .query(move |m| {
            let me = m.players[&g].position();
            let at = Vector3d {
                x: me.x + 1.0,
                ..me
            };
            objects::summon(m, None, WORKED, at, 0.0, 0, 0, 0, None)
        })
        .await
        .unwrap()
        .expect("the object");
    c.sees_on(map, obj).await;
    obj
}

/// The custom animation `obj` was seen to play, if any, within 300 ms.
async fn animation(c: &mut crate::game::harness::Client, obj: Guid) -> Option<u32> {
    c.gets(Duration::from_millis(300), |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_CUSTOM_ANIM(a) if a.guid == obj => {
            Some(a.animation_id)
        }
        _ => None,
    })
    .await
}

/// The good arrival puts its traveller at its place and nothing more; the
/// bad one puts them at its own place and triggers the arrival, which
/// goes off (its marker on) and rolls its own outcome, at most one of a
/// malfunction or an evil twin (Thistle Tea's "Gadgetzan child spells
/// preserve teleport requests and the arrival chain"; mangos-classic's
/// `EffectTeleportUnits` to `spell_target_position` and
/// `EffectTriggerSpell`; vmangos's transporter dummies).
#[tokio::test]
async fn a_failed_transporter_teleports_and_triggers_its_arrival() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W1", "Traveller", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let place = |m: &Map| m.players[&g].position();
            spells::cast_triggered(m, g, GADGETZAN, Some(g));
            let good = (place(m), has(m, g, ARRIVAL));
            spells::cast_triggered(m, g, FAILURE, Some(g));
            let outcomes = [MALFUNCTION, EVIL_TWIN]
                .iter()
                .filter(|s| has(m, g, **s))
                .count();
            let bad = (place(m), has(m, g, ARRIVAL), outcomes);
            (good, bad)
        })
        .await
        .unwrap();
    let ((good_at, good_arrival), (bad_at, bad_arrival, outcomes)) = out;
    assert!(near(good_at, LANDING), "at its place: {good_at:?}");
    assert!(!good_arrival, "no arrival of its own");
    assert!(near(bad_at, BAD_LANDING), "at the bad place: {bad_at:?}");
    assert!(bad_arrival, "the arrival went off");
    assert!(outcomes <= 1, "one outcome at most: {outcomes}");
}

/// A trap's blast of 100 about its caster takes 100 from a kobold five
/// yards off, and with Entrapment of rank 5's flags and Explosive Trap's
/// family the one gathered hit procs it once: the root is on at one
/// stack, of five (Thistle Tea's "Explosive Trap delivers one proc check
/// alongside its immediate damage"; mangos-classic's
/// `DoAllEffectOnTarget` and its one `ProcDamageAndSpell`).
#[tokio::test]
async fn a_traps_blast_procs_entrapment_once() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W2", "Trapper", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            kit::apply(m, g, g, ENTRAP);
            let k = kit::dummy(m, g, 60, 5.0);
            let before = kit::health(m, k);
            let pos = m.position_of(k).unwrap();
            spells::cast_triggered_with(
                m,
                g,
                TRAP,
                Targets {
                    unit: Some(k),
                    dest: Some(pos),
                    ..Default::default()
                },
            );
            (
                before - kit::health(m, k),
                kit::holder(m, k, ROOT).map(|h| h.stacks),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (100, Some(1)), "100 dealt, one proc");
}

/// A trap's burn at a kobold that resists everything: nothing is put on,
/// Entrapment (whose flags and family the burn meets) procs nothing, and
/// the trapper is told it was resisted. Unwarded, the burn goes on and
/// Entrapment roots (Thistle Tea's "resisted trap applications cannot
/// report successful activation"; mangos-classic's `SpellHitResult` and
/// `PROC_EX_RESIST`, which no default rule procs on).
#[tokio::test]
async fn a_resisted_trap_puts_nothing_on_and_procs_nothing() {
    let realm = fast_realm(data);
    let (mut c, g, map) = human(&realm, "P9W3", "Resisted", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            kit::apply(m, g, g, ENTRAP);
            let k = kit::dummy(m, g, 60, 5.0);
            kit::apply(m, k, k, WARD);
            spells::cast_triggered(m, g, IMMOLATION, Some(k));
            let resisted = (has(m, k, IMMOLATION), has(m, k, ROOT));
            remove_spell(m, k, WARD, None);
            spells::cast_triggered(m, g, IMMOLATION, Some(k));
            let landed = (has(m, k, IMMOLATION), has(m, k, ROOT));
            (resisted, landed)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (false, false), "nothing on, no root");
    assert_eq!(out.1, (true, true), "unwarded, burnt and rooted");
    assert_eq!(kit::missed(&mut c, IMMOLATION).await, 2, "resisted");
}

/// A ground trap of ten yards, checking every two seconds, pulses at two
/// kobolds inside: nothing before its first period, a pulse each at 2 s;
/// laid again at 2.25 s, nothing new before 4 s and one pulse each at 4 s
/// for the two areas, none at the second's own 4.25 s. One kobold gone
/// out of it and the other dead, the next checks pass them by; back in,
/// one is pulsed again; and once both areas have ended, nothing more
/// (Thistle Tea's "Frost Trap checks every two seconds without rerolling
/// on area refreshes" and "leaving the area, removing the source, expiry
/// and death stop trap checks"; mangos-classic's `PersistentAreaAura`
/// and its periodic trigger).
#[tokio::test]
async fn a_ground_trap_pulses_once_a_period_and_stops_when_left_dead_or_ended() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W4", "Frosty", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            let a = kit::dummy(m, g, 60, 3.0);
            let b = kit::dummy(m, g, 60, 2.0);
            let here = m.players[&g].position();
            let t0 = m.now;
            let lost = |m: &Map, k: Guid| 1_000_000 - kit::health(m, k);
            let ticked = |m: &Map, k: Guid| {
                m.spells
                    .area_ticked
                    .get(&(g, FROST, 0, k))
                    .map(|t| t.duration_since(t0).as_millis() as u64)
            };
            let seen = |m: &Map| (lost(m, a), lost(m, b), ticked(m, a), ticked(m, b));
            spells::cast_triggered_at(m, g, FROST, here);
            let mut steps = Vec::new();
            pass(m, 1_999);
            steps.push(seen(m));
            pass(m, 1);
            steps.push(seen(m));
            pass(m, 250);
            spells::cast_triggered_at(m, g, FROST, here);
            pass(m, 1_749);
            steps.push(seen(m));
            pass(m, 1);
            steps.push(seen(m));
            pass(m, 250);
            steps.push(seen(m));
            // One out, the other dead.
            let out_of_it = Vector3d {
                x: here.x + 30.0,
                ..here
            };
            crate::game::map::scripts::place(m, a, out_of_it, 0.0);
            kill(m, b);
            pass(m, 2_000);
            let left_and_dead = (lost(m, a), ticked(m, a), ticked(m, b));
            // Back in, until both have ended.
            crate::game::map::scripts::place(m, a, here, 0.0);
            while m.now < t0 + Duration::from_millis(17_500) {
                pass(m, 250);
            }
            let ended = (m.spells.areas.is_empty(), lost(m, a), ticked(m, a));
            pass(m, 4_000);
            let after = (lost(m, a), ticked(m, a));
            (steps, left_and_dead, ended, after)
        })
        .await
        .unwrap();
    let (steps, left_and_dead, ended, after) = out;
    assert_eq!(steps[0], (0, 0, None, None), "nothing before 2 s");
    assert_eq!(
        steps[1],
        (10, 10, Some(2_000), Some(2_000)),
        "a pulse each at 2 s"
    );
    assert_eq!(
        steps[2],
        (10, 10, Some(2_000), Some(2_000)),
        "laid again, nothing before 4 s"
    );
    assert_eq!(
        steps[3],
        (20, 20, Some(4_000), Some(4_000)),
        "a pulse each at 4 s"
    );
    assert_eq!(
        steps[4],
        (20, 20, Some(4_000), Some(4_000)),
        "none at the second area's own 4.25 s"
    );
    assert_eq!(
        left_and_dead,
        (20, Some(4_000), Some(4_000)),
        "out of it, and dead: passed by"
    );
    assert!(ended.0, "both areas ended");
    assert!(ended.1 > 20, "back in, pulsed again: {}", ended.1);
    assert_eq!(after, (ended.1, ended.2), "nothing after the end");
}

/// Two effects of wild objects give two objects, nobody's, of no end,
/// each of its entry, where the caster stands; one at the caster's front
/// (47) of a 3-yard radius stands 3 yards before them; and a caster
/// lying dead still raises its own (Thistle Tea's "retains separate
/// objects from separate effects", "uses the effect radius for forward
/// placement" and "a corpse still raises its own object but not
/// another's"; mangos-classic's `EffectSummonObjectWild`).
#[tokio::test]
async fn wild_objects_one_per_effect_from_a_corpse_and_ahead_by_radius() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W5", "Wilder", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let here = m.players[&g].position();
            let o = m.players[&g].movement.orientation;
            spells::cast_triggered(m, g, TWO_WILD, Some(g));
            let two = raised(m, TWO_WILD);
            spells::cast_triggered(m, g, AHEAD, Some(g));
            let ahead = raised(m, AHEAD);
            death::kill(m, g);
            let dead = !m.unit_view(g).is_some_and(|u| u.alive);
            spells::cast_triggered_with(
                m,
                g,
                ONE_WILD,
                Targets {
                    unit: Some(g),
                    ..Default::default()
                },
            );
            (here, o, two, ahead, dead, raised(m, ONE_WILD))
        })
        .await
        .unwrap();
    let (here, o, two, ahead, dead, one) = out;
    assert_eq!(
        two.iter().map(|x| (x.0, x.1, x.2)).collect::<Vec<_>>(),
        vec![(WILD_1, None, false), (WILD_2, None, false)],
        "two, nobody's, of no end"
    );
    assert!(two.iter().all(|x| near(x.3, here)), "where the caster is");
    let before = Vector3d {
        x: here.x + 3.0 * o.cos(),
        y: here.y + 3.0 * o.sin(),
        z: here.z,
    };
    assert_eq!(ahead.len(), 1, "one ahead");
    assert!(near(ahead[0].3, before), "3 yards before: {:?}", ahead[0].3);
    assert!(dead, "the caster lies dead");
    assert_eq!(one.len(), 1, "one from the corpse");
    assert!(near(one[0].3, here), "where it lies: {:?}", one[0].3);
}

/// A trap nobody set springs on a player three yards from it: its spell
/// lands as the player's own, its heal on them, and its wild object
/// stands at the trap's place, the cast's destination (Thistle Tea's
/// "object origins supply destinations without a unit component", which
/// has the object cast; mangos-classic's trap `GameObject::Use`).
#[tokio::test]
async fn an_ownerless_trap_casts_at_its_victim_with_its_place_as_destination() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W6", "Sprung", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            let me = m.players[&g].position();
            let at = Vector3d {
                x: me.x + 3.0,
                ..me
            };
            hurt(m, g, 1_000, 10_000);
            let trap = objects::summon(m, None, TRAP_OBJECT, at, 0.0, 0, 0, 0, None);
            m.objects.next_trap_look = None;
            objects::update(m);
            (trap.is_some(), at, kit::health(m, g), raised(m, TRAP_SPELL))
        })
        .await
        .unwrap();
    let (trap, at, health, wild) = out;
    assert!(trap, "the trap");
    assert_eq!(health, 1_100, "healed by its spell");
    assert_eq!(wild.len(), 1, "its object: {wild:?}");
    assert_eq!((wild[0].0, wild[0].1), (WILD_1, None));
    assert!(near(wild[0].3, at), "at the trap: {:?}", wild[0].3);
}

/// A triggered spell that works the object its script targets name, a
/// yard away, works it: the object plays its custom animation 0 (Thistle
/// Tea's "triggered object spells retain object hits and concrete
/// delivery"; mangos-classic's `EffectActivateObject` at
/// `TARGET_GAMEOBJECT_SCRIPT_NEAR_CASTER`).
#[tokio::test]
async fn a_triggered_object_spell_works_the_named_object() {
    let realm = fast_realm(data);
    let (mut c, g, map) = human(&realm, "P9W7", "Worker", Class::Warrior).await;
    let obj = worked_object(&mut c, &map, g).await;
    map.query(move |m| spells::cast_triggered(m, g, WORK, Some(g)))
        .await
        .unwrap();
    assert!(kit::saw_go(&mut c, WORK).await, "it went off");
    assert_eq!(animation(&mut c, obj).await, Some(0), "worked");
}

/// A cast that works a named object works it; the object gone, the cast
/// finds it no more and works nothing (Thistle Tea's "focus selectors and
/// long ranges only see live objects in this world").
#[tokio::test]
async fn a_despawned_object_is_worked_no_more() {
    let realm = fast_realm(data);
    let (mut c, g, map) = human(&realm, "P9W8", "Idler", Class::Warrior).await;
    let obj = worked_object(&mut c, &map, g).await;
    let s = realm.data.spells.get(WORK).cloned().unwrap();
    let first = map
        .query({
            let s = s.clone();
            move |m| {
                kit::ready(m, g, 60, &[WORK]);
                let aimed = objects::cast_at(m, g, &s, &Targets::default());
                (aimed, cast_fresh(m, g, WORK, Targets::default()))
            }
        })
        .await
        .unwrap();
    assert_eq!(first, (vec![obj], true), "aimed at it, cast");
    assert_eq!(animation(&mut c, obj).await, Some(0), "worked");
    let again = map
        .query(move |m| {
            objects::despawn(m, obj);
            let aimed = objects::cast_at(m, g, &s, &Targets::default());
            (aimed, cast_fresh(m, g, WORK, Targets::default()))
        })
        .await
        .unwrap();
    assert_eq!(again, (Vec::new(), true), "aimed at nothing, cast");
    assert_eq!(animation(&mut c, obj).await, None, "nothing worked");
}

/// A periodic trigger whose aura spell has no reagents pays the triggered
/// spell's, a mantle, a cowl and a robe its bearer wears, and the spell
/// goes on; with no robe on, it doesn't go and takes nothing; under an
/// aura spell with a reagent of its own it goes free (Thistle Tea's "a
/// trigger from an aura without reagents spends the worn reagents of the
/// triggered spell"; mangos-classic's `IgnoreItemRequirements`).
#[tokio::test]
async fn a_triggered_spell_spends_reagents_worn_by_its_bearer() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W9", "Wearer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            // A GM pays no reagents.
            m.players.get_mut(&g).unwrap().gm = false;
            let worn = |m: &Map| [MANTLE, COWL, ROBE].map(|e| m.players[&g].carried_count_of(e));
            p3::equip(m, g, MANTLE, SHOULDERS);
            p3::equip(m, g, COWL, HEAD);
            p3::equip(m, g, ROBE, CHEST);
            let dressed = worn(m);
            kit::apply(m, g, g, DRAIN);
            kit::tick_now(m, g, DRAIN);
            let paid = (worn(m), has(m, g, PAID_FOR));
            remove_spell(m, g, PAID_FOR, None);
            p3::equip(m, g, MANTLE, SHOULDERS);
            p3::equip(m, g, COWL, HEAD);
            kit::tick_now(m, g, DRAIN);
            let unrobed = (worn(m), has(m, g, PAID_FOR));
            remove_spell(m, g, DRAIN, None);
            kit::apply(m, g, g, DRAIN_PAID);
            kit::tick_now(m, g, DRAIN_PAID);
            let free = (worn(m), has(m, g, PAID_FOR));
            (dressed, paid, unrobed, free)
        })
        .await
        .unwrap();
    assert_eq!(out.0, [1, 1, 1], "dressed");
    assert_eq!(out.1, ([0, 0, 0], true), "paid with what is worn");
    assert_eq!(out.2, ([1, 1, 0], false), "no robe, no spell");
    assert_eq!(out.3, ([1, 1, 0], true), "free under a paid aura");
}

/// A missile of 100 makes its threat as it lands, by its caster's threat
/// modifiers then: 30 percent less, put on while it flies, makes it 70;
/// taken off, the next one adds its 100 (Thistle Tea's "refreshes threat
/// at impact without replacing the damage snapshot"; mangos-classic's
/// `DealDamage` threat at the hit).
#[tokio::test]
async fn a_missiles_threat_takes_its_casters_modifiers_as_it_lands() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W10", "Thrower", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            m.spells.travel = Some(true);
            kit::ready(m, g, 60, &[]);
            let k = kit::dummy(m, g, 60, 10.0);
            let threat = |m: &Map| {
                m.creature(k)
                    .and_then(|c| c.threat.iter().find(|(u, _)| *u == g).map(|(_, t)| *t))
                    .unwrap_or(0.0)
            };
            spells::cast_triggered(m, g, MISSILE, Some(k));
            let flying = (m.spells.in_flight.len(), threat(m));
            kit::apply(m, g, g, LESS_THREAT);
            pass(m, 600);
            let landed = (threat(m), 1_000_000 - kit::health(m, k));
            remove_spell(m, g, LESS_THREAT, None);
            spells::cast_triggered(m, g, MISSILE, Some(k));
            pass(m, 600);
            (flying, landed, threat(m))
        })
        .await
        .unwrap();
    let (flying, landed, last) = out;
    assert_eq!(flying.0, 1, "in flight");
    assert!(flying.1.abs() < 1e-3, "no threat yet: {}", flying.1);
    assert_eq!(landed.1, 100, "100 dealt");
    assert!(
        (landed.0 - 70.0).abs() < 1e-3,
        "70 as it lands: {}",
        landed.0
    );
    assert!((last - 170.0).abs() < 1e-3, "100 more: {last}");
}

/// One caster's damage over time from two items is two holders, each
/// with its own amount (50 and 75); from the first item again, its own
/// holder is refreshed in place (Thistle Tea's "keeps separate combat
/// snapshots for holders from different items"; mangos-classic lets only
/// a weapon's self buffs stand together so, `IsWeaponBuffCoexistableWith`,
/// and would replace this one).
#[tokio::test]
async fn one_spell_from_two_items_keeps_a_holder_for_each() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W11", "Itemised", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            let k = kit::dummy(m, g, 60, 5.0);
            let s = m.realm.data.spells.get(FROM_ITEMS).cloned().unwrap();
            let from = |m: &mut Map, item: u32, amount: i32| {
                let ctx = Context {
                    item: Some(guid::item(item)),
                    ..Default::default()
                };
                let _ = apply_spell(m, g, k, &s, &[(0, amount)], ctx);
            };
            let held = |m: &Map| -> Vec<(Option<Guid>, i32)> {
                auras(m, k)
                    .map(|a| {
                        a.holders
                            .iter()
                            .filter(|h| h.spell == FROM_ITEMS)
                            .map(|h| (h.item, h.effects.first().map_or(0, |x| x.amount)))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            from(m, 101, 50);
            from(m, 102, 75);
            let two = held(m);
            from(m, 101, 60);
            (two, held(m))
        })
        .await
        .unwrap();
    let (first, second) = (Some(guid::item(101)), Some(guid::item(102)));
    assert_eq!(out.0, vec![(first, 50), (second, 75)], "a holder each");
    assert_eq!(
        out.1,
        vec![(first, 60), (second, 75)],
        "the first refreshed"
    );
}

/// A heal at a script's kobold near the caster that must be below a tenth
/// of its health, aimed at one at full health a yard off, goes to the one
/// three yards off at 4 percent; aimed at one that is below too, it goes
/// to the one aimed at (Thistle Tea's "applies health thresholds before
/// preferring an explicit target"; mangos-classic's
/// `CheckScriptTargeting` with `spell_script_target`'s condition).
#[tokio::test]
async fn a_script_target_failing_its_condition_passes_to_one_meeting_it() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W12", "Mender", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let whole = kit::dummy(m, g, 60, 1.0);
            let wounded = kit::dummy(m, g, 60, 3.0);
            hurt(m, whole, 1_000, 1_000);
            hurt(m, wounded, 40, 1_000);
            let s = m.realm.data.spells.get(MEND).cloned().unwrap();
            let mods = mods::for_spell(m, g, &s);
            let past = targets::effect_targets(m, g, &s, 0, &at(whole), &mods);
            hurt(m, whole, 50, 1_000);
            let kept = targets::effect_targets(m, g, &s, 0, &at(whole), &mods);
            (whole, wounded, past, kept)
        })
        .await
        .unwrap();
    let (whole, wounded, past, kept) = out;
    assert_eq!(past, vec![wounded], "to the one below a tenth");
    assert_eq!(kept, vec![whole], "the one aimed at, below too");
}

/// A dummy at a script's kobold corpse near the caster finds the corpse
/// and runs its spell script, whose cast goes off (its marker on the
/// caster), the corpse left dead; one whose row names another entry's
/// corpse is refused for bad targets and runs nothing (Thistle Tea's
/// "entry-bound corpse effects execute without a global dead-target
/// flag"; vmangos's `SPELL_TARGET_TYPE_DEAD` and `EffectDummy` starting
/// `sSpellScripts`).
#[tokio::test]
async fn a_dummy_at_a_named_corpse_runs_its_script() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W13", "Mourner", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[AT_CORPSE, AT_OTHER]);
            let corpse = kit::dummy(m, g, 60, 1.0);
            kill(m, corpse);
            let cast = cast_fresh(m, g, AT_CORPSE, Targets::default());
            let ran = has(m, g, SECOND);
            remove_spell(m, g, SECOND, None);
            let s = m.realm.data.spells.get(AT_OTHER).cloned().unwrap();
            let mods = mods::for_spell(m, g, &s);
            kit::fresh(m, g);
            let refused = checks::check(m, g, &s, &Targets::default(), None, false, &mods);
            let other = cast_fresh(m, g, AT_OTHER, Targets::default());
            let still_dead = !m.unit_view(corpse).is_some_and(|u| u.alive);
            (cast, ran, refused, other, has(m, g, SECOND), still_dead)
        })
        .await
        .unwrap();
    assert_eq!((out.0, out.1), (true, true), "cast, its script ran");
    assert_eq!(out.2, Some(checks::fail::BAD_TARGETS), "another's refused");
    assert_eq!((out.3, out.4), (false, false), "not cast, nothing ran");
    assert!(out.5, "the corpse left dead");
}

/// A script effect's spell script runs from its caster at the unit it
/// lands on: cast at a kobold, the script's cast puts its mark on the
/// kobold, the caster's; cast at oneself, on oneself (Thistle Tea's
/// "database script effects use the caster's existing script runner";
/// vmangos's `EffectScriptEffect` starting `sSpellScripts` with the
/// caster as source and the unit as target).
#[tokio::test]
async fn a_script_effect_runs_its_spell_script_from_the_caster() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W14", "Scripter", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[]);
            let k = kit::dummy(m, g, 60, 5.0);
            spells::cast_triggered(m, g, SCRIPTED, Some(k));
            let at_kobold = (kit::holder(m, k, MARK).map(|h| h.caster), has(m, g, MARK));
            spells::cast_triggered(m, g, SCRIPTED, Some(g));
            let own = kit::holder(m, g, MARK).map(|h| h.caster);
            (at_kobold, own)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (Some(g), false), "on the kobold, from the caster");
    assert_eq!(out.1, Some(g), "on the caster, cast at itself");
}

/// A heal only at a script's kobold near the caster, with one there, is
/// cast whatever unit the client names, one not even on the map; with a
/// heal at an ally beside it, that unit must be there, and the cast is
/// refused for bad targets (Thistle Tea's "does not validate unrelated
/// selection for scripted-only effects"; mangos-classic's `CheckCast`
/// checking the unit only for a target that wants one).
#[tokio::test]
async fn a_script_only_spell_ignores_an_unrelated_selection() {
    let realm = fast_realm(data);
    let (_c, g, map) = human(&realm, "P9W15", "Unrelated", Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[SCRIPT_HEAL, SCRIPT_HEAL_ALLY]);
            kit::dummy(m, g, 60, 2.0);
            let nobody = guid::unit(KOBOLD, 0x7F_FFFF);
            let check = |m: &mut Map, id: u32| {
                let s = m.realm.data.spells.get(id).cloned().unwrap();
                let mods = mods::for_spell(m, g, &s);
                kit::fresh(m, g);
                checks::check(m, g, &s, &at(nobody), None, false, &mods)
            };
            let alone = check(m, SCRIPT_HEAL);
            let cast = cast_fresh(m, g, SCRIPT_HEAL, at(nobody));
            let with_ally = check(m, SCRIPT_HEAL_ALLY);
            (alone, cast, with_ally)
        })
        .await
        .unwrap();
    assert_eq!((out.0, out.1), (None, true), "the selection ignored");
    assert_eq!(out.2, Some(checks::fail::BAD_TARGETS), "an ally wanted");
}
