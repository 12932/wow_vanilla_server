//! Round 9's creature tests, ported from Thistle Tea's
//! `test/game/core/ai/bt/caster_chase_test.exs`, `mob/spells_test.exs`,
//! `scripted_melee_test.exs`, `mob_test.exs` (scripted waypoint routes),
//! `test/game/core/combat/damage_received_test.exs` and
//! `test/game/core/aura/priority_test.exs` (docs/parity/matrix.csv): a
//! caster standing off from the air, a list's instant cast and a script's,
//! a script's melee spell held by the swing's checks, a script's path
//! walked and its point told once, a power burn's damage the target's
//! modifiers take, and a party aura shown on its caster.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::{AiEvent, EF_REPEATABLE, EVENT_MOVEMENT_INFORM};
use crate::game::data::spells::{
    self as sd, CreatureSpell, SpellDuration, SpellEffect, SpellTemplate,
};
use crate::game::data::{GameData, Waypoint};
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::creature::Motion;
use crate::game::map::spells::lists;
use crate::game::map::{Map, ai, combat, event_ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, START, aura_of, setup_with, spawn};
use crate::game::tests_p3_entity::step;

/// A passive party aura (a totem's, a battle standard's), the same not
/// passive, and a passive of no visual.
const PARTY_PASSIVE: u32 = 95_990;
const PARTY: u32 = 95_991;
const PASSIVE: u32 = 95_992;
/// An instant buff a list casts, that starts the swing over.
const QUICK_BUFF: u32 = 95_993;
const QUICK_LIST: u32 = 95_993;
/// An instant buff that ends auto attack.
const STOPS_ATTACK: u32 = 95_994;
/// A melee spell: a point of shadow.
const JAB: u32 = 95_995;
/// A burn of 40 mana at half, of half the coefficient; and its aura.
const BURN: u32 = 95_996;
const BURNING: u32 = 95_997;
/// The movement script of a path's point, and the EventAI script of its
/// arrival.
const ARRIVED: u32 = 95_998;

const SHADOW: u32 = 5;
const SHADOW_MASK: i32 = 1 << SHADOW;

fn buff(id: u32, aura: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura,
        base_points: 4,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut add = |s: SpellTemplate| {
        d.spells.spells.insert(s.id, s);
    };
    let mut party = buff(PARTY_PASSIVE, AURA_MOD_STAT);
    party.effects[0].effect = 35;
    party.attributes[0] |= sd::ATTR_PASSIVE;
    add(party.clone());
    party.id = PARTY;
    party.attributes[0] &= !sd::ATTR_PASSIVE;
    add(party);
    let mut passive = buff(PASSIVE, AURA_MOD_STAT);
    passive.attributes[0] |= sd::ATTR_PASSIVE;
    add(passive);
    let mut quick = buff(QUICK_BUFF, AURA_MOD_STAT);
    quick.interrupt_flags = 0x08;
    add(quick);
    let mut stops = buff(STOPS_ATTACK, AURA_MOD_STAT);
    stops.attributes[0] |= sd::ATTR_CANCELS_AUTO_ATTACK;
    add(stops);
    let mut jab = SpellTemplate {
        id: JAB,
        name: "Jab".into(),
        school: SHADOW,
        dmg_class: 1,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    jab.attributes[3] = sd::ATTR_EX3_ALWAYS_HIT;
    jab.effects[0] = SpellEffect {
        effect: 2,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    add(jab);
    for (id, effect, aura) in [(BURN, 62, 0), (BURNING, 6, AURA_POWER_BURN_MANA)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Burn {id}"),
            school: SHADOW,
            dmg_class: 1,
            equipped_item_class: -1,
            range_index: 1,
            duration_index: 900,
            ..Default::default()
        };
        s.attributes[3] = sd::ATTR_EX3_ALWAYS_HIT;
        s.effects[0] = SpellEffect {
            effect,
            aura,
            base_points: 39,
            base_dice: 1,
            die_sides: 1,
            multiple_value: 0.5,
            bonus_coefficient: 0.5,
            amplitude: if aura != 0 { 1000 } else { 0 },
            target_a: 6,
            ..Default::default()
        };
        add(s);
    }
    d.spells.durations.insert(
        900,
        SpellDuration {
            base: 3000,
            per_level: 0,
            max: 3000,
        },
    );
    d.spells.creature_spells.insert(
        QUICK_LIST,
        vec![CreatureSpell {
            spell: QUICK_BUFF,
            probability: 100,
            target: 0,
            delay_repeat: (4_000, 4_000),
            ..Default::default()
        }],
    );
    // A path's point 37 runs its movement script (phase up 10); its
    // arrival told, EventAI's (phase up 1).
    d.scripts
        .movement
        .insert(ARRIVED, vec![step(44, [10, 1, 0, 0])]);
    d.scripts
        .ai_scripts
        .insert(ARRIVED, vec![step(44, [1, 1, 0, 0])]);
    d.scripts.ai_events.insert(
        BRUTE,
        vec![AiEvent {
            kind: EVENT_MOVEMENT_INFORM,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [2, 37, 0, 0],
            actions: [ARRIVED, 0, 0],
            ..Default::default()
        }],
    );
    d
}

/// The creatures take `n` turns of 50 ms.
fn turns(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_millis(50);
        ai::update_creatures(m);
        combat::update_creatures(m);
    }
}

fn at(east: f32, north: f32, up: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2 + up,
    }
}

/// Creature `g` put at `p`, its grid cell with it.
fn put(m: &mut Map, g: Guid, p: Vector3d) {
    let c = m.creature_mut(g).unwrap();
    let old = c.cell;
    c.position = p;
    c.home = p;
    c.cell = crate::game::map::grid::cell_of(p.x, p.y);
    let new = c.cell;
    m.grid.relocate(g, old, new);
}

/// Alice at her start, standing, facing east, with health to spare.
fn stand(m: &mut Map, g: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at(0.0, 0.0, 0.0);
    p.movement.orientation = 0.0;
    p.movement.flags = MovementInfo_MovementFlags::empty();
    p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    p.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
}

fn destination(m: &Map, g: Guid) -> Option<Vector3d> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.destination()),
        _ => None,
    }
}

fn spline(m: &Map, g: Guid) -> Option<u32> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.spline_id),
        _ => None,
    }
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    combat::dist(a, b) < 0.05
}

/// The user's call (#57), mangos-classic's `IsNeedVisibleSlot` ("passive
/// party auras shown even on caster"), over Thistle Tea's `visible?/2`
/// "passive area buffs are hidden on their caster but visible on
/// recipients and totems": a party aura, passive or not, takes a slot and
/// shows on its caster as on another; a passive of no visual takes none.
#[tokio::test]
async fn a_party_aura_shows_on_its_caster_passive_or_not() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 3.0);
            let put = |m: &mut Map, from: Guid, on: Guid, id: u32| {
                let s = m.realm.data.spells.get(id).cloned().unwrap();
                auras::apply_spell(m, from, on, &s, &[(0, 5)], auras::Context::default()).unwrap();
            };
            put(m, ga, ga, PASSIVE);
            put(m, ga, ga, PARTY_PASSIVE);
            put(m, k, k, PARTY_PASSIVE);
            put(m, k, ga, PARTY);
            let slots = |m: &Map, g: Guid| -> Vec<(u32, Option<usize>)> {
                auras::auras(m, g).map_or_else(Vec::new, |a| {
                    a.holders.iter().map(|h| (h.spell, h.slot)).collect()
                })
            };
            let field = |m: &Map, g: Guid, i: u16| m.unit_fields(g).unwrap().get_u32(i);
            (
                slots(m, ga),
                slots(m, k),
                [
                    field(m, ga, UNIT_FIELD_AURA),
                    field(m, ga, UNIT_FIELD_AURA + 1),
                ],
                field(m, k, UNIT_FIELD_AURA),
                auras::visible(&m.realm.data.spells.get(PARTY_PASSIVE).unwrap().clone()),
            )
        })
        .await
        .unwrap();
    let (hers, its, her_fields, its_field, visible) = out;
    assert_eq!(
        hers,
        [(PASSIVE, None), (PARTY_PASSIVE, Some(0)), (PARTY, Some(1))],
        "her own passive party aura shows"
    );
    assert_eq!(its, [(PARTY_PASSIVE, Some(0))], "so does a creature's");
    assert_eq!(her_fields, [PARTY_PASSIVE, PARTY]);
    assert_eq!(its_field, PARTY_PASSIVE);
    assert!(visible);
}

/// Thistle Tea's `CasterChase.destination/2` "approaches a grounded target
/// from the air while retaining the configured separation": a caster with
/// a chase distance of 25 flying 30 yards up comes down the line to its
/// victim to 25 yards of her, still in the air; one on the ground comes to
/// 25 yards of her on her level.
#[tokio::test]
async fn a_caster_stands_off_at_its_distance_from_the_air_or_on_the_ground() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            stand(m, ga);
            let mut ends = Vec::new();
            for (fly, up) in [(true, 30.0), (false, 0.0)] {
                let k = spawn(m, ga, BRUTE, 20, 3.0);
                put(m, k, at(40.0, 0.0, up));
                let s = event_ai::scripted_mut(m, k).unwrap();
                s.caster_chase_distance = Some(25.0);
                s.fly = fly;
                ai::attacked(m, k, ga);
                turns(m, 1);
                ends.push(destination(m, k));
                m.remove_creature(k);
            }
            ends
        })
        .await
        .unwrap();
    let her = at(0.0, 0.0, 0.0);
    let aloft = out[0].expect("the flyer comes on");
    assert!(
        (combat::dist(aloft, her) - 25.0).abs() < 0.1,
        "25 yards off: {aloft:?}"
    );
    assert!(
        (aloft.z - (her.z + 15.0)).abs() < 0.1,
        "down the line, in the air"
    );
    let ground = out[1].expect("the walker comes on");
    assert!(near(ground, at(25.0, 0.0, 0.0)), "25 yards off: {ground:?}");
}

/// Thistle Tea's `try_cast/3` "instant casts preserve swing resets
/// alongside spell-list timers" and `attempt_commanded_cast/5` "instant
/// attack cancellation survives the commanded cast handoff": an instant
/// buff a creature's list casts in its fight starts its swing over (its
/// base attack time from now), its entry ready again at its repeat and the
/// list looked at again at its tick; a script's instant buff that ends
/// auto attack ends it, its swing's time left as it was.
#[tokio::test]
async fn a_lists_instant_cast_starts_the_swing_over_and_a_scripts_ends_the_attack() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            stand(m, ga);
            let k = spawn(m, ga, BRUTE, 20, 2.0);
            ai::attacked(m, k, ga);
            combat::start_attack(m, k, ga);
            lists::set_list(m, k, QUICK_LIST);
            let now = m.now;
            m.combat_mut(k).unwrap().next_swing = Some(now + Duration::from_millis(300));
            lists::tick(m, k, ga);
            let period = m.unit_fields(k).unwrap().get_u32(UNIT_FIELD_BASEATTACKTIME);
            let c = m.creature(k).unwrap();
            let list = c.caster.spell_list.as_deref().unwrap();
            let listed = (
                c.auras.has_spell(QUICK_BUFF),
                c.combat.next_swing == Some(now + Duration::from_millis(u64::from(period))),
                list.ready_at == [now + Duration::from_secs(4)],
                list.next_at == Some(now + lists::LIST_TICK),
            );
            // A script's: the attack ends, its swing's time kept.
            let later = now + Duration::from_millis(700);
            m.combat_mut(k).unwrap().next_swing = Some(later);
            let went = scripts::cast(m, k, k, STOPS_ATTACK, 0);
            let c = m.creature(k).unwrap();
            (
                listed,
                (
                    went,
                    c.auras.has_spell(STOPS_ATTACK),
                    c.combat.attacking,
                    c.combat.next_swing == Some(later),
                ),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true, true, true), "the list's cast");
    assert_eq!(out.1, (true, true, false, true), "the script's");
}

/// Thistle Tea's `melee_attack_with_context/3` "replacement spells retain
/// range, world, life and control checks": a creature whose script made a
/// spell its swing casts it only where it could swing: not at a victim out
/// of reach, nor stunned or pacified, nor dead, nor at the dead; in reach,
/// free, it does.
#[tokio::test]
async fn a_melee_spell_goes_only_where_the_swing_could() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            stand(m, ga);
            let health = |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH);
            let brute = |m: &mut Map, east: f32| {
                let k = spawn(m, ga, BRUTE, 20, 2.0);
                put(m, k, at(east, 0.0, 0.0));
                let set = step(scripts::C_SET_MELEE_SPELL, [JAB, 0, 0, 0]);
                scripts::execute(m, &set, k, Guid::zero());
                ai::attacked(m, k, ga);
                k
            };
            // One swing's chance, due now.
            let swing = |m: &mut Map, k: Guid| {
                let before = health(m);
                let now = m.now;
                if let Some(c) = m.combat_mut(k) {
                    c.next_swing = Some(now);
                }
                turns(m, 1);
                before - health(m)
            };
            let far = brute(m, 20.0);
            let out_of_reach = swing(m, far);
            m.remove_creature(far);
            let k = brute(m, 1.5);
            aura_of(m, k, 95_980, AURA_MOD_STUN, 0, 0);
            let stunned = swing(m, k);
            auras::remove_spell(m, k, 95_980, None);
            aura_of(m, k, 95_981, AURA_MOD_PACIFY, 0, 0);
            let pacified = swing(m, k);
            auras::remove_spell(m, k, 95_981, None);
            let free = swing(m, k);
            let dead = brute(m, 1.5);
            ai::die(m, dead, ga);
            let from_the_dead = swing(m, dead);
            // At the dead: no cast, so its swing's time doesn't move on.
            crate::game::map::death::kill(m, ga);
            let now = m.now;
            m.combat_mut(k).unwrap().next_swing = Some(now);
            combat::try_swing(m, k);
            let at_the_dead = m.combat(k).unwrap().next_swing != Some(now);
            (
                [out_of_reach, stunned, pacified, from_the_dead],
                free,
                at_the_dead,
                event_ai::melee_spell(m, k),
            )
        })
        .await
        .unwrap();
    let (held, free, at_the_dead, spell) = out;
    assert_eq!(spell, Some(JAB));
    assert_eq!(held, [0; 4], "out of reach, stunned, pacified, dead");
    assert_eq!(free, 1, "in reach and free: the jab");
    assert!(!at_the_dead, "nothing at the dead");
}

fn point(point: u32, east: f32, wait_ms: u32, script_id: u32) -> Waypoint {
    let p = at(east, 5.0, 0.0);
    Waypoint {
        point,
        x: p.x,
        y: p.y,
        z: p.z,
        wait_ms,
        script_id,
        ..Default::default()
    }
}

/// Creature `g` on a script's path of `points`, once, from its first,
/// due to set off now.
fn on_route(m: &mut Map, g: Guid, points: Vec<Waypoint>) {
    let now = m.now;
    event_ai::scripted_mut(m, g).unwrap().route = Some(event_ai::Route {
        points: std::sync::Arc::new(points),
        repeat: false,
    });
    let c = m.creature_mut(g).unwrap();
    c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
    c.waypoint = 0;
    c.last_waypoint = None;
    c.motion = Motion::Idle { until: Some(now) };
}

/// Thistle Tea's scripted waypoint routes "the behavior tree prioritizes
/// the runtime route over spawn movement", "arrival reports the original
/// point once, walking straight there when no path is found" and
/// `set_next_waypoint_wait/3` "schedules from explicit time and clears
/// waypoint state": a creature with a path of its spawn's and one a script
/// gave it walks the script's, straight to its point with no navmesh; on
/// the way nothing is told; there, its point is told by its number (37)
/// once, its movement script runs once, and it waits the point's time from
/// then; the path over, nothing more is told.
#[tokio::test]
async fn a_scripts_path_goes_first_and_its_point_is_told_once_and_waited_at() {
    let mut d = world();
    d.waypoints_by_entry
        .insert(BRUTE, vec![point(1, -20.0, 0, 0)]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let k = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 3.0);
            m.creature_mut(k).unwrap().motion = Motion::Idle { until: None };
            k
        })
        .await
        .unwrap();
    a.sees(k).await;
    let out = map
        .query(move |m| {
            on_route(m, k, vec![point(37, 10.0, 250, ARRIVED)]);
            let from = m.position_of(k).unwrap();
            crate::game::map::world::update_creatures(m);
            let to = point(37, 10.0, 0, 0);
            let to = Vector3d {
                x: to.x,
                y: to.y,
                z: to.z,
            };
            let walk = match &m.creature(k).unwrap().motion {
                Motion::Moving(w) => Some(w.points.clone()),
                _ => None,
            };
            let set_off = walk.is_some_and(|p| p.len() == 2 && near(p[0], from) && near(p[1], to));
            // On its way: nothing told.
            m.now += Duration::from_millis(100);
            crate::game::map::world::update_creatures(m);
            let on_the_way = event_ai::phase(m, k);
            // There.
            m.now += Duration::from_secs(10);
            crate::game::map::world::update_creatures(m);
            let now = m.now;
            let c = m.creature(k).unwrap();
            let waits = matches!(c.motion, Motion::Idle { until: Some(t) } if t == now + Duration::from_millis(250));
            let there = (near(c.position, to), c.last_waypoint, event_ai::phase(m, k));
            // Its wait over, its path ends; nothing more is told.
            for _ in 0..4 {
                m.now += Duration::from_secs(1);
                crate::game::map::world::update_creatures(m);
            }
            let c = m.creature(k).unwrap();
            let after = (
                event_ai::phase(m, k),
                c.movement_type,
                event_ai::scripted(m, k).is_some_and(|s| s.route.is_none()),
            );
            (set_off, on_the_way, waits, there, after)
        })
        .await
        .unwrap();
    let (set_off, on_the_way, waits, there, after) = out;
    assert!(set_off, "straight to the script's point, not the spawn's");
    assert_eq!(on_the_way, 0, "nothing told on the way");
    assert_eq!(there, (true, Some(0), 11), "told once, its script once");
    assert!(waits, "its wait from its arrival");
    assert_eq!(
        after,
        (11, crate::game::map::creature::MOVEMENT_IDLE, true),
        "its path over, nothing more"
    );
}

/// Thistle Tea's scripted waypoint routes "a point movement suspends idle
/// waypoint navigation until arrival": a creature on a path sent by a
/// script to a point walks there, its path's walk not taking over on the
/// way; there, the move isn't a point of its path reached, and its path
/// goes on from the point it was bound for.
#[tokio::test]
async fn a_scripts_move_holds_the_path_until_it_arrives() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let k = map
        .query(move |m| {
            let k = spawn(m, ga, BRUTE, 20, 3.0);
            m.creature_mut(k).unwrap().motion = Motion::Idle { until: None };
            k
        })
        .await
        .unwrap();
    a.sees(k).await;
    let out = map
        .query(move |m| {
            on_route(m, k, vec![point(1, 20.0, 0, 0)]);
            let to = at(10.0, -5.0, 0.0);
            scripts::move_with(
                m,
                k,
                to,
                scripts::Move {
                    point: Some(1),
                    ..Default::default()
                },
            );
            let first = spline(m, k);
            m.now += Duration::from_millis(500);
            crate::game::map::world::update_creatures(m);
            let held = (
                spline(m, k) == first,
                destination(m, k).map(|d| near(d, to)),
            );
            m.now += Duration::from_secs(10);
            crate::game::map::world::update_creatures(m);
            let c = m.creature(k).unwrap();
            let arrived = (near(c.position, to), c.waypoint, c.last_waypoint);
            (first.is_some(), held, arrived)
        })
        .await
        .unwrap();
    let (moved, held, arrived) = out;
    assert!(moved, "it sets off");
    assert_eq!(held, (true, Some(true)), "on its way, its own walk");
    assert_eq!(arrived, (true, 0, None), "no point of its path reached");
}

/// A creature 5 yards ahead of `g` of 10,000 health and 100 mana, its
/// pool mana, that takes 20 more of shadow and twice as much.
fn burnt(m: &mut Map, g: Guid) -> Guid {
    let c = spawn(m, g, BRUTE, 60, 5.0);
    let f = m.unit_fields_mut(c).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
    f.set_u32(UNIT_FIELD_HEALTH, 10_000);
    f.set_u32(UNIT_FIELD_MAXPOWER1, 100);
    f.set_u32(UNIT_FIELD_POWER1, 100);
    f.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
    aura_of(m, c, 95_982, AURA_MOD_DAMAGE_TAKEN, SHADOW_MASK, 20);
    aura_of(
        m,
        c,
        95_983,
        AURA_MOD_DAMAGE_PERCENT_TAKEN,
        SHADOW_MASK,
        100,
    );
    c
}

/// Thistle Tea's `receive/4` "power burns use the original effect
/// coefficient for received bonuses" (mangos's `CalculateSpellDamage` of
/// `EffectPowerBurn`'s damage): a burn of 40 mana at half harms by 20, the
/// target's 20 more by the effect's coefficient of a half (10), then twice
/// over: 60, a burn at once and a burn's tick alike.
#[tokio::test]
async fn a_burns_harm_takes_the_targets_modifiers_by_its_own_coefficient() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = map
        .query(move |m| {
            let pools = |m: &Map, c: Guid| {
                let f = m.unit_fields(c).unwrap();
                (f.get_u32(UNIT_FIELD_HEALTH), f.get_u32(UNIT_FIELD_POWER1))
            };
            let c = burnt(m, ga);
            crate::game::map::spells::cast_triggered(m, ga, BURN, Some(c));
            let direct = pools(m, c);
            let c = burnt(m, ga);
            crate::game::map::spells::cast_triggered(m, ga, BURNING, Some(c));
            crate::game::tests_p3_spells_kit::tick_now(m, c, BURNING);
            (direct, pools(m, c))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (9_940, 60), "at once");
    assert_eq!(out.1, (9_940, 60), "a tick");
}
