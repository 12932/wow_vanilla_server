//! Phase 3's spell effect tests, ported from Thistle Tea's
//! `test/game/world/entity/effect_resolver` (docs/parity/matrix.csv): a
//! meteor's damage shared among those it falls on.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_SCHOOL_IMMUNITY};
use crate::game::map::spells;
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{aura_effect, on, spell};

/// 302 fire to the enemies within ten yards of where it falls, shared;
/// the same with a second effect of 90 at its target; the same, not
/// shared; and fire proofing.
pub(super) const METEOR: u32 = 99_000;
const METEOR_AND_BOLT: u32 = 99_001;
const FIREBALL_RAIN: u32 = 99_002;
const LIMITED_METEOR: u32 = 99_003;
const SLOW_METEOR: u32 = 99_004;
const FIRE_PROOF: u32 = 99_005;

fn meteor(id: u32, shared: bool) -> crate::game::data::spells::SpellTemplate {
    let mut s = spell(
        id,
        &[SpellEffect {
            effect: 2,
            base_points: 301,
            base_dice: 1,
            die_sides: 1,
            target_a: 16,
            radius_index: 13,
            ..Default::default()
        }],
    );
    s.school = 2;
    if shared {
        s.script_name = "spell_meteor".into();
    }
    s
}

pub(super) fn world() -> GameData {
    let mut d = data();
    d.spells.radii.insert(13, 10.0);
    let mut both = meteor(METEOR_AND_BOLT, true);
    both.effects[1] = SpellEffect {
        effect: 2,
        base_points: 89,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut limited = meteor(LIMITED_METEOR, true);
    limited.max_affected_targets = 2;
    let mut slow = meteor(SLOW_METEOR, true);
    slow.speed = 20.0;
    for s in [
        meteor(METEOR, true),
        both,
        meteor(FIREBALL_RAIN, false),
        limited,
        slow,
        spell(
            FIRE_PROOF,
            &[aura_effect(AURA_SCHOOL_IMMUNITY, 1, 1 << 2, 1)],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

const FULL: u32 = 100_000;

pub(super) fn lost(m: &Map, g: Guid) -> u32 {
    FULL - m
        .unit_fields(g)
        .map_or(FULL, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

pub(super) fn heal(m: &mut Map, gs: &[Guid]) {
    for g in gs {
        if let Some(f) = m.unit_fields_mut(*g) {
            f.set_u32(UNIT_FIELD_HEALTH, FULL);
        }
    }
}

/// Thistle Tea's `SharedDamage`: a meteor's damage is divided among the
/// living it falls on within its radius, each share truncated; a target
/// limit is applied before the count; another effect's own target isn't
/// divided; ordinary area damage isn't shared; a recipient immune to it
/// keeps its share from the others; and the count is the launch's, though
/// recipients go before it lands.
#[tokio::test]
async fn a_meteors_damage_is_shared_among_those_it_falls_on() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brutes: Vec<Guid> = (0..3)
            .map(|i| spawn(m, ga, BRUTE, 20, 10.0 + i as f32))
            .collect();
        let dead = spawn(m, ga, BRUTE, 20, 9.0);
        crate::game::map::combat::deal_damage(m, ga, dead, 10_000_000);
        let far = spawn(m, ga, BRUTE, 20, 40.0);
        let at = m.position_of(brutes[1]).unwrap();
        let mut seen = Vec::new();
        let mut fall = |m: &mut Map, id: u32, unit: Option<Guid>| {
            heal(m, &brutes);
            heal(m, &[far]);
            spells::cast_triggered_with(
                m,
                ga,
                id,
                spells::Targets {
                    unit,
                    dest: Some(at),
                    ..Default::default()
                },
            );
            seen.push((
                brutes.iter().map(|g| lost(m, *g)).collect::<Vec<_>>(),
                lost(m, far),
            ));
        };
        fall(m, METEOR, None);
        fall(m, LIMITED_METEOR, None);
        fall(m, METEOR_AND_BOLT, Some(brutes[0]));
        fall(m, FIREBALL_RAIN, None);
        // The first proof against fire: the others' shares stay a third.
        let proof = m.realm.data.spells.get(FIRE_PROOF).cloned().unwrap();
        auras::apply(m, brutes[0], brutes[0], &proof, 0, 1);
        fall(m, METEOR, None);
        auras::remove_spell(m, brutes[0], FIRE_PROOF, None);
        seen
    })
    .await;
    assert_eq!(
        out[0],
        (vec![100, 100, 100], 0),
        "a third each, truncated; the dead and the far don't count"
    );
    let limited: Vec<u32> = out[1].0.iter().copied().filter(|d| *d > 0).collect();
    assert_eq!(limited, vec![151, 151], "two of the three, a half each");
    assert_eq!(
        out[2].0,
        vec![190, 100, 100],
        "the second effect's target takes it whole besides"
    );
    assert_eq!(
        out[3].0,
        vec![302, 302, 302],
        "ordinary area damage is whole"
    );
    assert_eq!(
        out[4].0,
        vec![0, 100, 100],
        "the proofed one keeps its share"
    );
}

/// Thistle Tea's "retains counts when recipients disappear after launch":
/// a meteor on its way keeps the count it was launched with, though one of
/// those it was falling on has gone when it lands.
#[tokio::test]
async fn a_meteor_on_its_way_keeps_its_count() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brutes: Vec<Guid> = (0..3)
            .map(|i| spawn(m, ga, BRUTE, 20, 10.0 + i as f32))
            .collect();
        let at = m.position_of(brutes[1]).unwrap();
        m.spells.travel = Some(true);
        spells::cast_triggered_with(
            m,
            ga,
            SLOW_METEOR,
            spells::Targets {
                dest: Some(at),
                ..Default::default()
            },
        );
        let flying = m.spells.in_flight.len();
        m.remove_creature(brutes[2]);
        m.now += Duration::from_secs(5);
        spells::update(m);
        m.spells.travel = None;
        (flying, [brutes[0], brutes[1]].map(|g| lost(m, g)))
    })
    .await;
    assert!(out.0 >= 1, "it flies");
    assert_eq!(out.1, [100, 100], "still a third each");
}

const DRAIN: u32 = 99_010;
const SOUL_SHARD: u32 = 6265;
const WORTHLESS: u32 = 3_740;

fn drain_world() -> GameData {
    let mut d = world();
    let mut drain = spell(
        DRAIN,
        &[aura_effect(
            crate::game::map::auras::AURA_CHANNEL_DEATH_ITEM,
            1,
            0,
            25,
        )],
    );
    drain.effects[0].item_type = u64::from(SOUL_SHARD);
    drain.family_name = 5;
    d.spells.spells.insert(DRAIN, drain);
    d.items.insert(
        SOUL_SHARD,
        crate::game::data::items::ItemTemplate {
            entry: SOUL_SHARD,
            name: "Soul Shard".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    let mut worthless = crate::game::tests_p3_combat::template(WORTHLESS, 7);
    worthless.xp_multiplier = Some(0.0);
    d.creature_templates.insert(WORTHLESS, worthless);
    d
}

/// Thistle Tea's `DeathItem` for soul shards: none from a creature worth
/// no experience, though it is its drainer's own; one from a player not
/// grey to the warlock, who has no tap; and none to a creature's own
/// drain.
#[tokio::test]
async fn a_soul_shard_is_of_a_death_worth_something_to_a_warlock() {
    use crate::game::map::spells::outcome::death_items;
    let realm = crate::game::harness::realm_with(drain_world());
    let mut w = crate::game::harness::Client::connect(&realm, "W");
    let gw = w.enter("Wanda", Race::Human, Class::Warlock).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(gw).unwrap().map();
    let out = on(&map, move |m| {
        let s = m.realm.data.spells.get(DRAIN).cloned().unwrap();
        let drain = |m: &mut Map, caster: Guid, target: Guid| {
            auras::apply_spell(m, caster, target, &s, &[(0, 1)], Default::default()).unwrap();
        };
        let brute = spawn(m, gw, BRUTE, 20, 3.0);
        let worthless = spawn(m, gw, WORTHLESS, 20, 4.0);
        for g in [brute, worthless] {
            m.creature_mut(g).unwrap().tapped_by = Some(gw);
            drain(m, gw, g);
        }
        let worth = death_items(m, brute, 10_000_000);
        let none = death_items(m, worthless, 10_000_000);
        // Bob, level 1 as she is: no tap to ask for.
        drain(m, gw, gb);
        let bob = death_items(m, gb, 10_000_000);
        // A creature's own drain on another: nothing for it.
        let other = spawn(m, gw, BRUTE, 20, 5.0);
        drain(m, brute, other);
        let given = death_items(m, other, 10_000_000);
        crate::game::map::spells::outcome::give_death_items(m, other, given);
        (worth, none, bob)
    })
    .await;
    assert_eq!(out.0, vec![(gw, SOUL_SHARD, 1)], "an ordinary creature's");
    assert_eq!(out.1, vec![], "none from one worth no experience");
    assert_eq!(out.2, vec![(gw, SOUL_SHARD, 1)], "Bob's, with no tap");
    let _ = &mut b;
}

/// A leap ten yards ahead.
const LEAP: u32 = 99_020;

/// Thistle Tea's `Leap` "supports creature targets through the same
/// resolver" and "rejects taxi passengers": a creature's leap puts it ten
/// yards on, as a player's sends them; one on a flight doesn't leap.
#[tokio::test]
async fn a_leap_moves_a_creature_too_but_not_one_on_a_flight() {
    let mut d = world();
    d.spells.radii.insert(14, 10.0);
    d.spells.spells.insert(
        LEAP,
        spell(
            LEAP,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEAP,
                target_a: 1,
                radius_index: 14,
                ..Default::default()
            }],
        ),
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let (moved, flew) = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let before = m.position_of(brute).unwrap();
        spells::cast_triggered(m, brute, LEAP, Some(brute));
        let after = m.position_of(brute).unwrap();
        let moved = crate::game::map::combat::dist(before, after);
        // On a flight, Alice stays on it.
        let pos = m.players[&ga].position();
        let far = Vector3d {
            x: pos.x + 1000.0,
            ..pos
        };
        m.players.get_mut(&ga).unwrap().flight = Some(crate::game::map::npc::Flight {
            walk: crate::game::map::creature::Walk::new(vec![pos, far], 1.0, m.now, 0, true),
            destination: 0,
            mount: 0,
        });
        spells::cast_triggered(m, ga, LEAP, Some(ga));
        (moved, m.players[&ga].flight.is_some())
    })
    .await;
    assert!((moved - 10.0).abs() < 0.5, "the brute leapt {moved} yards");
    assert!(flew, "still flying");
    assert!(
        a.gets(crate::game::tests_p3_entity::NONE_WITHIN, |m| {
            matches!(
                m,
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)
            )
            .then_some(())
        })
        .await
        .is_none(),
        "and not sent anywhere"
    );
}

const TOWN_GUARD: u32 = 3_750;

/// Thistle Tea's `LocalDefense.resolve`: a town guard felled by a Horde
/// player's pet warns the Alliance its area is under attack, as by the
/// player; a guard a creature kills, an ordinary creature, and a guard in
/// an area not known warn no one.
#[tokio::test]
async fn a_fallen_guard_warns_its_side_of_a_pets_kill_and_not_otherwise() {
    let mut d = world();
    let mut guard = crate::game::tests_p3_combat::template(TOWN_GUARD, 7);
    guard.flags_extra = 0x400;
    d.creature_templates.insert(TOWN_GUARD, guard);
    let realm = crate::game::harness::realm_with(d);
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut h = crate::game::harness::Client::connect(&realm, "H");
    let gh = h.enter("Hordo", Race::Undead, Class::Warlock).await;
    let map = realm.registry.online(ga).unwrap().map();
    assert_eq!(
        realm.registry.online(gh).unwrap().map().key,
        map.key,
        "one map"
    );
    a.clear();
    // His pet (a creature he charms) fells a guard, Hordo in area 40.
    on(&map, move |m| {
        m.players.get_mut(&gh).unwrap().area = 40;
        let guard = spawn(m, ga, TOWN_GUARD, 20, 5.0);
        let pet = spawn(m, ga, BRUTE, 20, 6.0);
        crate::game::map::pets::charmed(m, gh, pet, false);
        crate::game::map::combat::deal_damage(m, pet, guard, 10_000_000);
    })
    .await;
    let raw = a
        .wait_frame("SMSG_ZONE_UNDER_ATTACK", 0x254, |_| true)
        .await;
    assert_eq!(raw, 40u32.to_le_bytes(), "Alice hears of area 40");
    // A creature's kill, an ordinary creature, a guard of no known area.
    a.clear();
    on(&map, move |m| {
        m.players.get_mut(&gh).unwrap().area = 41;
        let guard = spawn(m, ga, TOWN_GUARD, 20, 5.0);
        let brute = spawn(m, ga, BRUTE, 20, 6.0);
        crate::game::map::combat::deal_damage(m, brute, guard, 10_000_000);
        m.players.get_mut(&gh).unwrap().area = 42;
        let ordinary = spawn(m, ga, BRUTE, 20, 7.0);
        crate::game::map::combat::deal_damage(m, gh, ordinary, 10_000_000);
        let p = m.players.get_mut(&gh).unwrap();
        p.area = 0;
        p.zone = 0;
        let lost = spawn(m, ga, TOWN_GUARD, 20, 8.0);
        crate::game::map::combat::deal_damage(m, gh, lost, 10_000_000);
    })
    .await;
    assert!(
        !crate::game::tests_p3_entity::gets_frame(&mut a, 0x254, |_| true).await,
        "no warning for any of those"
    );
}
