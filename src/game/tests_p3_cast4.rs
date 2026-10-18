//! Phase 3's cast checks, as Thistle Tea's `cast_validation_test` and its
//! kin have them: silence and pacify, power and health, gear, the dead,
//! the global cooldown, posture, stealth and mounts, range and facing. On
//! made-up spells and the fast suite, through `checks::check`.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, at, dummy, fresh, ready, set_health, set_power};
use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::checks::{self, fail};
use crate::game::map::spells::{self, mods};

/// Why `caster` can't cast `s` at `t`, untriggered, if it can't.
fn check(m: &mut Map, caster: Guid, s: &SpellTemplate, t: spells::Targets) -> Option<u8> {
    check_as(m, caster, s, t, false)
}

fn check_as(
    m: &mut Map,
    caster: Guid,
    s: &SpellTemplate,
    t: spells::Targets,
    triggered: bool,
) -> Option<u8> {
    let mods = mods::for_spell(m, caster, s);
    checks::check(m, caster, s, &t, None, triggered, &mods)
}

/// A harmful bolt of `prevention` at a unit.
fn bolt(id: u32, prevention: u32) -> SpellTemplate {
    let mut s = bare(id, 4);
    s.prevention_type = prevention;
    s.range_index = RANGE_30;
    effect(&mut s, 0, damage(10));
    s
}

/// A helpful spell on oneself.
fn helpful(id: u32) -> SpellTemplate {
    let mut s = bare(id, 2);
    s.prevention_type = 1;
    effect(
        &mut s,
        0,
        crate::game::data::spells::SpellEffect {
            effect: 10,
            target_a: 1,
            ..damage(10)
        },
    );
    s
}

const RANGE_30: u32 = 900;
const RANGE_MELEE: u32 = 2;
const RANGE_8_TO_30: u32 = 901;

fn ranges(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_30, (0.0, 30.0));
    d.spells.ranges.insert(RANGE_MELEE, (0.0, 5.0));
    d.spells.ranges.insert(RANGE_8_TO_30, (8.0, 30.0));
}

/// A control aura of `kind` on `g`, lasting.
fn control(m: &mut Map, g: Guid, id: u32, kind: u32) {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    effect(&mut s, 0, aura(kind, 0, 0, 1));
    apply_template(m, g, g, &s);
}

/// Silence keeps off what it prevents (a spell of prevention type 1), not
/// an ability or a wand's Shoot (2) nor a Hearthstone (0); pacify the
/// other way about (Thistle Tea's prevention types, mangos's
/// `CheckCasterAuras`).
#[tokio::test]
async fn silence_and_pacify_keep_off_what_their_prevention_type_says() {
    let realm = fast_realm(ranges);
    let (_c, mage, map) = human(&realm, "P3VS", "Silenced", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let mut out = Vec::new();
            for (id, kind) in [(990_400, AURA_MOD_SILENCE), (990_401, AURA_MOD_PACIFY)] {
                control(m, mage, id, kind);
                out.push(
                    [0, 1, 2]
                        .map(|p| check(m, mage, &bolt(990_410 + p, p), at(k)))
                        .to_vec(),
                );
                remove_spell(m, mage, id, None);
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(out[0], [None, Some(fail::SILENCED), None], "silenced");
    assert_eq!(out[1], [None, None, Some(fail::PACIFIED)], "pacified");
}

/// Power: a player wants every power a spell costs, and health above a
/// health cost; an ordinary creature with mana wants it, one without
/// casts mana spells, and none wants rage or energy (Thistle Tea's
/// creature resources, mangos's `CheckPower`).
#[tokio::test]
async fn a_cast_wants_its_power_but_an_ordinary_creature_only_the_mana_it_has() {
    let realm = fast_realm(ranges);
    let (_c, mage, map) = human(&realm, "P3VP", "Powerless", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let costing = |power: u32| {
                let mut s = helpful(990_420 + power.min(9));
                s.power_type = power;
                s.mana_cost = 60;
                s
            };
            for i in 0..5 {
                set_power(m, mage, i, 0);
            }
            let player: Vec<Option<u8>> = (0..5)
                .map(|p| check(m, mage, &costing(p), Default::default()))
                .collect();
            set_health(m, mage, 60);
            let health = spells::POWER_HEALTH;
            let player_health = check(m, mage, &costing(health), Default::default());
            set_health(m, mage, 61);
            let player_health_above = check(m, mage, &costing(health), Default::default());
            // The kobold: without mana, then with a pool and none in it.
            let creature_none: Vec<Option<u8>> = (0..5)
                .map(|p| check(m, k, &costing(p), Default::default()))
                .collect();
            if let Some(c) = m.creature_mut(k) {
                c.base.max_mana = 100;
                c.fields.set_u32(UNIT_FIELD_MAXPOWER1, 100);
                c.fields.set_u32(UNIT_FIELD_POWER1, 0);
            }
            let creature_mana = check(m, k, &costing(0), Default::default());
            if let Some(c) = m.creature_mut(k) {
                c.fields.set_u32(UNIT_FIELD_POWER1, 60);
            }
            let creature_enough = check(m, k, &costing(0), Default::default());
            set_health(m, k, 60);
            let creature_health = check(m, k, &costing(health), Default::default());
            (
                player,
                player_health,
                player_health_above,
                creature_none,
                creature_mana,
                creature_enough,
                creature_health,
            )
        })
        .await
        .unwrap();
    let (player, health, above, creature_none, creature_mana, enough, creature_health) = out;
    assert_eq!(
        player,
        [Some(fail::NO_POWER); 5],
        "a player wants every power"
    );
    assert_eq!(
        health,
        Some(fail::CASTER_AURASTATE),
        "health above the cost"
    );
    assert_eq!(above, None);
    assert_eq!(creature_none, [None; 5], "a creature without the pools");
    assert_eq!(
        creature_mana,
        Some(fail::NO_POWER),
        "one with mana wants it"
    );
    assert_eq!(enough, None);
    assert_eq!(
        creature_health,
        Some(fail::CASTER_AURASTATE),
        "and a creature health above a health cost"
    );
}

/// A spell that wants a weapon: a creature casts it without one, a player
/// can't (Thistle Tea's weapon restrictions, mangos's `CheckItems`).
#[tokio::test]
async fn a_creature_shoots_without_the_weapon_a_player_wants() {
    let realm = fast_realm(ranges);
    let (_c, mage, map) = human(&realm, "P3VW", "Unarmed", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let mut shoot = bolt(990_430, 2);
            shoot.school = 0;
            shoot.dmg_class = 3;
            shoot.equipped_item_class = 2;
            shoot.equipped_item_subclass_mask = 0x4000;
            let mut sword = helpful(990_431);
            sword.equipped_item_class = 2;
            sword.equipped_item_subclass_mask = 0x8000;
            (
                check(m, k, &shoot, at(mage)),
                check(m, k, &sword, Default::default()),
                check(m, mage, &shoot, at(k)),
                check(m, mage, &sword, Default::default()),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, None, "a creature shoots without a bow");
    assert_eq!(out.1, None, "and casts without the sword");
    assert_eq!(out.2, Some(fail::EQUIPPED_ITEM_CLASS), "a player doesn't");
    assert_eq!(out.3, Some(fail::EQUIPPED_ITEM_CLASS));
}

/// The dead cast only what is castable while dead; a spell for corpses
/// goes at a dead enemy, and still not at a friend's (Thistle Tea's
/// `castable_while_dead` and `allow_dead_target`).
#[tokio::test]
async fn the_dead_cast_what_the_dead_may_and_corpse_spells_take_a_dead_enemy() {
    let realm = fast_realm(ranges);
    let (_c, mage, map) = human(&realm, "P3VD", "Deadcast", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let plain = bolt(990_440, 1);
            let mut corpse = bolt(990_441, 1);
            corpse.attributes[2] |= sd::ATTR_EX2_ALLOW_DEAD_TARGET;
            crate::game::map::ai::die(m, k, mage);
            let at_dead = (
                check(m, mage, &plain, at(k)),
                check(m, mage, &corpse, at(k)),
            );
            crate::game::map::death::kill(m, mage);
            let mut ghostly = helpful(990_444);
            ghostly.attributes[0] |= sd::ATTR_CASTABLE_WHILE_DEAD;
            let dead_caster = (
                check(m, mage, &helpful(990_445), Default::default()),
                check(m, mage, &ghostly, Default::default()),
            );
            (at_dead, dead_caster)
        })
        .await
        .unwrap();
    let (at_dead, dead_caster) = out;
    assert_eq!(at_dead.0, Some(fail::TARGETS_DEAD), "not at the dead");
    assert_eq!(at_dead.1, None, "a corpse spell is");
    assert_eq!(dead_caster.0, Some(fail::CASTER_DEAD));
    assert_eq!(dead_caster.1, None, "castable while dead");
}

/// The global cooldown keeps off a spell of its category until it lapses,
/// and never a spell without one (Thistle Tea's `trigger_gcd`).
#[tokio::test]
async fn the_global_cooldown_holds_its_category_until_it_lapses() {
    let mut gcd = helpful(990_450);
    gcd.start_recovery_category = 133;
    gcd.start_recovery_time = 1_500;
    let mut free = helpful(990_451);
    free.start_recovery_category = 0;
    let both = [gcd.clone(), free.clone()];
    let realm = fast_realm(move |d| {
        ranges(d);
        insert(d, both);
    });
    let (_c, mage, map) = human(&realm, "P3VG", "Cooling", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[990_450, 990_451]);
            fresh(m, mage);
            let cast = spells::cast(m, mage, 990_450, Default::default(), None);
            m.now += Duration::from_millis(100);
            let held = (
                check(m, mage, &gcd, Default::default()),
                check(m, mage, &free, Default::default()),
            );
            m.now += Duration::from_millis(1_400);
            let lapsed = check(m, mage, &gcd, Default::default());
            (cast, held, lapsed)
        })
        .await
        .unwrap();
    assert!(out.0, "the cast went");
    assert_eq!(
        out.1,
        (Some(fail::NOT_READY), None),
        "held, and not the free spell"
    );
    assert_eq!(out.2, None, "lapsed at a second and a half");
}

/// Posture, stealth and mounts: a seated player casts only what may be
/// cast sitting, or a triggered spell; an opener wants stealth; a mounted
/// player casts only what may be cast mounted, and keeps the mount for it
/// (Thistle Tea's `Posture`, `Stealth.validate` and mounts).
#[tokio::test]
async fn sitting_stealth_and_a_mount_gate_what_may_be_cast() {
    let realm = fast_realm(ranges);
    let (_c, rogue, map) = human(&realm, "P3VM", "Gated", Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, rogue, 60, &[]);
            let plain = helpful(990_460);
            let mut sitting = helpful(990_461);
            sitting.attributes[0] |= sd::ATTR_ALLOW_WHILE_SITTING;
            if let Some(p) = m.players.get_mut(&rogue) {
                p.set_stand_state(1);
            }
            let seated = (
                check(m, rogue, &plain, Default::default()),
                check(m, rogue, &sitting, Default::default()),
                check_as(m, rogue, &plain, Default::default(), true),
            );
            if let Some(p) = m.players.get_mut(&rogue) {
                p.set_stand_state(0);
            }
            let mut opener = helpful(990_462);
            opener.attributes[0] |= sd::ATTR_ONLY_STEALTHED;
            let unstealthed = check(m, rogue, &opener, Default::default());
            control(m, rogue, 990_463, AURA_MOD_STEALTH);
            let stealthed = check(m, rogue, &opener, Default::default());
            remove_spell(m, rogue, 990_463, None);
            if let Some(p) = m.players.get_mut(&rogue) {
                p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2_404);
            }
            let mut mounted_ok = helpful(990_464);
            mounted_ok.attributes[0] |= sd::ATTR_ALLOW_WHILE_MOUNTED;
            let mounted = (
                check(m, rogue, &plain, Default::default()),
                check(m, rogue, &mounted_ok, Default::default()),
            );
            (seated, unstealthed, stealthed, mounted)
        })
        .await
        .unwrap();
    let (seated, unstealthed, stealthed, mounted) = out;
    assert_eq!(seated, (Some(fail::NOT_STANDING), None, None), "seated");
    assert_eq!(unstealthed, Some(fail::ONLY_STEALTHED));
    assert_eq!(stealthed, None);
    assert_eq!(mounted, (Some(fail::NOT_MOUNTED), None), "mounted");
}

/// Range: the spell's, with its family's range modifiers flat then
/// percent, and the two units' reaches and a yard; not another family's
/// or mask's modifier; a melee spell's by melee reach; the minimum range;
/// and a triggered spell goes at any distance (Thistle Tea's `Range`).
#[tokio::test]
async fn range_takes_its_modifiers_and_the_reaches_and_a_triggered_spell_none() {
    let mut longer = bare(990_470, 0);
    longer.family_name = 3;
    longer.duration_index = LASTS_LONG;
    effect(
        &mut longer,
        0,
        crate::game::data::spells::SpellEffect {
            item_type: 1,
            ..aura(AURA_ADD_FLAT_MODIFIER, mods::OP_RANGE as i32, 5, 1)
        },
    );
    let mut wider = longer.clone();
    wider.id = 990_471;
    wider.effects[0].aura = AURA_ADD_PCT_MODIFIER;
    wider.effects[0].base_points = 49;
    let talents = [longer, wider];
    let realm = fast_realm(move |d| {
        ranges(d);
        insert(d, talents);
    });
    let (_c, mage, map) = human(&realm, "P3VR", "Ranger", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let reach = {
                let a = m.unit_view(mage).unwrap().combat_reach;
                let b = m.unit_view(k).unwrap().combat_reach;
                a + b + 1.0
            };
            let mut s = bolt(990_472, 1);
            s.family_name = 3;
            s.family_flags = 1;
            let mut other = s.clone();
            other.family_flags = 2;
            let place = |m: &mut Map, d: f32| {
                let me = m.unit_view(mage).unwrap();
                let (x, y) = (
                    me.position.x + d * me.orientation.cos(),
                    me.position.y + d * me.orientation.sin(),
                );
                if let Some(c) = m.creature_mut(k) {
                    c.position.x = x;
                    c.position.y = y;
                }
            };
            let mut seen = Vec::new();
            for d in [30.0 + reach - 0.05, 30.0 + reach + 0.05] {
                place(m, d);
                seen.push(check(m, mage, &s, at(k)));
            }
            for id in [990_470, 990_471] {
                let t = m.realm.data.spells.get(id).cloned().unwrap();
                apply_template(m, mage, mage, &t);
            }
            for d in [52.5 + reach - 0.05, 52.5 + reach + 0.05] {
                place(m, d);
                seen.push(check(m, mage, &s, at(k)));
            }
            seen.push(check(m, mage, &other, at(k)));
            seen.push(check_as(m, mage, &other, at(k), true));
            // Melee reach.
            let mut strike = bolt(990_473, 2);
            strike.dmg_class = 2;
            strike.school = 0;
            strike.range_index = RANGE_MELEE;
            let melee = crate::game::map::combat::melee_range(
                &m.unit_view(mage).unwrap(),
                &m.unit_view(k).unwrap(),
            );
            for d in [melee + 0.95, melee + 1.05] {
                place(m, d);
                seen.push(check(m, mage, &strike, at(k)));
            }
            // The minimum range.
            let mut far = bolt(990_474, 1);
            far.range_index = RANGE_8_TO_30;
            for d in [7.9, 8.1] {
                place(m, d);
                seen.push(check(m, mage, &far, at(k)));
            }
            seen
        })
        .await
        .unwrap();
    let ok = None;
    let out_of_range = Some(fail::OUT_OF_RANGE);
    assert_eq!(
        seen_at(&out, 0..2),
        [ok, out_of_range],
        "thirty yards and the reaches"
    );
    assert_eq!(
        seen_at(&out, 2..4),
        [ok, out_of_range],
        "fifty-two and a half, modified"
    );
    assert_eq!(out[4], out_of_range, "not another mask's");
    assert_eq!(out[5], ok, "a triggered spell goes at any distance");
    assert_eq!(
        seen_at(&out, 6..8),
        [ok, out_of_range],
        "melee reach and a yard"
    );
    assert_eq!(
        seen_at(&out, 8..10),
        [Some(fail::TOO_CLOSE), ok],
        "the minimum"
    );
}

fn seen_at(v: &[Option<u8>], r: std::ops::Range<usize>) -> Vec<Option<u8>> {
    v[r].to_vec()
}

/// Facing: a melee-range spell wants its target within the half circle in
/// front, wrapped; close enough to overlap (1.4 yards), any way; and not a
/// triggered or next-swing spell (Thistle Tea's `Facing`).
#[tokio::test]
async fn a_melee_spell_faces_its_target_unless_overlapping_or_triggered() {
    let realm = fast_realm(ranges);
    let (_c, warrior, map) = human(&realm, "P3VF", "Facer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, warrior, 60, &[]);
            let k = dummy(m, warrior, 60, 2.0);
            let mut strike = bolt(990_480, 2);
            strike.dmg_class = 2;
            strike.school = 0;
            strike.range_index = RANGE_MELEE;
            let mut swing = strike.clone();
            swing.id = 990_481;
            swing.attributes[0] |= sd::ATTR_ON_NEXT_SWING;
            let turn = |m: &mut Map, o: f32| {
                if let Some(p) = m.players.get_mut(&warrior) {
                    p.movement.orientation = o;
                }
            };
            let o = m.players.get(&warrior).unwrap().movement.orientation;
            let pi = std::f32::consts::PI;
            let mut seen = Vec::new();
            for turned in [0.0, 1.5, 2.0 * pi - 1.5, pi] {
                turn(m, o + turned);
                seen.push(check(m, warrior, &strike, at(k)));
            }
            seen.push(check_as(m, warrior, &strike, at(k), true));
            seen.push(check(m, warrior, &swing, at(k)));
            // Overlapping, backwards.
            let me = m.unit_view(warrior).unwrap().position;
            if let Some(c) = m.creature_mut(k) {
                c.position.x = me.x + 1.0 * o.cos();
                c.position.y = me.y + 1.0 * o.sin();
            }
            seen.push(check(m, warrior, &strike, at(k)));
            seen
        })
        .await
        .unwrap();
    let ok = None;
    let behind = Some(fail::UNIT_NOT_INFRONT);
    assert_eq!(out[..4], [ok, ok, ok, behind], "the front half, wrapped");
    assert_eq!(out[4], ok, "triggered");
    assert_eq!(out[5], ok, "next swing");
    assert_eq!(out[6], ok, "overlapping");
}

/// A silence stops a cast under way that silence prevents, and lets a
/// Hearthstone's kind (prevention type 0) go on (mangos's
/// `HandleAuraModSilence`).
#[tokio::test]
async fn a_silence_stops_only_the_casts_it_prevents() {
    let mut spell = helpful(990_490);
    spell.casting_time_index = CAST_3S;
    let mut stone = spell.clone();
    stone.id = 990_491;
    stone.prevention_type = 0;
    let both = [spell, stone];
    let realm = fast_realm(move |d| {
        ranges(d);
        insert(d, both);
    });
    let (_c, mage, map) = human(&realm, "P3VI", "Hushed", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[990_490, 990_491]);
            let mut kept = Vec::new();
            for id in [990_490, 990_491] {
                fresh(m, mage);
                spells::cast(m, mage, id, Default::default(), None);
                let began = super::tests_p3_spells_kit::casting(m, mage);
                control(m, mage, 990_492, AURA_MOD_SILENCE);
                kept.push((began, super::tests_p3_spells_kit::casting(m, mage)));
                remove_spell(m, mage, 990_492, None);
                spells::interrupt(m, mage);
            }
            kept
        })
        .await
        .unwrap();
    assert_eq!(out[0], (Some(990_490), None), "a spell is stopped");
    assert_eq!(
        out[1],
        (Some(990_491), Some(990_491)),
        "a Hearthstone isn't"
    );
}
