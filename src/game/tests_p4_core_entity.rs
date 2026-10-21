//! Phase 3, round 4, against Thistle Tea's `entity_test.exs`: a script's
//! health set to a share and drained, a floor of health, a split too small
//! to take, what a death clears (a walk, a snare, temporary auras and their
//! spell modifiers, one soul shard a warlock), a spirit of redemption
//! taking nothing, the leash, and who heeds a distraction.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::creature::Motion;
use crate::game::map::spells::redemption;
use crate::game::map::{Map, auras, creature_combat, event_ai, scripts};
use crate::game::tests_p3_combat::*;

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH)
}

fn alive(m: &Map, g: Guid) -> bool {
    m.unit_view(g).is_some_and(|u| u.alive)
}

fn step(command: u32, d1: u32) -> ScriptStep {
    ScriptStep {
        command,
        datalong: [d1, 0, 0, 0],
        ..Default::default()
    }
}

#[tokio::test]
async fn a_script_sets_a_creatures_health_to_a_share_and_drains_it_dying_only_of_its_last_point() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let f = m.unit_fields_mut(brute).unwrap();
            f.set_u32(UNIT_FIELD_MAXHEALTH, 200);
            f.set_u32(UNIT_FIELD_HEALTH, 90);
            scripts::execute(m, &step(scripts::C_SET_HEALTH_PCT, 25), brute, brute);
            let share = health(m, brute);
            // A drain of 50 from 52 leaves 2; from 51, it dies of it.
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 52);
            scripts::execute(m, &step(scripts::C_LOSE_HEALTH, 50), brute, brute);
            let drained = (health(m, brute), alive(m, brute));
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 51);
            scripts::execute(m, &step(scripts::C_LOSE_HEALTH, 50), brute, brute);
            let died = (health(m, brute), alive(m, brute));
            // The dead stay dead.
            scripts::execute(m, &step(scripts::C_SET_HEALTH_PCT, 50), brute, brute);
            (share, drained, died, health(m, brute), alive(m, brute))
        })
        .await
        .unwrap();
    assert_eq!(r, (50, (2, true), (0, false), 0, false));
}

#[tokio::test]
async fn a_floor_of_health_holds_against_every_blow_and_a_blow_short_of_death_kills_no_one() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 100);
            event_ai::scripted_mut(m, brute).unwrap().invincible_hp = Some(10);
            combat::deal_damage(m, g, brute, 100);
            let first = health(m, brute);
            combat::deal_damage(m, g, brute, 100);
            let second = (health(m, brute), alive(m, brute));
            // Without the floor, a blow short of its health leaves it be.
            event_ai::scripted_mut(m, brute).unwrap().invincible_hp = Some(0);
            combat::deal_damage(m, g, brute, 5);
            let short = (health(m, brute), alive(m, brute));
            combat::deal_damage(m, g, brute, 5);
            (first, second, short, alive(m, brute))
        })
        .await
        .unwrap();
    assert_eq!(r, (10, (10, true), (5, true), false));
}

const SOUL_LINK: u32 = 97_201;

#[tokio::test]
async fn a_split_too_small_to_take_leaves_the_whole_blow_where_it_fell() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let pet = spawn(m, g, HELPER, 20, 3.0);
            // Thirty percent of her harm goes to the helper.
            let mut s = SpellTemplate {
                id: SOUL_LINK,
                name: "Soul Link".into(),
                equipped_item_class: -1,
                ..Default::default()
            };
            s.effects[0] = SpellEffect {
                effect: 6,
                aura: 81,
                misc_value: 1,
                target_a: 1,
                ..Default::default()
            };
            auras::apply_spell(m, pet, g, &s, &[(0, 30)], Default::default()).unwrap();
            let max = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let (pet_before, before) = (health(m, pet), max);
            m.unit_fields_mut(g)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, max);
            // 100 splits 70 and 30.
            let (left, shared) = auras::absorb_and_share(m, brute, g, 100, 1);
            let pet_after = health(m, pet);
            // 2 makes no share at all.
            let (small_left, small_shared) = auras::absorb_and_share(m, brute, g, 2, 1);
            (
                (left, shared, pet_before - pet_after),
                (small_left, small_shared, pet_after - health(m, pet)),
                before,
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, (70, 30, 30));
    assert_eq!(r.1, (2, 0, 0));
}

#[tokio::test]
async fn death_stops_a_walk_and_takes_a_snare_off_its_speed() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let base = m.creature(brute).unwrap().speeds.run;
            aura_of(m, brute, 97_301, auras::AURA_MOD_DECREASE_SPEED, 0, -50);
            crate::game::map::derive::unit(m, brute);
            let snared = m.creature(brute).unwrap().speeds.run;
            let to = {
                let p = m.creature(brute).unwrap().position;
                Vector3d { x: p.x + 20.0, ..p }
            };
            scripts::move_to(m, brute, to, true);
            let walking = matches!(m.creature(brute).unwrap().motion, Motion::Moving(_));
            combat::deal_damage(m, g, brute, 1_000_000);
            let c = m.creature(brute).unwrap();
            (
                base,
                snared,
                walking,
                matches!(c.motion, Motion::Idle { .. }),
                c.auras.has(auras::AURA_MOD_DECREASE_SPEED),
                c.speeds.run,
            )
        })
        .await
        .unwrap();
    let (base, snared, walking, idle, still_snared, after) = r;
    assert!((snared - base / 2.0).abs() < 0.01, "{base} {snared}");
    assert!(walking && idle, "walking, then still");
    assert!(!still_snared);
    assert!((after - base).abs() < 0.01, "{after} {base}");
}

const TALENT_PASSIVE: u32 = 97_401;
const QUICK_CASTS: u32 = 97_402;

#[tokio::test]
async fn death_keeps_the_passives_and_takes_the_rest_telling_the_client_its_modifiers_are_gone() {
    let mut d = data();
    let mut passive = SpellTemplate {
        id: TALENT_PASSIVE,
        name: "Talent".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    passive.attributes[0] = 0x40;
    passive.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    let mut quick = SpellTemplate {
        id: QUICK_CASTS,
        name: "Quick Casts".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    quick.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_ADD_PCT_MODIFIER,
        misc_value: 10,
        item_type: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(TALENT_PASSIVE, passive);
    d.spells.spells.insert(QUICK_CASTS, quick);
    let (_realm, mut a, g, map) = setup_with(d, Class::Warrior).await;
    map.query(move |m| {
        for (id, amount) in [(TALENT_PASSIVE, 1), (QUICK_CASTS, -100)] {
            let s = m.realm.data.spells.get(id).unwrap().clone();
            auras::apply_spell(m, g, g, &s, &[(0, amount)], Default::default()).unwrap();
        }
    })
    .await
    .unwrap();
    a.wait_frame("the modifier", 0x267, |b| {
        b == [0, 10, 0x9C, 0xFF, 0xFF, 0xFF]
    })
    .await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            combat::deal_damage(m, brute, g, 10_000_000);
            let a = auras::auras(m, g).unwrap();
            (
                alive(m, g),
                a.has_spell(TALENT_PASSIVE),
                a.has_spell(QUICK_CASTS),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (false, true, false));
    a.wait_frame("the modifier gone", 0x267, |b| b == [0, 10, 0, 0, 0, 0])
        .await;
}

const DRAIN_SOUL: u32 = 97_501;
const SHADOWBURN: u32 = 97_502;
const SOUL_SHARD: u32 = 6265;

#[tokio::test]
async fn a_warlocks_two_drains_on_a_kill_give_one_shard_and_a_blow_short_of_it_none() {
    let mut d = data();
    for id in [DRAIN_SOUL, SHADOWBURN] {
        let mut s = SpellTemplate {
            id,
            name: format!("Drain {id}"),
            equipped_item_class: -1,
            family_name: 5,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: auras::AURA_CHANNEL_DEATH_ITEM,
            item_type: u64::from(SOUL_SHARD),
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let (_realm, _a, g, map) = setup_with(d, Class::Warlock).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            m.creature_mut(brute).unwrap().tapped_by = Some(g);
            for id in [DRAIN_SOUL, SHADOWBURN] {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                auras::apply_spell(m, g, brute, &s, &[(0, 1)], Default::default()).unwrap();
            }
            let hp = health(m, brute);
            let short = crate::game::map::spells::outcome::death_items(m, brute, g, hp - 1);
            let lethal = crate::game::map::spells::outcome::death_items(m, brute, g, hp);
            (short, lethal)
        })
        .await
        .unwrap();
    assert_eq!(r.0, vec![]);
    assert_eq!(r.1, vec![(g, SOUL_SHARD, 1)]);
}

fn with_redemption() -> GameData {
    let mut d = data();
    for id in std::iter::once(redemption::FORM).chain(redemption::STATE) {
        let mut s = SpellTemplate {
            id,
            name: "Spirit of Redemption".into(),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: auras::AURA_DUMMY,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

#[tokio::test]
async fn a_spirit_of_redemption_takes_no_harm_till_its_end_kills_it() {
    let (_realm, _a, g, map) = setup_with(with_redemption(), Class::Priest).await;
    let r = map
        .query(move |m| {
            m.players
                .get_mut(&g)
                .unwrap()
                .spells
                .push(redemption::TALENT);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let max = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            combat::deal_damage(m, brute, g, max + 100);
            let spirit = (redemption::spirit(m, g), alive(m, g), health(m, g) == max);
            // Blows and a lethal one: nothing.
            combat::deal_damage(m, brute, g, 50);
            forced_swing(m, brute, g, Outcome::Hit);
            combat::deal_damage(m, brute, g, max * 10);
            let untouched = (health(m, g) == max, alive(m, g));
            redemption::aura_gone(m, g, redemption::STATE[1], true);
            (spirit, untouched, alive(m, g))
        })
        .await
        .unwrap();
    assert_eq!(r, ((true, true, true), (true, true), false));
}

const LEASHED: u32 = 3710;
const FAR_LEASHED: u32 = 3711;
const UNLEASHED: u32 = 3712;

#[tokio::test]
async fn the_leash_snaps_past_its_range_at_once_and_else_waits_its_twelve_seconds_but_not_everywhere()
 {
    let mut d = data();
    for (entry, leash, extra) in [
        (LEASHED, 50.0, 0),
        (FAR_LEASHED, 120.0, 0),
        (UNLEASHED, 0.0, 0x1),
    ] {
        let mut t = template(entry, 7);
        t.leash_range = leash;
        t.flags_extra = extra;
        d.creature_templates.insert(entry, t);
    }
    // The Deadmines, a dungeon.
    d.maps.insert(
        36,
        crate::game::data::MapInfo {
            id: 36,
            internal_name: "DeadMines".into(),
            name: "Deadmines".into(),
            instance_type: 1,
            player_limit: 10,
        },
    );
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let now = m.now;
            // Each `yards` from where its fight began, far from Alice, its
            // last hostile act `ago`.
            let place = |m: &mut Map, entry: u32, yards: f32, ago: u64| {
                let c = spawn(m, g, entry, 20, 2.0);
                let p = m.players[&g].position();
                let origin = Vector3d {
                    x: p.x + 200.0,
                    ..p
                };
                let c_ = m.creature_mut(c).unwrap();
                c_.engagement.origin = Some(origin);
                c_.position = Vector3d {
                    x: origin.x + yards,
                    ..origin
                };
                c_.engagement.last_hostile = Some(now - Duration::from_millis(ago));
                creature_combat::should_evade(m, c, g)
            };
            let hard = place(m, LEASHED, 60.0, 500);
            let far_soon = place(m, FAR_LEASHED, 100.0, 6_000);
            let far_late = place(m, FAR_LEASHED, 100.0, 12_001);
            let unleashed = place(m, UNLEASHED, 100.0, 12_001);
            let ordinary = place(m, BRUTE, 100.0, 12_001);
            // A dungeon's creature doesn't leash at the ordinary range (a
            // battleground's does: question #9).
            let world = m.key;
            m.key = crate::game::map::MapKey {
                map: 36,
                instance: 1,
            };
            let dungeon = place(m, BRUTE, 100.0, 12_001);
            let dungeon_hard = place(m, LEASHED, 60.0, 500);
            m.key = world;
            (
                hard,
                far_soon,
                far_late,
                unleashed,
                ordinary,
                dungeon,
                dungeon_hard,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false, true, false, true, false, true));
}

#[tokio::test]
async fn a_distraction_holds_only_a_creature_that_can_heed_it_and_ends_with_a_fight_or_its_time() {
    use crate::game::map::creature_motion::{distract, distracted, held_by_distraction};
    let (_realm, _a, g, map) = setup(Class::Rogue).await;
    let r = map
        .query(move |m| {
            let point = |m: &Map| {
                let p = m.players[&g].position();
                Vector3d {
                    x: p.x,
                    y: p.y + 5.0,
                    ..p
                }
            };
            let at = point(m);
            // The dead, and the stunned, confused, feared or feigning.
            let dead = spawn(m, g, BRUTE, 20, 2.0);
            combat::deal_damage(m, g, dead, 1_000_000);
            distract(m, dead, at, 10);
            let mut ignored = vec![held_by_distraction(m, dead)];
            for (n, kind) in [
                auras::AURA_MOD_STUN,
                auras::AURA_MOD_CONFUSE,
                auras::AURA_MOD_FEAR,
                auras::AURA_FEIGN_DEATH,
            ]
            .into_iter()
            .enumerate()
            {
                let c = spawn(m, g, BRUTE, 20, 3.0 + n as f32);
                aura_of(m, c, 97_801 + n as u32, kind, 0, 1);
                distract(m, c, at, 10);
                ignored.push(held_by_distraction(m, c));
            }
            // One that heeds it, until its time is up.
            let calm = spawn(m, g, BRUTE, 20, 8.0);
            distract(m, calm, at, 10);
            let held = distracted(m, calm);
            m.creature_mut(calm).unwrap().engagement.distracted_until =
                Some(m.now - Duration::from_millis(1));
            let expired = (
                distracted(m, calm),
                m.creature(calm).unwrap().engagement.distracted_until,
            );
            // Another, until a fight: then for good.
            let other = spawn(m, g, BRUTE, 20, 9.0);
            distract(m, other, at, 10);
            crate::game::map::ai::attacked(m, other, g);
            combat::engage(m, g, other);
            let fought = distracted(m, other);
            (
                ignored,
                held,
                expired,
                fought,
                m.creature(other).unwrap().engagement.distracted_until,
            )
        })
        .await
        .unwrap();
    let (ignored, held, expired, fought, cleared) = r;
    assert_eq!(ignored, vec![false; 5]);
    assert!(held);
    assert_eq!(expired, (false, None));
    assert!(!fought);
    assert_eq!(cleared, None);
}

const KEEPS_MOVING: u32 = 3713;

#[tokio::test]
async fn a_talk_pause_holds_a_wanderer_or_a_patroller_three_minutes_but_never_less_than_it_waits() {
    use crate::game::map::creature::{MOVEMENT_IDLE, MOVEMENT_RANDOM, MOVEMENT_WAYPOINT};
    use crate::game::map::creature_motion::{MOVEMENT_CYCLIC, TALK_PAUSE, talk_pause};
    let mut d = data();
    let mut t = template(KEEPS_MOVING, 7);
    t.flags_extra = 0x20;
    d.creature_templates.insert(KEEPS_MOVING, t);
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let now = m.now;
            let make = |m: &mut Map, entry: u32, kind: u8, yards: f32| {
                let c = spawn(m, g, entry, 20, yards);
                m.creature_mut(c).unwrap().movement_type = kind;
                let to = {
                    let p = m.creature(c).unwrap().position;
                    Vector3d { x: p.x + 20.0, ..p }
                };
                let (now, id) = (m.now, m.next_spline_id);
                m.next_spline_id += 1;
                let from = m.creature(c).unwrap().position;
                let _ = m
                    .creature_mut(c)
                    .unwrap()
                    .walk(vec![from, to], true, now, id);
                c
            };
            let held = |m: &Map, c: Guid| match m.creature(c).unwrap().motion {
                Motion::Idle { until: Some(t) } => Some(t.duration_since(now)),
                _ => None,
            };
            // A wanderer, walking: it stops for three minutes.
            let wanderer = make(m, BRUTE, MOVEMENT_RANDOM, 2.0);
            let was_walking = matches!(m.creature(wanderer).unwrap().motion, Motion::Moving(_));
            talk_pause(m, wanderer);
            let wanderer_held = held(m, wanderer);
            // A patroller keeps the point it was bound for.
            let patroller = make(m, BRUTE, MOVEMENT_WAYPOINT, 3.0);
            m.creature_mut(patroller).unwrap().waypoint = 4;
            talk_pause(m, patroller);
            let patrol = (held(m, patroller), m.creature(patroller).unwrap().waypoint);
            // A longer wait stands.
            let waiting = make(m, BRUTE, MOVEMENT_RANDOM, 4.0);
            let longer = now + TALK_PAUSE + Duration::from_secs(60);
            m.creature_mut(waiting).unwrap().motion = Motion::Idle {
                until: Some(longer),
            };
            talk_pause(m, waiting);
            let kept = held(m, waiting);
            // A cyclic path, one standing, one fighting, one that keeps
            // moving, one on a script's move: none stops.
            let mut untouched = Vec::new();
            for (entry, kind) in [
                (BRUTE, MOVEMENT_CYCLIC),
                (BRUTE, MOVEMENT_IDLE),
                (BRUTE, MOVEMENT_RANDOM),
                (KEEPS_MOVING, MOVEMENT_RANDOM),
            ] {
                let c = make(m, entry, kind, 5.0 + untouched.len() as f32);
                if untouched.len() == 2 {
                    m.creature_mut(c).unwrap().combat.attacking = true;
                }
                let before = held(m, c);
                talk_pause(m, c);
                untouched.push(held(m, c) == before);
            }
            let scripted = spawn(m, g, BRUTE, 20, 9.0);
            m.creature_mut(scripted).unwrap().movement_type = MOVEMENT_RANDOM;
            let to = {
                let p = m.creature(scripted).unwrap().position;
                Vector3d { x: p.x + 20.0, ..p }
            };
            scripts::move_to(m, scripted, to, false);
            talk_pause(m, scripted);
            untouched.push(matches!(
                m.creature(scripted).unwrap().motion,
                Motion::Moving(_)
            ));
            (was_walking, wanderer_held, patrol, kept, untouched)
        })
        .await
        .unwrap();
    let (was_walking, wanderer_held, patrol, kept, untouched) = r;
    assert!(was_walking);
    assert_eq!(wanderer_held, Some(TALK_PAUSE));
    assert_eq!(patrol, (Some(TALK_PAUSE), 4));
    assert_eq!(kept, Some(TALK_PAUSE + Duration::from_secs(60)));
    assert_eq!(untouched, vec![true; 5]);
}
