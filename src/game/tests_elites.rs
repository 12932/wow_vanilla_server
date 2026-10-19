//! The Stormwind Elite Guard (`map/elites.rs`): its template and its
//! query's answer, its coming at the alarm's crossing of `elite_from` and
//! with Jaina, its going, its Charge at healers, and the warmaster's
//! Whirlwind and Enrage; on the guards tests' world (`tests_guards.rs`),
//! and on the real navmesh and data in the extended suite.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::ScriptTable;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::jaina::{self, Place};
use crate::game::map::battleground::av_scripts::{CHARGE, ENRAGE, WHIRLWIND};
use crate::game::map::combat::dist;
use crate::game::map::creature::MOVEMENT_RANDOM;
use crate::game::map::elites::{self, ELITE_GUARD};
use crate::game::map::guards::{self, Alarm};
use crate::game::map::{Map, MapKey, ai, auras, event_ai};
use crate::game::tests_guards::{
    ALLIANCE, BAKE, CITY_GUARD, CIVILIAN, GOING, ORCS, ROYAL_GUARD, STORMWIND, called, hardy,
    heard, in_town, place, raider, set_alarm, unit, world,
};
use crate::game::tests_p3_combat::{HELPER, START, faction, setup_with, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

/// A heal the tests' healers cast on themselves.
const HEAL: u32 = 990_700;

/// The spells the tests look at, as far as they do: Charge's run from 8
/// to 25 yards, Whirlwind's two seconds, Enrage, and a heal.
fn spells(d: &mut GameData) {
    d.spells.ranges.insert(95, (8.0, 25.0));
    for (index, ms) in [(39, 2_000), (4, 120_000)] {
        d.spells.durations.insert(
            index,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    let spell = |id: u32, name: &str, range_index: u32, duration_index: u32, e: SpellEffect| {
        let mut s = SpellTemplate {
            id,
            name: name.into(),
            range_index,
            duration_index,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = e;
        s
    };
    let on_self = |aura: u32| SpellEffect {
        effect: 6,
        aura,
        target_a: 1,
        ..Default::default()
    };
    for s in [
        spell(
            CHARGE,
            "Charge",
            95,
            0,
            SpellEffect {
                effect: 96,
                target_a: 6,
                ..Default::default()
            },
        ),
        spell(WHIRLWIND, "Whirlwind", 1, 39, on_self(4)),
        spell(ENRAGE, "Enrage", 1, 4, on_self(4)),
        spell(
            HEAL,
            "Heal",
            1,
            0,
            SpellEffect {
                effect: 10,
                base_points: 9,
                base_dice: 1,
                die_sides: 1,
                target_a: 1,
                ..Default::default()
            },
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
}

/// The guards tests' world with the Elite Guard put in after it, as the
/// loader puts it in: the Royal Guard there as he is where it matters (his
/// name, and his notice of 18 yards), a level-62 warrior creature's stats
/// (vmangos's), and its spells.
fn elite_world_with(f: impl FnOnce(&mut GameData)) -> GameData {
    let mut d = world();
    let royal = d.creature_templates.get_mut(&ROYAL_GUARD).unwrap();
    royal.name = "Stormwind Royal Guard".into();
    royal.detection_range = 18.0;
    d.class_level_stats.insert(
        (1, elites::LEVEL),
        ClassLevelStats {
            health: 3237,
            melee_damage: 125.504,
            attack_power: 262,
            armor: 3574,
            ..Default::default()
        },
    );
    spells(&mut d);
    // Its swords, as the data has them.
    for (entry, name, display) in [
        (elites::MAIN_HAND, "Monster - Sword, 1H Alliance PvP", 34128),
        (elites::OFF_HAND, "Grand Marshal's Swiftblade", 36066),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class: 2,
                subclass: 7,
                name: name.into(),
                display_id: display,
                inventory_type: 13,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    f(&mut d);
    elites::install(&mut d);
    d
}

fn elite_world() -> GameData {
    elite_world_with(|_| {})
}

/// [`elite_world`] with Stormwind's people, guards and Jaina of the
/// Alliance, the Horde's enemy, and the Alliance's players' friends.
fn alliance_world() -> GameData {
    elite_world_with(|d| {
        d.faction_templates.extend([
            (ALLIANCE, faction(ALLIANCE, 0x1, 0x4)),
            (ORCS, faction(ORCS, 0x4, 0x1)),
        ]);
        for e in [CIVILIAN, CITY_GUARD, ROYAL_GUARD, jaina::JAINA] {
            d.creature_templates.get_mut(&e).unwrap().faction = ALLIANCE;
        }
    })
}

/// An Elite Guard, as a script summons it, `yards` east of `at`.
fn summon_elite(m: &mut Map, summoner: Guid, at: Vector3d, yards: f32) -> Guid {
    let spot = Vector3d {
        x: at.x + yards,
        ..at
    };
    crate::game::map::scripts::summon(m, summoner, ELITE_GUARD, spot, std::f32::consts::PI, None)
        .expect("an Elite Guard")
}

/// The index of the Elite Guard's event whose first step casts `spell`.
fn event_of(m: &Map, spell: u32) -> usize {
    let data = &m.realm.data.scripts;
    data.ai_events[&ELITE_GUARD]
        .iter()
        .position(|e| {
            data.ai_scripts
                .get(&e.actions[0])
                .and_then(|s| s.first())
                .is_some_and(|s| s.command == 15 && s.datalong[0] == spell)
        })
        .expect("its event")
}

/// `g`'s event `i` due now, and its events looked at.
fn fire(m: &mut Map, g: Guid, i: usize) {
    let now = m.now;
    let s = event_ai::scripted_mut(m, g).unwrap();
    s.timers[i] = Some(now);
    s.disabled[i] = false;
    s.next_tick = None;
    event_ai::tick(m, g);
}

fn spell_go(m: &ServerOpcodeMessage, spell: u32, caster: Guid) -> bool {
    matches!(m, ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == spell && s.caster == caster)
}

/// The Elite Guard is a creature template of its own, made in code: the
/// Royal Guard's looks, side and weapons at level 62, elite, with the
/// Tower Point Warmaster's multipliers, swing and class; and a client's
/// query of it is answered with its name.
#[tokio::test]
async fn the_elite_guard_is_a_template_of_its_own_and_its_name_is_answered() {
    let d = elite_world();
    let t = d
        .creature_template(ELITE_GUARD)
        .cloned()
        .expect("its template");
    let royal = d.creature_template(ROYAL_GUARD).cloned().unwrap();
    assert_eq!(t.name, "Stormwind Elite Guard");
    assert_eq!((t.level_min, t.level_max, t.rank), (62, 62, 1));
    assert_eq!(
        (
            t.health_multiplier,
            t.damage_multiplier,
            t.base_attack_time,
            t.unit_class
        ),
        (20.0, 13.5, 2000, 1),
        "the warmaster's"
    );
    assert_eq!(
        (t.display_ids, t.faction, t.flags_extra),
        (royal.display_ids, royal.faction, royal.flags_extra),
        "the Royal Guard's"
    );
    assert_eq!(t.health_max, 3237 * 20);
    assert_eq!(t.spell_list_id, 0, "the warmaster's fight, not his spells");
    assert_eq!(d.scripts.ai_events[&ELITE_GUARD].len(), 6);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let (e, level, health) = on(&map, move |m| {
        hardy(m, ga);
        let at = m.players[&ga].position();
        let e = summon_elite(m, ga, at, 30.0);
        let f = &m.creature(e).unwrap().fields;
        (
            e,
            f.get_u32(UNIT_FIELD_LEVEL),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert_eq!((level, health), (62, 64_740));
    a.sees(e).await;
    a.send(CMSG_CREATURE_QUERY {
        creature: guid::entry(e),
        guid: e,
    })
    .await;
    let found = a
        .wait_for("its query's answer", |m| match m {
            ServerOpcodeMessage::SMSG_CREATURE_QUERY_RESPONSE(r)
                if r.creature_entry == ELITE_GUARD =>
            {
                Some(r.found.clone())
            }
            _ => None,
        })
        .await
        .expect("it is known");
    assert_eq!(found.name1, "Stormwind Elite Guard");
    assert_eq!(found.creature_rank, 1, "elite");
    assert_eq!(found.display_id, royal.display_ids[0]);
}

/// A packed guid at the start of `b`, and what follows it.
fn packed_guid(b: &[u8]) -> Option<(Guid, &[u8])> {
    let (&mask, mut rest) = b.split_first()?;
    let mut v = 0u64;
    for bit in 0..8 {
        if mask & (1 << bit) != 0 {
            let (&byte, more) = rest.split_first()?;
            v |= u64::from(byte) << (bit * 8);
            rest = more;
        }
    }
    Some((Guid::new(v), rest))
}

/// The user's: a one-handed sword in each hand, clean ones (the Alliance's
/// PvP blade, and the Grand Marshal's Swiftblade), its own equipment and
/// not the Royal Guard's; and it swings both, as a creature with a weapon
/// in its off hand does, the off hand's blows marked as such.
#[tokio::test]
async fn the_elite_guard_wields_a_sword_in_each_hand_and_swings_both() {
    let d = elite_world();
    let t = d.creature_template(ELITE_GUARD).unwrap();
    assert_ne!(
        t.equipment_id,
        d.creature_template(ROYAL_GUARD).unwrap().equipment_id
    );
    assert_eq!(
        d.creature_equipment.get(&t.equipment_id),
        Some(&[elites::MAIN_HAND, elites::OFF_HAND, 0])
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let (e, displays, dual) = on(&map, move |m| {
        hardy(m, ga);
        let at = m.players[&ga].position();
        let e = summon_elite(m, ga, at, 3.0);
        let f = &m.creature(e).unwrap().fields;
        let displays: Vec<u32> = (0..3)
            .map(|i| f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i))
            .collect();
        ai::attacked(m, e, ga);
        (e, displays, crate::game::map::combat::dual_wields(m, e))
    })
    .await;
    assert_eq!(displays, [34128, 36066, 0], "a sword in each hand, no bow");
    assert!(dual, "it wields both");
    a.wait_frame(
        "a blow of its off hand",
        crate::game::map::combat::SMSG_ATTACKERSTATEUPDATE,
        |b| {
            let left = u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                & crate::game::map::combat::HITINFO_LEFTSWING
                != 0;
            left && packed_guid(&b[4..]).is_some_and(|(g, _)| g == e)
        },
    )
    .await;
    a.wait_frame(
        "and of its main hand",
        crate::game::map::combat::SMSG_ATTACKERSTATEUPDATE,
        |b| {
            let left = u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                & crate::game::map::combat::HITINFO_LEFTSWING
                != 0;
            !left && packed_guid(&b[4..]).is_some_and(|(g, _)| g == e)
        },
    )
    .await;
}

/// As Stormwind's alarm rises through 50, two Elite Guards come near the
/// civilian whose call took it there, within 30 yards, against its
/// attacker: once for the crossing, not at the calls after it; cooled by a
/// half below 50, the next rise brings two more. They aren't counted in
/// the alarm, and go and wander as guards do.
#[tokio::test]
async fn two_elite_guards_come_as_the_alarm_rises_through_fifty_not_at_each_call() {
    let (_realm, mut a, ga, map) = setup_with(elite_world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let seen = on(&map, move |m| {
        hardy(m, ga);
        in_town(m, civ, STORMWIND);
        // Two Royal Guards a call past 40: 47, then 49.
        set_alarm(m, STORMWIND, 47);
        ai::attacked(m, civ, ga);
        let below = called(m, ELITE_GUARD).len();
        // 49 to 51: through 50.
        let helper = spawn(m, ga, HELPER, 20, 6.0);
        ai::taunted(m, civ, helper);
        let came = called(m, ELITE_GUARD);
        let at = m.creature(civ).unwrap().position;
        let near: Vec<f32> = came
            .iter()
            .map(|e| dist(m.creature(*e).unwrap().position, at))
            .collect();
        let victims: Vec<Guid> = came
            .iter()
            .map(|e| m.creature(*e).unwrap().combat.victim)
            .collect();
        let rules: Vec<(i32, Option<Duration>, u8, f32)> = came
            .iter()
            .map(|e| {
                let c = m.creature(*e).unwrap();
                let s = c.scripted.as_deref().unwrap();
                (
                    s.despawn_type,
                    s.despawn_after,
                    c.movement_type,
                    c.wander_distance,
                )
            })
            .collect();
        let alarm = guards::alarm(m, STORMWIND);
        // The calls after: 53, 55.
        ai::taunted(m, civ, ga);
        ai::taunted(m, civ, helper);
        let later = called(m, ELITE_GUARD).len();
        // Cooled by a half since its last call, to 49: a call takes it
        // through 50 again.
        let then = m.now.checked_sub(Duration::from_secs(300)).unwrap();
        m.guards.alarms.insert(
            STORMWIND,
            Alarm {
                guards: 98,
                last_call: then,
            },
        );
        ai::taunted(m, civ, ga);
        let again = called(m, ELITE_GUARD).len();
        (
            below, came, near, victims, rules, alarm, later, again, helper,
        )
    })
    .await;
    let (below, came, near, victims, rules, alarm, later, again, helper) = seen;
    assert_eq!(below, 0, "none below 50");
    assert_eq!(came.len(), 2, "two at the crossing");
    assert!(near.iter().all(|d| *d <= guards::SUMMON_RADIUS), "{near:?}");
    assert_eq!(victims, [helper, helper], "against the civilian's attacker");
    assert!(
        rules
            .iter()
            .all(|r| *r == (2, Some(Duration::from_secs(300)), MOVEMENT_RANDOM, 10.0)),
        "the guards' rules: {rules:?}"
    );
    assert_eq!(alarm, 51, "the alarm counts the guards called, not them");
    assert_eq!(later, 2, "none at the calls after");
    assert_eq!(again, 4, "two more at the next crossing");
}

/// The Elite Guards who come with Jaina: where they came, and whom they
/// fight.
fn elites_with_her(m: &Map) -> (Guid, Vec<Guid>) {
    let Place::Away { copy, .. } = place(m) else {
        panic!("she is away");
    };
    (copy, m.guards.jaina.elites.clone())
}

/// Brings Jaina to Stormwind on a raider's call, the raider dead before
/// she comes, and the Elite Guards with her.
async fn jaina_comes(
    realm: &std::sync::Arc<crate::game::Realm>,
    map: &crate::game::map::MapHandle,
) -> (Guid, Vec<Guid>) {
    let civ = unit(CIVILIAN);
    let (_foe_client, foe) = raider(realm, "Foe", (START.0 - 3.0, START.1, START.2)).await;
    on(map, move |m| {
        in_town(m, civ, STORMWIND);
        // Two Royal Guards from 58: 60, and she is called.
        set_alarm(m, STORMWIND, 58);
        ai::attacked(m, civ, foe);
        crate::game::map::death::kill(m, foe);
    })
    .await;
    until(map, "she comes", |m| matches!(place(m), Place::Away { .. })).await;
    on(map, |m| elites_with_her(m)).await
}

/// Jaina in Stormwind: four Elite Guards come about 40 yards around her,
/// facing her, walk to her side, and take on a Horde raider they meet on
/// the way; she goes home, and they go with her, with her flash.
#[tokio::test]
async fn four_elite_guards_come_with_jaina_walk_to_her_fighting_the_horde_on_their_way_and_leave_with_her()
 {
    let (realm, mut a, ga, map) = setup_with(alliance_world(), Class::Warrior).await;
    a.sees(unit(CIVILIAN)).await;
    on(&map, move |m| hardy(m, ga)).await;
    // A raider of level 60 waits out of their sight (they notice him at
    // 20 yards).
    let (grunt_client, grunt) = raider(&realm, "Grunt", (START.0 + 90.0, START.1, START.2)).await;
    on(&map, move |m| {
        let f = &mut m.players.get_mut(&grunt).unwrap().fields;
        f.set_u32(UNIT_FIELD_LEVEL, 60);
    })
    .await;
    let (copy, elites) = jaina_comes(&realm, &map).await;
    assert_eq!(elites.len(), 4, "four with her");
    let came = on(&map, move |m| {
        let at = m.creature(copy).unwrap().position;
        let mut out = Vec::new();
        for e in called(m, ELITE_GUARD) {
            let c = m.creature(e).unwrap();
            let to_her = (at.y - c.home.y).atan2(at.x - c.home.x);
            out.push((e, dist(c.home, at), (c.home_orientation - to_her).abs()));
        }
        out
    })
    .await;
    assert_eq!(
        came.iter().map(|(e, _, _)| *e).collect::<Vec<_>>(),
        elites,
        "no others"
    );
    for (_, yards, facing) in &came {
        assert!((30.0..=50.0).contains(yards), "{came:?}");
        assert!(*facing < 0.01, "facing her: {came:?}");
    }
    // The raider steps into the first one's way, 12 yards from her.
    let first = elites[0];
    let spot = on(&map, move |m| {
        let at = m.creature(copy).unwrap().position;
        let p = m.creature(first).unwrap().position;
        let f = 12.0 / dist(p, at);
        Vector3d {
            x: at.x + (p.x - at.x) * f,
            y: at.y + (p.y - at.y) * f,
            z: at.z,
        }
    })
    .await;
    grunt_client.heartbeat(spot.x, spot.y, spot.z).await;
    until(&map, "an Elite Guard takes him on, on its way", move |m| {
        let at = m.creature(copy).unwrap().position;
        m.creature(first).is_some_and(|c| {
            c.combat.attacking
                && c.combat.victim == grunt
                && dist(c.position, at) > elites::BY_JAINA + 2.0
        })
    })
    .await;
    // He falls; they close on her and keep by her.
    on(&map, move |m| crate::game::map::death::kill(m, grunt)).await;
    let four = elites.clone();
    let by_her = move |m: &Map| {
        let at = m.creature(copy).unwrap().position;
        four.iter().all(|e| {
            m.creature(*e).is_some_and(|c| {
                !c.combat.attacking && !c.evading && dist(c.position, at) <= elites::BY_JAINA + 4.0
            })
        })
    };
    until(&map, "the four by her side", by_her).await;
    // At peace a minute, she says farewell; they flash with her and go.
    a.clear();
    on(&map, move |m| {
        m.guards.jaina.calm_since = m.now.checked_sub(Duration::from_secs(61));
    })
    .await;
    a.wait_for("her farewell", |m| heard(m, GOING).then_some(()))
        .await;
    for e in elites.iter().copied() {
        a.wait_for("its flash", move |m| {
            spell_go(m, jaina::TELEPORT_VISUAL, e).then_some(())
        })
        .await;
    }
    for e in elites.iter().copied() {
        a.forgets(e).await;
    }
    until(&map, "gone with her", |m| {
        called(m, ELITE_GUARD).is_empty() && m.guards.jaina.elites.is_empty()
    })
    .await;
}

/// Jaina dead in Stormwind: her Elite Guards stay as guards do, wandering
/// about where they stand, to go five minutes out of combat or at death.
#[tokio::test]
async fn jaina_dead_her_elite_guards_stay_as_guards() {
    let (realm, mut a, ga, map) = setup_with(alliance_world(), Class::Warrior).await;
    a.sees(unit(CIVILIAN)).await;
    on(&map, move |m| hardy(m, ga)).await;
    let (copy, elites) = jaina_comes(&realm, &map).await;
    assert_eq!(elites.len(), 4);
    let after = on(&map, move |m| {
        ai::die(m, copy, ga);
        let left: Vec<_> = elites
            .iter()
            .map(|e| {
                let c = m.creature(*e).unwrap();
                let s = c.scripted.as_deref().unwrap();
                (
                    s.follow.is_some(),
                    m.script_groups.contains_key(e),
                    c.movement_type,
                    s.despawn_type,
                )
            })
            .collect();
        (left, m.guards.jaina.elites.len())
    })
    .await;
    let (left, listed) = after;
    assert_eq!(listed, 0);
    assert!(
        left.iter()
            .all(|l| *l == (false, false, MOVEMENT_RANDOM, guards::TIMED_OOC_OR_DEAD)),
        "{left:?}"
    );
}

/// The user's twist on the warmaster's Charge: an Elite Guard charges a
/// healer (a priest or paladin here) in sight and between 8 and 25 yards
/// over a nearer warrior and over its victim, one who healed in the last
/// five seconds first; with no healer in sight or reach, its victim in
/// reach, as the warmaster does; with none, no one. Its cast of a heal
/// marks a healer. After its charge it fights the one it charged, though
/// another has more threat.
#[tokio::test]
async fn an_elite_guards_charge_goes_to_a_healer_in_sight_over_a_nearer_warrior_else_its_victim() {
    let (realm, a, ga, map) = setup_with(elite_world(), Class::Warrior).await;
    // Alice faces east; the Elite Guard will stand 10 yards east of her.
    a.heartbeat(START.0, START.1, START.2).await;
    let e_at = Vector3d {
        x: START.0 + 10.0,
        y: START.1,
        z: START.2,
    };
    let mut players = Vec::new();
    for (name, class, dx, dy) in [
        ("Bob", Class::Priest, 0.0, 20.0),
        ("Dana", Class::Paladin, 0.0, -15.0),
        ("Carl", Class::Warrior, 9.0, 0.0),
        ("Eve", Class::Priest, 0.0, 30.0),
    ] {
        let mut c = Client::connect(&realm, name);
        let g = c.enter(name, Race::Human, class).await;
        c.heartbeat(e_at.x + dx, e_at.y + dy, e_at.z).await;
        players.push((
            c,
            g,
            Vector3d {
                x: e_at.x + dx,
                y: e_at.y + dy,
                z: e_at.z,
            },
        ));
    }
    let placed: Vec<(Guid, Vector3d)> = players.iter().map(|(_, g, p)| (*g, *p)).collect();
    let [bob, dana, carl, eve] = [placed[0].0, placed[1].0, placed[2].0, placed[3].0];
    until(&map, "everyone in place", move |m| {
        placed.iter().all(|(g, p)| {
            m.players
                .get(g)
                .is_some_and(|x| dist(x.position(), *p) < 0.1)
        })
    })
    .await;
    let picks = on(&map, move |m| {
        for g in [ga, bob, dana, carl, eve] {
            hardy(m, g);
        }
        // Bob's heal marks him.
        crate::game::map::spells::cast(
            m,
            bob,
            HEAL,
            crate::game::map::spells::Targets {
                unit: Some(bob),
                ..Default::default()
            },
            None,
        );
        let marked = m.players[&bob].misc.healed_at == Some(m.now);
        m.players.get_mut(&bob).unwrap().misc.healed_at = None;
        let e = summon_elite(m, ga, START_V, 10.0);
        // Its victim Alice, with the most threat.
        ai::attacked(m, e, ga);
        ai::add_threat(m, e, ga, 1000.0);
        let picks = |m: &Map| -> Vec<Guid> {
            (0..60)
                .filter_map(|_| elites::charge_target(m, e))
                .collect()
        };
        let neither_healed = picks(m);
        let now = m.now;
        m.players.get_mut(&dana).unwrap().misc.healed_at = Some(now);
        let dana_healed = picks(m);
        m.players.get_mut(&dana).unwrap().misc.healed_at = now.checked_sub(Duration::from_secs(6));
        let healed_long_ago = picks(m);
        // Walls hide Bob and Dana; Eve, in sight, is out of reach.
        let wall = |dy: f32| {
            (
                Vector3d {
                    x: e_at.x - 3.0,
                    y: e_at.y + dy,
                    ..e_at
                },
                Vector3d {
                    x: e_at.x + 3.0,
                    y: e_at.y + dy,
                    ..e_at
                },
            )
        };
        m.spells.walls.extend([wall(10.0), wall(-7.0)]);
        let hidden = picks(m);
        // Its victim out of reach too (Eve, say): no one.
        m.creature_mut(e).unwrap().combat.victim = eve;
        let nobody = elites::charge_target(m, e);
        m.creature_mut(e).unwrap().combat.victim = ga;
        m.spells.walls.clear();
        // Its Charge at Dana, who healed now.
        m.players.get_mut(&dana).unwrap().misc.healed_at = Some(now);
        let i = event_of(m, CHARGE);
        fire(m, e, i);
        (
            e,
            marked,
            neither_healed,
            dana_healed,
            healed_long_ago,
            hidden,
            nobody,
        )
    })
    .await;
    let (e, marked, neither_healed, dana_healed, healed_long_ago, hidden, nobody) = picks;
    assert!(marked, "a heal's cast marks its healer");
    for (picks, what) in [
        (&neither_healed, "neither healed"),
        (&healed_long_ago, "healed six seconds ago"),
    ] {
        assert_eq!(picks.len(), 60, "{what}");
        assert!(
            picks.iter().all(|g| [bob, dana].contains(g)),
            "{what}: healers in sight and reach only"
        );
        assert!(
            picks.contains(&bob) && picks.contains(&dana),
            "{what}: either, at random"
        );
    }
    assert!(
        dana_healed.iter().all(|g| *g == dana) && dana_healed.len() == 60,
        "the one who healed lately"
    );
    assert!(
        hidden.iter().all(|g| *g == ga) && hidden.len() == 60,
        "no healer in sight or reach: its victim"
    );
    assert_eq!(nobody, None, "none in reach");
    // It charges Dana, and fights her though Alice has more threat.
    until(&map, "it charges her and fights her", move |m| {
        m.creature(e).is_some_and(|c| {
            c.engagement.charge.is_none() && c.combat.attacking && c.combat.victim == dana
        }) && m
            .players
            .get(&dana)
            .is_some_and(|p| dist(p.position(), m.creature(e).unwrap().position) < 5.0)
    })
    .await;
    let start = on(&map, |m| m.stats.updates).await;
    until(&map, "twenty updates on", move |m| {
        m.stats.updates > start + 20
    })
    .await;
    let still = on(&map, move |m| m.creature(e).unwrap().combat.victim).await;
    assert_eq!(still, dana, "it keeps to the one it charged");
    drop(players);
}

/// Alice's place.
const START_V: Vector3d = Vector3d {
    x: START.0,
    y: START.1,
    z: START.2,
};

/// The warmaster's Whirlwind twice in a row, two seconds apart (the second
/// a script timed by its delay, not at once with the first); and Enrage
/// below 30%, once a fight.
#[tokio::test]
async fn the_elite_guard_whirlwinds_twice_two_seconds_apart_and_enrages_below_thirty_percent() {
    let (_realm, mut a, ga, map) = setup_with(elite_world(), Class::Warrior).await;
    let e = on(&map, move |m| {
        hardy(m, ga);
        let at = m.players[&ga].position();
        let e = summon_elite(m, ga, at, 3.0);
        ai::attacked(m, e, ga);
        e
    })
    .await;
    a.sees(e).await;
    a.clear();
    let pending = on(&map, move |m| {
        let i = event_of(m, WHIRLWIND);
        fire(m, e, i);
        let data = m.realm.data.clone();
        m.scripts
            .runs
            .iter()
            .filter(|r| r.source == e && r.table == ScriptTable::Generic && r.next == 0)
            .filter_map(|r| data.scripts.generic.get(&r.id))
            .map(|steps| (steps[0].delay_ms, steps[0].datalong[0]))
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(pending, [(2000, WHIRLWIND)], "the second waits two seconds");
    a.wait_for("the first", |m| spell_go(m, WHIRLWIND, e).then_some(()))
        .await;
    let first = tokio::time::Instant::now();
    a.wait_for("the second", |m| spell_go(m, WHIRLWIND, e).then_some(()))
        .await;
    let apart = first.elapsed();
    assert!(apart >= Duration::from_millis(1800), "{apart:?} apart");
    let enraged = on(&map, move |m| {
        let max = m.creature(e).unwrap().fields.get_u32(UNIT_FIELD_MAXHEALTH);
        let mut seen = Vec::new();
        for (pct, strip) in [(40, false), (25, true), (20, false)] {
            m.creature_mut(e)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, max * pct / 100);
            event_ai::scripted_mut(m, e).unwrap().next_tick = None;
            event_ai::tick(m, e);
            let has = auras::auras(m, e).is_some_and(|a| a.has_spell(ENRAGE));
            seen.push((pct, has));
            if strip {
                auras::remove_spell(m, e, ENRAGE, None);
            }
        }
        seen
    })
    .await;
    assert_eq!(
        enraged,
        [(40, false), (25, true), (20, false)],
        "below 30%, once a fight"
    );
}

// The real world, in the extended suite.

/// The real data: the Elite Guard is there, made of the Royal Guard and
/// the Tower Point Warmaster as the database has them, its spells are, and
/// no spawn of the database is of its entry.
#[test]
#[ignore = "extended"]
fn the_real_data_has_the_elite_guard() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let t = d.creature_template(ELITE_GUARD).expect("the Elite Guard");
    assert_eq!(t.name, "Stormwind Elite Guard");
    assert_eq!(t.display_ids, [3167, 0, 0, 0], "the Royal Guard's look");
    assert_eq!(t.faction, 11, "his side");
    // Its swords: the Alliance's PvP blade and the Grand Marshal's
    // Swiftblade, one-handed swords of the client's.
    assert_eq!(
        d.creature_equipment.get(&t.equipment_id),
        Some(&[elites::MAIN_HAND, elites::OFF_HAND, 0])
    );
    for (entry, name, display) in [
        (21573, "Monster - Sword, 1H Alliance PvP", 34128),
        (23456, "Grand Marshal's Swiftblade", 36066),
    ] {
        let i = d.item(entry).unwrap_or_else(|| panic!("{entry}"));
        assert_eq!(
            (
                i.name.as_str(),
                i.display_id,
                i.class,
                i.subclass,
                i.inventory_type
            ),
            (name, display, 2, 7, 13),
            "a one-handed sword"
        );
    }
    assert_eq!(
        (t.level_min, t.level_max, t.rank, t.unit_class),
        (62, 62, 1, 1)
    );
    assert_eq!(
        (t.health_multiplier, t.damage_multiplier, t.base_attack_time),
        (20.0, 13.5, 2000)
    );
    assert_eq!(t.health_max, 3237 * 20, "level 62's warrior, twentyfold");
    assert_eq!(t.spell_list_id, 0);
    let events = &d.scripts.ai_events[&ELITE_GUARD];
    assert_eq!(events.len(), 6);
    assert!(events.iter().all(|e| e.id > 92_000_000));
    for s in [CHARGE, 20684, 23511, WHIRLWIND, ENRAGE] {
        assert!(d.spells.get(s).is_some(), "{s}");
    }
    assert_eq!(d.spells.range(d.spells.get(CHARGE).unwrap()), (8.0, 25.0));
    assert!(
        d.creature_spawns
            .values()
            .flatten()
            .all(|s| !s.entries.contains(&ELITE_GUARD)),
        "no spawn of its entry"
    );
}

/// On the real navmesh, in Stormwind's Trade District: Jaina's four Elite
/// Guards come at points about 40 yards around her that a walk from her
/// reaches, and walk to her.
#[tokio::test]
#[ignore = "extended"]
async fn on_the_navmesh_jainas_elite_guards_come_at_reachable_points_and_walk_to_her() {
    let bake = BAKE.as_path();
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    // Before Stormwind's auction house.
    const TRADE: (f32, f32, f32) = (-8816.0, 660.0, 96.0);
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(BAKE.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, alliance_world(), None);
    let map = realm.maps.get(&realm, MapKey::world(0));
    // Alice watches: a creature no one sees doesn't move.
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.say(&format!(".go {} {} {}", TRADE.0 + 10.0, TRADE.1, TRADE.2))
        .await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let z = m
            .nav
            .as_mut()
            .and_then(|n| n.ground(TRADE.0, TRADE.1, TRADE.2))
            .expect("Stormwind's ground");
        let at = Vector3d {
            x: TRADE.0,
            y: TRADE.1,
            z,
        };
        let t = m
            .realm
            .data
            .creature_template(jaina::JAINA)
            .cloned()
            .unwrap();
        let counter = m.next_creature_counter;
        m.next_creature_counter += 1;
        let copy = m.add_creature(
            crate::game::map::creature::Creature::new(
                guid::unit(jaina::JAINA, counter | 0x80_0000),
                &t,
                20,
                at,
                0.0,
            )
            .with_base(),
        );
        m.realm.social.jaina.lock().place = Place::Away { map: m.key, copy };
        elites::with_jaina(m, copy);
        let elites = m.guards.jaina.elites.clone();
        let mut came = Vec::new();
        for e in &elites {
            let home = m.creature(*e).unwrap().home;
            let reached = m
                .nav
                .as_mut()
                .and_then(|n| n.path(at, home))
                .and_then(|p| p.last().copied())
                .is_some_and(|end| dist(end, home) <= 1.0);
            let flat = ((home.x - at.x).powi(2) + (home.y - at.y).powi(2)).sqrt();
            came.push((flat, (home.z - at.z).abs(), reached));
        }
        (copy, elites, came)
    })
    .await;
    let (copy, elites, came) = out;
    assert_eq!(elites.len(), 4, "{came:?}");
    for (flat, rise, reached) in &came {
        assert!((39.0..=41.0).contains(flat), "{came:?}");
        assert!(*rise < 15.0, "on her floor: {came:?}");
        assert!(reached, "a walk from her reaches it: {came:?}");
    }
    for e in &elites {
        a.sees(*e).await;
    }
    until(&map, "they walk to her", move |m| {
        let at = m.creature(copy).unwrap().position;
        elites.iter().all(|e| {
            m.creature(*e)
                .is_some_and(|c| dist(c.position, at) <= elites::BY_JAINA + 4.0)
        })
    })
    .await;
    on(&map, |m| m.realm.social.jaina.lock().place = Place::Home).await;
}
