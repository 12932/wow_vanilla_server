//! Phase 3, round 6, against Thistle Tea's `chain_targets_test.exs`: whom
//! a chain jumps to, harm and heals, and one choice of targets shared by a
//! spell's effects.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, health, set_health};
use crate::game::data::spells::{ATTR_EX2_IGNORE_LOS, SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, FactionTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOD_HASTE, AURA_MOD_STEALTH};

/// A beast of the monsters, a dragonkin of theirs, a friend of
/// Stormwind's.
pub const BEAST: u32 = 99_601;
pub const DRAGONKIN: u32 = 99_602;
pub const FRIEND: u32 = 99_603;
/// Thirty yards.
pub const RANGE_30: u32 = 951;

/// A bolt that jumps to three; one that ignores sight; one for beasts.
const BOLT: u32 = 991_000;
const BOLT_SIGHTLESS: u32 = 991_001;
const BOLT_BEASTS: u32 = 991_002;
/// A heal that jumps to three.
const CHAIN_HEAL: u32 = 991_003;
/// Damage of 200 jumping to three, each half the last.
const HALVING: u32 = 991_004;
/// Damage and a slow about the caster, four at most.
const NOVA: u32 = 991_005;
/// A stealth that no one sees through.
const HIDDEN: u32 = 991_006;

pub fn creature(entry: u32, creature_type: u32, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Made-up {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 60,
        level_max: 60,
        faction,
        creature_type,
        ..Default::default()
    }
}

pub fn faction(id: u32, faction: u32, ours: u32, friends: u32, enemies: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction,
        ours,
        friend_mask: friends,
        enemy_mask: enemies,
        ..Default::default()
    }
}

/// A human's faction, Stormwind's and a monster's; the beast, the
/// dragonkin and the friend; thirty yards.
pub fn sides(d: &mut GameData) {
    d.faction_templates.insert(1, faction(1, 1, 0x3, 0x2, 0x8));
    d.faction_templates
        .insert(11, faction(11, 72, 0x2, 0x2, 0xC));
    d.faction_templates.insert(14, faction(14, 14, 0x8, 0, 0x7));
    for t in [
        creature(BEAST, 1, 14),
        creature(DRAGONKIN, 2, 14),
        creature(FRIEND, 7, 11),
    ] {
        d.creature_templates.insert(t.entry, t);
    }
    d.spells.ranges.insert(RANGE_30, (0.0, 30.0));
}

fn data(d: &mut GameData) {
    sides(d);
    let chained = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 3);
        s.range_index = RANGE_30;
        effect(
            &mut s,
            0,
            SpellEffect {
                chain_target: 3,
                ..e
            },
        );
        s
    };
    let mut sightless = chained(BOLT_SIGHTLESS, damage(10));
    sightless.attributes[2] |= ATTR_EX2_IGNORE_LOS;
    let mut beasts = chained(BOLT_BEASTS, damage(10));
    beasts.target_creature_type = 1;
    let heal = SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        target_a: 45,
        ..Default::default()
    };
    let mut halving = chained(HALVING, damage(200));
    halving.dmg_multiplier[0] = 0.5;
    halving.attributes[2] |= ATTR_EX2_IGNORE_LOS;
    let mut nova = bare(NOVA, 0);
    nova.max_affected_targets = 4;
    nova.duration_index = LASTS_LONG;
    d.spells.radii.insert(990, 10.0);
    effect(
        &mut nova,
        0,
        SpellEffect {
            target_a: 22,
            target_b: 15,
            radius_index: 990,
            ..damage(10)
        },
    );
    effect(
        &mut nova,
        1,
        SpellEffect {
            target_a: 22,
            target_b: 15,
            radius_index: 990,
            ..aura(AURA_MOD_HASTE, 0, -10, 22)
        },
    );
    let mut hidden = bare(HIDDEN, 0);
    hidden.duration_index = LASTS_LONG;
    effect(&mut hidden, 0, aura(AURA_MOD_STEALTH, 0, 1000, 1));
    let spells: Vec<SpellTemplate> = vec![
        chained(BOLT, damage(10)),
        sightless,
        beasts,
        chained(CHAIN_HEAL, heal),
        halving,
        nova,
        hidden,
    ];
    insert(d, spells);
}

/// `g` at `at`, its grid cell with it.
pub fn put(m: &mut Map, g: Guid, at: Vector3d) {
    if let Some(c) = m.creature_mut(g) {
        let old = c.cell;
        c.position = at;
        c.cell = crate::game::map::grid::cell_of(at.x, at.y);
        let new = c.cell;
        m.grid.relocate(g, old, new);
    } else if let Some(p) = m.players.get_mut(&g) {
        let old = p.cell;
        p.movement.position = at;
        p.cell = crate::game::map::grid::cell_of(at.x, at.y);
        let new = p.cell;
        m.grid.relocate(g, old, new);
    }
}

/// `dx` yards east and `dy` north of `g`.
pub fn off(m: &Map, g: Guid, dx: f32, dy: f32) -> Vector3d {
    let p = m.position_of(g).unwrap();
    Vector3d {
        x: p.x + dx,
        y: p.y + dy,
        z: p.z,
    }
}

/// `g` facing east.
pub fn face_east(m: &mut Map, g: Guid) {
    if let Some(p) = m.players.get_mut(&g) {
        p.movement.orientation = 0.0;
    }
}

/// A creature of `entry` at `dx`, `dy` from `from`.
pub fn unit_at(m: &mut Map, from: Guid, entry: u32, dx: f32, dy: f32) -> Guid {
    let g = kit::spawn(m, from, entry, 60, 1.0);
    let to = off(m, from, dx, dy);
    put(m, g, to);
    g
}

/// The units a cast of `spell` went off at, as its caster's client heard:
/// its hits, then its misses (mangos's `WriteSpellGoTargets`).
pub async fn go(c: &mut Client, spell: u32) -> Vec<Guid> {
    c.wait_for("the go", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(
            g.hits
                .iter()
                .copied()
                .chain(g.misses.iter().map(|x| x.target))
                .collect(),
        ),
        _ => None,
    })
    .await
}

/// A unit's health short of its maximum by `short`.
fn hurt(m: &mut Map, g: Guid, max: u32, short: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, max);
    }
    set_health(m, g, max - short);
}

/// A harmful chain jumps to the nearest enemy within ten yards of the
/// last, each once, in sight of the last, unless the spell ignores sight;
/// never to the dead, one stealthed beyond the caster's sight, or a type
/// the spell doesn't name (Thistle Tea's `ChainTargets.expand`; mangos's
/// `SCHEME_CLOSEST_CHAIN` with its line of sight from each jump).
#[tokio::test]
async fn a_harmful_chain_jumps_to_the_nearest_seen_in_sight_of_the_last() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P6WA", "Chainer", Class::Mage).await;
    let sight = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BOLT, BOLT_SIGHTLESS, BOLT_BEASTS]);
            face_east(m, a);
            let first = unit_at(m, a, BEAST, 4.0, 0.0);
            let blocked = unit_at(m, a, BEAST, 4.0, 3.0);
            let second = unit_at(m, a, BEAST, 13.0, 0.0);
            let third = unit_at(m, a, BEAST, 22.0, 0.0);
            // A wall between the first and the one beside it, and between
            // the second and that one.
            let (w0, w1) = (off(m, a, 3.0, 1.5), off(m, a, 14.0, 1.5));
            m.spells.walls.push((w0, w1));
            assert!(cast_fresh(m, a, BOLT, at(first)));
            assert!(cast_fresh(m, a, BOLT_SIGHTLESS, at(first)));
            m.spells.walls.clear();
            for g in [first, blocked, second, third] {
                m.remove_creature(g);
            }
            (first, blocked, second, third)
        })
        .await
        .unwrap();
    let (first, blocked, second, third) = sight;
    assert_eq!(go(&mut c, BOLT).await, [first, second, third], "in sight");
    assert_eq!(
        go(&mut c, BOLT_SIGHTLESS).await,
        [first, blocked, second],
        "ignoring sight"
    );
    let kinds = map
        .query(move |m| {
            let first = unit_at(m, a, BEAST, 4.0, 0.0);
            let dead = unit_at(m, a, BEAST, 5.0, 1.0);
            crate::game::map::combat::kill(m, a, dead);
            let hidden = unit_at(m, a, BEAST, 5.0, -1.0);
            kit::apply(m, hidden, hidden, HIDDEN);
            let dragon = unit_at(m, a, DRAGONKIN, 5.0, 0.5);
            let valid = unit_at(m, a, BEAST, 8.0, 0.0);
            assert!(cast_fresh(m, a, BOLT_BEASTS, at(first)));
            (first, valid, [dead, hidden, dragon])
        })
        .await
        .unwrap();
    let (first, valid, _) = kinds;
    assert_eq!(
        go(&mut c, BOLT_BEASTS).await,
        [first, valid],
        "not the dead, the unseen, nor a dragonkin"
    );
}

/// A healing chain jumps only to a friend who is hurt: the last one's
/// group first, then the most hurt by health (not by its share), then the
/// nearest; it may come back to its hurt caster, but no further than a
/// heal's jump, 12.5 yards (Thistle Tea's `ChainTargets` healing priority;
/// mangos's `ChainHealingOrder`; the 12.5 your call, mangos's is 10).
#[tokio::test]
async fn a_healing_chain_jumps_to_the_most_hurt_of_the_last_ones_group_first() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P6WB", "Healer", Class::Priest).await;
    let (_d, b, _) = human(&realm, "P6WC", "Firsty", Class::Warrior).await;
    let (_e, member, _) = human(&realm, "P6WD", "Member", Class::Warrior).await;
    let lo = realm.registry.online(b).unwrap();
    let mo = realm.registry.online(member).unwrap();
    assert!(crate::game::social::groups::join_directly(&realm, &lo, &mo));
    let most_hurt = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CHAIN_HEAL]);
            face_east(m, a);
            let first = unit_at(m, a, FRIEND, 4.0, 0.0);
            hurt(m, first, 1000, 10);
            let full = unit_at(m, a, FRIEND, 5.0, 0.0);
            hurt(m, full, 1000, 0);
            let small = unit_at(m, a, FRIEND, 6.0, 0.0);
            hurt(m, small, 111, 100);
            let large = unit_at(m, a, FRIEND, 7.0, 0.0);
            hurt(m, large, 3333, 1000);
            assert!(cast_fresh(m, a, CHAIN_HEAL, at(first)));
            for g in [first, full, small, large] {
                m.remove_creature(g);
            }
            (first, large, small)
        })
        .await
        .unwrap();
    let (first, large, small) = most_hurt;
    assert_eq!(
        go(&mut c, CHAIN_HEAL).await,
        [first, large, small],
        "the most health short, not the full"
    );
    let group = map
        .query(move |m| {
            for (g, dx) in [(b, 4.0), (member, 6.0)] {
                let to = off(m, a, dx, 0.0);
                put(m, g, to);
            }
            hurt(m, b, 1000, 10);
            hurt(m, member, 1000, 50);
            let outsider = unit_at(m, a, FRIEND, 7.0, 0.0);
            hurt(m, outsider, 3000, 1000);
            assert!(cast_fresh(m, a, CHAIN_HEAL, at(b)));
            m.remove_creature(outsider);
            for g in [b, member] {
                let to = off(m, a, 0.0, 40.0);
                put(m, g, to);
            }
            outsider
        })
        .await
        .unwrap();
    assert_eq!(
        go(&mut c, CHAIN_HEAL).await,
        [b, member, group],
        "the first one's group before a stranger more hurt"
    );
    let back = map
        .query(move |m| {
            let first = unit_at(m, a, FRIEND, 9.0, 0.0);
            hurt(m, first, 1000, 10);
            let distant = unit_at(m, a, FRIEND, 22.0, 0.0);
            hurt(m, distant, 3000, 1000);
            hurt(m, a, 2000, 500);
            assert!(cast_fresh(m, a, CHAIN_HEAL, at(first)));
            first
        })
        .await
        .unwrap();
    assert_eq!(
        go(&mut c, CHAIN_HEAL).await,
        [back, a],
        "back to its caster, and no further than a jump"
    );
}

/// A chain lands on its targets in order, each jump's damage the last
/// one's times the effect's multiplier (Thistle Tea's chain amounts;
/// mangos's `DamageMultiplier`).
#[tokio::test]
async fn each_jump_of_a_chain_does_its_multiplier_of_the_last() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P6WE", "Halver", Class::Mage).await;
    let (order, taken) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[HALVING]);
            face_east(m, a);
            let units = [4.0, 13.0, 22.0].map(|dx| unit_at(m, a, BEAST, dx, 0.0));
            let before = units.map(|g| health(m, g));
            assert!(cast_fresh(m, a, HALVING, at(units[0])));
            let after = units.map(|g| health(m, g));
            (units, [0, 1, 2].map(|i| before[i] - after[i]))
        })
        .await
        .unwrap();
    assert_eq!(go(&mut c, HALVING).await, order, "in order");
    assert_eq!(taken, [200, 100, 50], "each half the last");
}

/// A spell's effects aimed alike choose their targets once: an area's
/// damage and its slow, four at most, land on the same four, the unit
/// aimed at among them (Thistle Tea's one capped selection; mangos's
/// `FillTargetMap` sharing an earlier effect's targets).
#[tokio::test]
async fn effects_aimed_alike_share_one_choice_of_targets() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P6WF", "Novas", Class::Mage).await;
    let (selected, rounds) = map
        .query(move |m| {
            m.spells.rolls = Some(true);
            kit::ready(m, a, 60, &[NOVA]);
            face_east(m, a);
            let units: Vec<Guid> = (1..=6)
                .map(|x| unit_at(m, a, BEAST, x as f32, 0.0))
                .collect();
            let selected = units[5];
            for _ in 0..12 {
                assert!(cast_fresh(m, a, NOVA, at(selected)));
            }
            (selected, 12)
        })
        .await
        .unwrap();
    for _ in 0..rounds {
        let hits = go(&mut c, NOVA).await;
        assert_eq!(hits.len(), 4, "four, both effects alike: {hits:?}");
        assert!(hits.contains(&selected), "the one aimed at");
    }
}
