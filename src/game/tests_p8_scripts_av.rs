//! Round 8's Alterac Valley leaders, lords and war riders, as Thistle
//! Tea scripts them (`AlteracValley`, `AlteracValleyAssault`,
//! `AlteracValleyWarRider`), through the protocol: Drek'Thar raising his
//! wolves, Balinda's ground and her spells for the close, a warmaster's
//! Charge and double Whirlwind, the war riders' thirty yards, Lokholar
//! feeding on players and the lords' kits, and a ritual only its waiting
//! summoner takes up.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::{casts_by, due, hurry_runs};
use super::tests_pvp2::{arrive, creature, npc, turn_in, valley};
use crate::game::data::scripts::{EVENT_TIMER_IN_COMBAT, ScriptStep};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_scripts;
use crate::game::map::{Map, MapHandle, ai, event_ai};

pub(super) const AV: u32 = 30;

pub(super) fn spawn_at(d: &mut GameData, guid: u32, entry: u32, at: (f32, f32, f32)) {
    d.creature_spawns
        .entry(AV)
        .or_default()
        .push(CreatureSpawn {
            guid,
            entries: vec![entry],
            map: AV,
            x: at.0,
            y: at.1,
            z: at.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        });
}

/// Instant spells of no effect, cast within fifty yards: at an enemy,
/// or (`own`) on the caster.
pub(super) fn instants(d: &mut GameData, at: &[u32], own: &[u32]) {
    d.spells.ranges.insert(1, (0.0, 50.0));
    for (&id, target_a) in at.iter().map(|i| (i, 6)).chain(own.iter().map(|i| (i, 1))) {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 3,
            target_a,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
}

/// Move `c` within the match's map, at once, to a height.
pub(super) async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
        .await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)
        )
        .then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
    // In before the test reads where they are.
    for _ in 0..200 {
        let here = map.query(move |m| m.players[&g].position()).await.unwrap();
        if (here.x - x).abs() < 0.01 && (here.y - y).abs() < 0.01 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

pub(super) async fn pair(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, Client, Guid, MapHandle) {
    let mut a = Client::connect(realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = super::tests_pvp::both_into(
        realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::AlteracValley,
    )
    .await;
    (a, ga, b, gb, map)
}

/// The living creatures of `entry`.
pub(super) fn all_of(m: &Map, entry: u32) -> Vec<Guid> {
    let mut out: Vec<Guid> = m
        .creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
        .collect();
    out.sort_by_key(|g| g.guid());
    out
}

/// The events of `entry` whose actions do `matches`: their ids.
pub(super) fn events_where(m: &Map, entry: u32, matches: impl Fn(&ScriptStep) -> bool) -> Vec<u32> {
    let s = &m.realm.data.scripts;
    s.ai_events
        .get(&entry)
        .into_iter()
        .flatten()
        .filter(|e| {
            e.actions
                .iter()
                .filter_map(|a| s.ai_scripts.get(a))
                .flatten()
                .any(&matches)
        })
        .map(|e| e.id)
        .collect()
}

/// The event of `entry` that casts `spell`.
pub(super) fn casting(m: &Map, entry: u32, spell: u32) -> u32 {
    let found = events_where(m, entry, |s| s.command == 15 && s.datalong[0] == spell);
    *found
        .first()
        .unwrap_or_else(|| panic!("{entry} casts {spell}"))
}

/// `g`'s event `id` due now with no global cooldown in the way.
fn due_now(m: &mut Map, g: Guid, id: u32) {
    no_gcd(m, g);
    due(m, g, id);
}

/// `g` at `at`, where it stays.
fn hold_at(m: &mut Map, g: Guid, at: Vector3d) {
    if let Some(c) = m.creature_mut(g) {
        c.position = at;
        c.motion = crate::game::map::creature::Motion::Idle { until: None };
    }
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.no_combat_movement = true;
    }
}

pub(super) fn spells_of(casts: &[(u32, Guid)]) -> Vec<u32> {
    casts.iter().map(|(s, _)| *s).collect()
}

const DREK_THAR: u32 = 11946;
const DREK_HOME: (f32, f32, f32) = (-1370.9, -219.8, 98.5);
const WOLVES: [u32; 2] = [12121, 12122];

/// Drek'Thar, done with a fight, raises the wolves fallen about him; while
/// he fights they lie where they fell (Thistle Tea's evade event, a
/// `respawn_creature` for each within a hundred yards).
#[tokio::test]
async fn drek_thar_raises_his_fallen_wolves_when_he_resets() {
    let mut d = valley();
    npc(&mut d, 12121, "Frostwolf Wolf", 35);
    npc(&mut d, 12122, "Frostwolf Bloodhound", 35);
    if let Some(s) = d
        .creature_spawns
        .get_mut(&AV)
        .and_then(|s| s.iter_mut().find(|s| s.entries == [DREK_THAR]))
    {
        (s.x, s.y, s.z) = DREK_HOME;
    }
    spawn_at(
        &mut d,
        170401,
        12121,
        (DREK_HOME.0 + 5.0, DREK_HOME.1, DREK_HOME.2),
    );
    spawn_at(
        &mut d,
        170402,
        12122,
        (DREK_HOME.0 + 5.0, DREK_HOME.1 + 4.0, DREK_HOME.2),
    );
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, DREK_HOME.0 + 10.0, DREK_HOME.1, DREK_HOME.2).await;
    let drek = map
        .query(|m| creature(m, DREK_THAR))
        .await
        .flatten()
        .unwrap();
    map.query(move |m| {
        ai::attacked(m, drek, grom);
        av_scripts::sweep_soon(m);
    })
    .await;
    harness::an_update(&map).await;
    // His wolves fall while he fights: they stay down.
    map.query(move |m| {
        let wolves: Vec<Guid> = WOLVES.iter().flat_map(|w| all_of(m, *w)).collect();
        for w in wolves {
            crate::game::map::combat::deal_damage(m, grom, w, 1_000_000);
        }
        av_scripts::sweep_soon(m);
    })
    .await;
    harness::an_update(&map).await;
    let (fighting, down) = map
        .query(move |m| {
            (
                m.creature(drek).unwrap().combat.attacking,
                WOLVES.map(|w| all_of(m, w).len()),
            )
        })
        .await
        .unwrap();
    assert!(fighting, "he fights on");
    assert_eq!(down, [0, 0], "his wolves lie where they fell");
    // He gives up the fight: they rise.
    map.query(move |m| {
        ai::enter_evade(m, drek);
        av_scripts::sweep_soon(m);
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let up = map
            .query(|m| WOLVES.map(|w| all_of(m, w).len()))
            .await
            .unwrap();
        if up == [1, 1] {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "he raises his wolves: {up:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

const BALINDA: u32 = 11949;
const BALINDA_HOME: (f32, f32, f32) = (-57.7, -286.6, 15.6);

/// Captain Balinda Stonehearth with Grom fifteen yards north of her home,
/// fighting him; Alice is elsewhere.
async fn balinda() -> (Client, Client, Guid, Guid, MapHandle) {
    let mut d = valley();
    npc(&mut d, BALINDA, "Captain Balinda Stonehearth", 35);
    npc(&mut d, 99_801, "Second", 35);
    spawn_at(&mut d, 170501, BALINDA, BALINDA_HOME);
    instants(&mut d, &[20420, 22746, 19712, 15534], &[]);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (a, _, mut b, grom, map) = pair(&realm).await;
    put(
        &map,
        &mut b,
        BALINDA_HOME.0,
        BALINDA_HOME.1 + 15.0,
        BALINDA_HOME.2,
    )
    .await;
    let her = map.query(|m| creature(m, BALINDA)).await.flatten().unwrap();
    b.sees_on(&map, her).await;
    map.query(move |m| ai::attacked(m, her, grom)).await;
    (a, b, grom, her, map)
}

/// Balinda holds her ground and casts while her victim is five to
/// twenty-five yards off, and closes on them otherwise, too near or too
/// far (Thistle Tea's casting spot, line of sight with it: this server's
/// sweep asks [`crate::game::map::conditions::in_sight`], the
/// `line_of_sight` condition's own test).
#[tokio::test]
async fn balinda_holds_her_ground_while_her_victim_is_at_casting_range() {
    let (_a, _b, grom, her, map) = balinda().await;
    let at = |dy: f32| Vector3d {
        x: BALINDA_HOME.0,
        y: BALINDA_HOME.1 + dy,
        z: BALINDA_HOME.2,
    };
    let mut held = Vec::new();
    for dy in [0.0, -15.0, 12.0, 0.0] {
        // Grom stands fifteen yards north of her home: she at it, fifteen
        // yards south (thirty from him), twelve north (three), and home
        // again.
        let h = map
            .query(move |m| {
                if let Some(c) = m.creature_mut(her) {
                    c.position = at(dy);
                }
                av_scripts::sweep_soon(m);
                av_scripts::update(m);
                (
                    event_ai::scripted(m, her).is_some_and(|s| s.no_combat_movement),
                    m.creature(her).unwrap().combat.victim == grom,
                )
            })
            .await
            .unwrap();
        held.push(h);
    }
    assert_eq!(
        held,
        vec![(true, true), (false, true), (false, true), (true, true)],
        "at fifteen yards she holds; at thirty and three she closes"
    );
}

/// Balinda polymorphs the second on her threat list from 1.75 seconds
/// in, and keeps her Cone of Cold for one within ten yards and her Arcane
/// Explosion for one within six (Thistle Tea's `hostile_second_aggro` and
/// `distance_to_target` conditions).
#[tokio::test]
async fn balinda_polymorphs_her_second_attacker_and_saves_cone_and_explosion_for_the_close() {
    let (_a, mut b, grom, her, map) = balinda().await;
    let (second, sheep, cone, blast, first) = map
        .query(move |m| {
            let at = m.position_of(her).unwrap();
            let second = super::tests_p3_scripts::summon(
                m,
                99_801,
                Vector3d {
                    x: at.x + 3.0,
                    ..at
                },
                0.0,
            );
            ai::add_threat(m, her, grom, 1_000.0);
            ai::add_threat(m, her, second, 10.0);
            let sheep = casting(m, BALINDA, 15534);
            let i = super::tests_p3_scripts::index(m, her, sheep);
            let e = &m.realm.data.scripts.ai_events[&BALINDA][i];
            (
                second,
                sheep,
                casting(m, BALINDA, 22746),
                casting(m, BALINDA, 19712),
                (e.kind, e.params[0], e.params[1]),
            )
        })
        .await
        .unwrap();
    assert_eq!(first, (EVENT_TIMER_IN_COMBAT, 1750, 1750), "1.75 s in");
    b.clear();
    map.query(move |m| due_now(m, her, sheep)).await;
    let casts = casts_by(&mut b, her, Duration::from_millis(400)).await;
    assert!(
        casts.contains(&(15534, second)),
        "Polymorph at her second: {casts:?}"
    );
    // Grom fifteen yards off: neither. Eight: the cone. Four: both.
    let mut seen = Vec::new();
    for dy in [0.0, 7.0, 11.0] {
        b.clear();
        map.query(move |m| {
            hold_at(
                m,
                her,
                Vector3d {
                    x: BALINDA_HOME.0,
                    y: BALINDA_HOME.1 + dy,
                    z: BALINDA_HOME.2,
                },
            );
            due_now(m, her, cone);
            due_now(m, her, blast);
        })
        .await;
        let casts = casts_by(&mut b, her, Duration::from_millis(400)).await;
        let mut close: Vec<u32> = spells_of(&casts)
            .into_iter()
            .filter(|s| [22746, 19712].contains(s))
            .collect();
        close.sort_unstable();
        close.dedup();
        seen.push(close);
    }
    assert_eq!(seen, vec![vec![], vec![22746], vec![19712, 22746]]);
}

/// A marshal charges its victim from eight to twenty-five yards, not
/// nearer, and whirlwinds twice, two seconds apart, from twelve seconds
/// in (vmangos's `npc_WarMaster`, Thistle Tea's `timed` second
/// Whirlwind).
#[tokio::test]
async fn a_warmaster_charges_from_range_and_whirlwinds_twice_in_a_row() {
    const MARSHAL: u32 = 14762;
    let mut d = valley();
    npc(&mut d, MARSHAL, "Dun Baldar North Marshal", 35);
    spawn_at(&mut d, 170601, MARSHAL, (100.0, 50.0, 0.0));
    instants(&mut d, &[22911, 20684], &[13736, 23511]);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, 115.0, 50.0, 0.0).await;
    let marshal = map.query(|m| creature(m, MARSHAL)).await.flatten().unwrap();
    b.sees_on(&map, marshal).await;
    let (charge, whirlwind, first) = map
        .query(move |m| {
            ai::attacked(m, marshal, grom);
            hold_at(
                m,
                marshal,
                Vector3d {
                    x: 100.0,
                    y: 50.0,
                    z: 0.0,
                },
            );
            let whirlwind =
                events_where(m, MARSHAL, |s| s.command == 15 && s.datalong[0] == 13736)[0];
            let i = super::tests_p3_scripts::index(m, marshal, whirlwind);
            let e = &m.realm.data.scripts.ai_events[&MARSHAL][i];
            (casting(m, MARSHAL, 22911), whirlwind, e.params[0])
        })
        .await
        .unwrap();
    assert_eq!(first, 12_000, "Whirlwind from twelve seconds in");
    // Fifteen yards: Charge. Four: none.
    let mut charged = Vec::new();
    for x in [100.0, 111.0] {
        b.clear();
        map.query(move |m| {
            hold_at(m, marshal, Vector3d { x, y: 50.0, z: 0.0 });
            due_now(m, marshal, charge);
        })
        .await;
        let casts = casts_by(&mut b, marshal, Duration::from_millis(400)).await;
        charged.push(casts.contains(&(22911, grom)));
    }
    assert_eq!(charged, vec![true, false], "from range only");
    // Whirlwind, and again two seconds on.
    b.clear();
    map.query(move |m| due_now(m, marshal, whirlwind)).await;
    let once = casts_by(&mut b, marshal, Duration::from_millis(400)).await;
    map.query(move |m| {
        if let Some(c) = m.caster_mut(marshal) {
            c.gcd_until = None;
        }
        hurry_runs(m, Some(marshal), Duration::from_millis(2_000));
    })
    .await;
    let again = casts_by(&mut b, marshal, Duration::from_millis(400)).await;
    let whirls = |c: &[(u32, Guid)]| spells_of(c).iter().filter(|s| **s == 13736).count();
    assert_eq!(
        (whirls(&once), whirls(&again)),
        (1, 1),
        "{once:?} {again:?}"
    );
}

/// Charge as the data has it, for these tests: range 95, 8 to 25 yards,
/// a charge at an enemy.
pub(super) fn real_shaped_charge(d: &mut GameData) {
    d.spells.ranges.insert(95, (8.0, 25.0));
    let mut s = SpellTemplate {
        id: av_scripts::CHARGE,
        name: "Charge".into(),
        range_index: 95,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 96,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(s.id, s);
}

/// The marshal of these tests, fighting Grom, held at `x` with Grom at
/// 115 (Charge's 8 to 25 yards are past both combat reaches, three yards
/// between them: 11 to 28 yards between their centres).
async fn a_marshal_fighting_grom() -> (Client, Guid, Client, Guid, MapHandle, Guid, u32) {
    const MARSHAL: u32 = 14762;
    let mut d = valley();
    npc(&mut d, MARSHAL, "Dun Baldar North Marshal", 35);
    spawn_at(&mut d, 170601, MARSHAL, (100.0, 50.0, 0.0));
    instants(&mut d, &[20684], &[13736, 23511]);
    real_shaped_charge(&mut d);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (a, alice, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, 115.0, 50.0, 0.0).await;
    let marshal = map.query(|m| creature(m, MARSHAL)).await.flatten().unwrap();
    b.sees_on(&map, marshal).await;
    let charge = map
        .query(move |m| {
            ai::attacked(m, marshal, grom);
            casting(m, MARSHAL, av_scripts::CHARGE)
        })
        .await
        .unwrap();
    (a, alice, b, grom, map, marshal, charge)
}

/// The user's rule for a warmaster's Charge, as the spell has it (8 to 25
/// yards, past both combat reaches, mangos's `GetMinMaxRange`): with its
/// victim beside it (none of its list in range), it doesn't charge, and
/// looks again 5 to 10 s on (vmangos's `npc_WarMasterAI`); 9.5 yards from its centre (past
/// the event's 8, short of the spell's least) its cast is refused, too
/// close, and it looks again at its next tick rather than 12 to 18 s on
/// (vmangos's timer, started over only by a cast that goes); 15 yards off,
/// it charges at once, its timer started over.
#[tokio::test]
async fn a_warmasters_charge_waits_past_the_spells_least_range_and_goes_at_once_beyond() {
    let (_a, _, mut b, grom, map, marshal, charge) = a_marshal_fighting_grom().await;
    let mut seen = Vec::new();
    for x in [112.0, 105.5, 100.0] {
        b.clear();
        let (left, charging) = map
            .query(move |m| {
                hold_at(m, marshal, Vector3d { x, y: 50.0, z: 0.0 });
                due_now(m, marshal, charge);
                let left = super::tests_p3_scripts::left(m, marshal, charge);
                let charging = m
                    .creature(marshal)
                    .is_some_and(|c| c.engagement.charge.is_some());
                (left, charging)
            })
            .await
            .unwrap();
        let casts = casts_by(&mut b, marshal, Duration::from_millis(300)).await;
        seen.push((
            casts.contains(&(av_scripts::CHARGE, grom)),
            charging,
            left.is_some_and(|l| l.is_zero()),
            left.is_some_and(|l| (Duration::from_secs(4)..=Duration::from_secs(10)).contains(&l)),
            left.is_some_and(|l| l >= Duration::from_secs(11)),
        ));
    }
    assert_eq!(
        seen,
        vec![
            (false, false, false, true, false),
            (false, false, true, false, false),
            (true, true, false, false, true),
        ],
        "(charged, charging, due again at once, 5 to 10 s on, 12 to 18 s on): beside, too \
         close, across"
    );
}

/// A random pick for a cast (target types 4 and 5, vmangos's hostile at
/// random and at random but the top) is one in the spell's range, from
/// its least, past both combat reaches (mangos's `SelectAttackingTarget`
/// with the spell's range): of Alice beside the marshal and Grom 15 yards
/// off, its Charge's pick is always Grom; with Grom gone from its list,
/// no one.
#[tokio::test]
async fn a_random_hostile_pick_for_a_charge_is_one_in_its_range() {
    let (mut a, alice, _b, grom, map, marshal, _) = a_marshal_fighting_grom().await;
    put(&map, &mut a, 100.0, 53.0, 0.0).await;
    let picks = map
        .query(move |m| {
            hold_at(
                m,
                marshal,
                Vector3d {
                    x: 100.0,
                    y: 50.0,
                    z: 0.0,
                },
            );
            ai::add_threat(m, marshal, alice, 1000.0);
            ai::add_threat(m, marshal, grom, 10.0);
            let pick = |m: &Map, target_type: u32| {
                let step = ScriptStep {
                    command: 15,
                    datalong: [av_scripts::CHARGE, 0, 0, 0],
                    target_type,
                    ..Default::default()
                };
                crate::game::map::scripts::resolve_targets(m, &step, marshal, Guid::zero())
                    .map(|(_, t)| t)
            };
            let random: Vec<Option<Guid>> = (0..40).map(|_| pick(m, 4)).collect();
            let not_top: Vec<Option<Guid>> = (0..40).map(|_| pick(m, 5)).collect();
            if let Some(c) = m.creature_mut(marshal) {
                c.threat.retain(|(t, _)| *t != grom);
            }
            (random, not_top, pick(m, 4))
        })
        .await
        .unwrap();
    let (random, not_top, alone) = picks;
    assert!(random.iter().all(|t| *t == Some(grom)), "{random:?}");
    assert!(not_top.iter().all(|t| *t == Some(grom)), "{not_top:?}");
    assert_eq!(alone, None, "Alice beside it is too close");
}

/// A war rider's Fireball, due from the fight's start, waits for its
/// victim to come within thirty yards, and goes the moment they are
/// (Thistle Tea's `within(30)` on the rider's three spells).
#[tokio::test]
async fn a_war_riders_fireball_waits_for_its_victim_within_thirty_yards() {
    const RIDER: u32 = 14946;
    let mut d = valley();
    npc(&mut d, RIDER, "Jeztor's War Rider", 35);
    spawn_at(&mut d, 170701, RIDER, (100.0, 50.0, 0.0));
    instants(&mut d, &[22088, 15285, 21188], &[]);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, 131.0, 50.0, 0.0).await;
    let rider = map.query(|m| creature(m, RIDER)).await.flatten().unwrap();
    b.sees_on(&map, rider).await;
    b.clear();
    map.query(move |m| {
        ai::attacked(m, rider, grom);
        hold_at(
            m,
            rider,
            Vector3d {
                x: 100.0,
                y: 50.0,
                z: 0.0,
            },
        );
    })
    .await;
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    let far = casts_by(&mut b, rider, Duration::from_millis(300)).await;
    assert!(
        !spells_of(&far).contains(&22088),
        "withheld at 31 yards: {far:?}"
    );
    map.query(move |m| {
        hold_at(
            m,
            rider,
            Vector3d {
                x: 106.0,
                y: 50.0,
                z: 0.0,
            },
        )
    })
    .await;
    let near = casts_by(&mut b, rider, Duration::from_secs(1)).await;
    assert!(
        near.contains(&(22088, grom)),
        "at 25 yards, at once: {near:?}"
    );
}

const LOKHOLAR: u32 = 13256;
const IVUS: u32 = 13419;

/// Lokholar feeds on the players he kills (Frost Nova's 21307 on himself),
/// not on creatures; and the lords fight with their kits, Lokholar's six
/// spells and Ivus's five on their timers (Thistle Tea's `events/1`).
#[tokio::test]
async fn lokholar_feeds_on_player_kills_while_both_lords_use_their_combat_kits() {
    let mut d = valley();
    npc(&mut d, LOKHOLAR, "Lokholar the Ice Lord", 35);
    npc(&mut d, 99_802, "Prey", 35);
    spawn_at(&mut d, 170801, LOKHOLAR, (100.0, 50.0, 0.0));
    instants(&mut d, &[], &[21307]);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut a, 104.0, 50.0, 0.0).await;
    put(&map, &mut b, 102.0, 50.0, 0.0).await;
    let lord = map
        .query(|m| creature(m, LOKHOLAR))
        .await
        .flatten()
        .unwrap();
    a.sees_on(&map, lord).await;
    // A creature's death feeds him nothing.
    a.clear();
    map.query(move |m| {
        let prey = super::tests_p3_scripts::summon(
            m,
            99_802,
            Vector3d {
                x: 98.0,
                y: 50.0,
                z: 0.0,
            },
            0.0,
        );
        crate::game::map::combat::deal_damage(m, lord, prey, 1_000_000);
    })
    .await;
    let fed = casts_by(&mut a, lord, Duration::from_millis(300)).await;
    assert!(!spells_of(&fed).contains(&21307), "{fed:?}");
    // A player's does.
    map.query(move |m| crate::game::map::combat::deal_damage(m, lord, grom, 1_000_000))
        .await;
    let fed = casts_by(&mut a, lord, Duration::from_millis(500)).await;
    assert!(fed.contains(&(21307, lord)), "{fed:?}");
    // Their kits, in order.
    let kits = map
        .query(|m| {
            let s = &m.realm.data.scripts;
            [LOKHOLAR, IVUS].map(|entry| {
                s.ai_events[&entry]
                    .iter()
                    .filter(|e| e.kind == EVENT_TIMER_IN_COMBAT)
                    .flat_map(|e| e.actions.iter().filter_map(|a| s.ai_scripts.get(a)))
                    .flatten()
                    .filter(|st| st.command == 15)
                    .map(|st| st.datalong[0])
                    .collect::<Vec<_>>()
            })
        })
        .await
        .unwrap();
    assert_eq!(
        kits,
        [
            vec![21367, 21369, 14907, 19133, 15878, 16869],
            vec![20654, 21670, 21669, 21668, 21667]
        ]
    );
}

/// The ritual's completion goes to the summoner waiting at her altar
/// (with Renferal there, Ivus rises: `tests_pvp2`); with her fallen, the
/// circle completed comes to nothing and its channellers are let go
/// (Thistle Tea's event 7268, sent to the nearest Renferal within forty
/// yards and taken up only as she invokes).
#[tokio::test]
async fn a_ritual_raises_its_lord_only_through_her_waiting_summoner() {
    let mut d = valley();
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (mut a, alice, _b, grom, map) = pair(&realm).await;
    turn_in(&map, alice, 7386, 40).await;
    arrive(&map).await;
    let (renferal, circle) = map
        .query(|m| {
            (
                creature(m, 13442).unwrap(),
                m.gameobjects
                    .iter()
                    .find(|(_, o)| o.entry == 178670)
                    .map(|(_, o)| (o.guid, o.position)),
            )
        })
        .await
        .unwrap();
    let (circle, at) = circle.expect("the Circle of Calling");
    let phase = map.query(move |m| event_ai::phase(m, renferal)).await;
    assert_eq!(phase, Some(3), "invoking at her altar");
    // She falls; the circle is completed all the same.
    map.query(move |m| crate::game::map::combat::deal_damage(m, grom, renferal, 1_000_000))
        .await;
    put(&map, &mut a, at.x, at.y, at.z).await;
    a.send(CMSG_GAMEOBJ_USE { guid: circle }).await;
    for _ in 0..30 {
        harness::an_update(&map).await;
    }
    let (ivus, channel, circle_left) = map
        .query(move |m| {
            (
                creature(m, IVUS),
                m.players[&alice].fields.get_u32(UNIT_CHANNEL_SPELL),
                m.gameobject(circle).is_some(),
            )
        })
        .await
        .unwrap();
    assert_eq!(ivus, None, "no one to take up the ritual");
    assert_eq!(channel, 0, "Alice let go");
    assert!(circle_left, "the circle stays");
}

/// `g` at `at`, standing, free to move as its fight takes it.
pub(super) fn place(m: &mut Map, g: Guid, at: Vector3d) {
    if let Some(c) = m.creature_mut(g) {
        c.position = at;
        c.motion = crate::game::map::creature::Motion::Idle { until: None };
    }
}

pub(super) fn no_gcd(m: &mut Map, g: Guid) {
    if let Some(c) = m.caster_mut(g) {
        c.gcd_until = None;
    }
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.gcd_until = None;
    }
}

/// Where `g` stands, after `n` of the map's updates.
async fn after_updates(map: &MapHandle, g: Guid, n: u32) -> Vector3d {
    for _ in 0..n {
        harness::an_update(map).await;
    }
    map.query(move |m| m.position_of(g))
        .await
        .flatten()
        .unwrap()
}

/// Waits until `g` has moved off `from`.
async fn moves_off(map: &MapHandle, g: Guid, from: Vector3d) -> bool {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while tokio::time::Instant::now() < deadline {
        let at = map.query(move |m| m.position_of(g)).await.flatten();
        if at.is_some_and(|p| crate::game::map::combat::dist(p, from) > 0.5) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

/// A marshal in reach of Grom, fighting him, with its Whirlwind, Cleave
/// and Charge events.
async fn a_marshal_beside_grom() -> (Client, Client, Guid, MapHandle, Guid, [u32; 3]) {
    const MARSHAL: u32 = 14762;
    let mut d = valley();
    npc(&mut d, MARSHAL, "Dun Baldar North Marshal", 35);
    spawn_at(&mut d, 170601, MARSHAL, (100.0, 50.0, 0.0));
    instants(
        &mut d,
        &[av_scripts::CHARGE, av_scripts::CLEAVE],
        &[av_scripts::WHIRLWIND, av_scripts::DEMORALIZING_SHOUT],
    );
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, 115.0, 50.0, 0.0).await;
    let marshal = map.query(|m| creature(m, MARSHAL)).await.flatten().unwrap();
    b.sees_on(&map, marshal).await;
    let events = map
        .query(move |m| {
            ai::attacked(m, marshal, grom);
            place(
                m,
                marshal,
                Vector3d {
                    x: 113.0,
                    y: 50.0,
                    z: 0.0,
                },
            );
            [
                av_scripts::WHIRLWIND,
                av_scripts::CLEAVE,
                av_scripts::CHARGE,
            ]
            .map(|s| casting(m, MARSHAL, s))
        })
        .await
        .unwrap();
    (a, b, grom, map, marshal, events)
}

/// vmangos's `npc_WarMaster`: for the four seconds of its two Whirlwinds a
/// warmaster stands (`UNIT_STATE_ROOT`) and casts nothing else (its global
/// cooldown): its victim walking off, it neither follows nor charges, and
/// a Cleave due meanwhile waits; at the end it goes after them, charging
/// and cleaving at its next look.
#[tokio::test]
async fn a_whirlwinding_warmaster_stands_and_holds_its_other_spells_until_it_ends() {
    let (_a, mut b, grom, map, marshal, [whirlwind, cleave, charge]) =
        a_marshal_beside_grom().await;
    b.clear();
    let at = map
        .query(move |m| {
            due_now(m, marshal, whirlwind);
            m.position_of(marshal).unwrap()
        })
        .await
        .unwrap();
    // Grom walks off, past Charge's least; a Cleave falls due.
    put(&map, &mut b, 130.0, 50.0, 0.0).await;
    map.query(move |m| {
        no_gcd(m, marshal);
        due(m, marshal, cleave);
        due(m, marshal, charge);
    })
    .await;
    let first = casts_by(&mut b, marshal, Duration::from_millis(800)).await;
    let held = after_updates(&map, marshal, 4).await;
    map.query(move |m| {
        no_gcd(m, marshal);
        hurry_runs(m, Some(marshal), Duration::from_millis(2_000));
    })
    .await;
    let second = casts_by(&mut b, marshal, Duration::from_millis(800)).await;
    let (still, state) = {
        let p = after_updates(&map, marshal, 4).await;
        let state = map
            .query(move |m| {
                (
                    event_ai::phase(m, marshal),
                    event_ai::combat_movement(m, marshal),
                    m.creature(marshal).map(|c| c.combat.victim),
                )
            })
            .await
            .unwrap();
        (p, state)
    };
    assert_eq!(
        (spells_of(&first), spells_of(&second)),
        (vec![av_scripts::WHIRLWIND], vec![av_scripts::WHIRLWIND]),
        "nothing but its Whirlwinds"
    );
    assert_eq!((held, still), (at, at), "it stands where it whirls");
    assert_eq!(
        state,
        (av_scripts::WHIRLING, false, Some(grom)),
        "whirling, rooted, on Grom"
    );
    // The four seconds are up: it moves, and its Cleave and Charge go.
    map.query(move |m| {
        no_gcd(m, marshal);
        hurry_runs(m, Some(marshal), Duration::from_millis(2_000));
    })
    .await;
    assert!(moves_off(&map, marshal, at).await, "it goes after Grom");
    let after = casts_by(&mut b, marshal, Duration::from_millis(1_500)).await;
    let after = spells_of(&after);
    assert!(
        after.contains(&av_scripts::CLEAVE) && after.contains(&av_scripts::CHARGE),
        "{after:?}"
    );
}

/// A warmaster out of its fight mid-whirl (its scripts stopped, the end of
/// the whirl among them) moves again in its next.
#[tokio::test]
async fn a_warmaster_that_evades_mid_whirl_moves_in_its_next_fight() {
    let (_a, mut b, grom, map, marshal, [whirlwind, ..]) = a_marshal_beside_grom().await;
    map.query(move |m| due_now(m, marshal, whirlwind)).await;
    let whirling = map
        .query(move |m| event_ai::phase(m, marshal))
        .await
        .unwrap();
    assert_eq!(whirling, av_scripts::WHIRLING);
    map.query(move |m| ai::enter_evade(m, marshal)).await;
    let state = map
        .query(move |m| {
            (
                event_ai::phase(m, marshal),
                event_ai::combat_movement(m, marshal),
            )
        })
        .await
        .unwrap();
    assert_eq!(state, (0, true), "out of the whirl as it evades");
    // Home again, it takes Grom on from across the room and goes to him.
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let home = map
            .query(move |m| m.creature(marshal).is_some_and(|c| !c.evading))
            .await
            .unwrap();
        if home {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "it gets home");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    put(&map, &mut b, 130.0, 50.0, 0.0).await;
    let at = map
        .query(move |m| {
            ai::attacked(m, marshal, grom);
            m.position_of(marshal).unwrap()
        })
        .await
        .unwrap();
    assert!(moves_off(&map, marshal, at).await, "it goes after Grom");
}

/// vmangos's `npc_DrekThar`: his Whirlwind holds him two seconds
/// (`m_uiIsInWhirlwind_Timer`), and a Knockdown due meanwhile waits for
/// its end (his global cooldown); then he goes after his victim. Out of
/// his fight mid-whirl, he moves again.
#[tokio::test]
async fn drek_thars_whirlwind_holds_him_two_seconds_and_his_knockdown_waits() {
    let mut d = valley();
    if let Some(s) = d
        .creature_spawns
        .get_mut(&AV)
        .and_then(|s| s.iter_mut().find(|s| s.entries == [DREK_THAR]))
    {
        (s.x, s.y, s.z) = DREK_HOME;
    }
    instants(&mut d, &[19128], &[av_scripts::WHIRLWIND, 28747]);
    av_scripts::install(&mut d);
    let realm = harness::realm_with(d);
    let (_a, _, mut b, grom, map) = pair(&realm).await;
    put(&map, &mut b, DREK_HOME.0 + 2.0, DREK_HOME.1, DREK_HOME.2).await;
    let drek = map
        .query(|m| creature(m, DREK_THAR))
        .await
        .flatten()
        .unwrap();
    b.sees_on(&map, drek).await;
    let home = Vector3d {
        x: DREK_HOME.0,
        y: DREK_HOME.1,
        z: DREK_HOME.2,
    };
    let (whirlwind, knockdown) = map
        .query(move |m| {
            ai::attacked(m, drek, grom);
            place(m, drek, home);
            (
                casting(m, DREK_THAR, av_scripts::WHIRLWIND),
                casting(m, DREK_THAR, 19128),
            )
        })
        .await
        .unwrap();
    b.clear();
    map.query(move |m| due_now(m, drek, whirlwind)).await;
    put(&map, &mut b, DREK_HOME.0 + 12.0, DREK_HOME.1, DREK_HOME.2).await;
    map.query(move |m| {
        no_gcd(m, drek);
        due(m, drek, knockdown);
    })
    .await;
    let during = casts_by(&mut b, drek, Duration::from_millis(800)).await;
    let held = after_updates(&map, drek, 4).await;
    assert_eq!(
        spells_of(&during),
        vec![av_scripts::WHIRLWIND],
        "no Knockdown"
    );
    assert_eq!(held, home, "he stands where he whirls");
    map.query(move |m| {
        no_gcd(m, drek);
        hurry_runs(m, Some(drek), Duration::from_millis(2_000));
    })
    .await;
    assert!(moves_off(&map, drek, home).await, "he goes after Grom");
    let after = casts_by(&mut b, drek, Duration::from_millis(1_500)).await;
    assert!(spells_of(&after).contains(&19128), "{after:?}");
    // Out of a fight mid-whirl: he moves again, fighting as before.
    let state = map
        .query(move |m| {
            no_gcd(m, drek);
            due(m, drek, whirlwind);
            let whirling = event_ai::phase(m, drek);
            ai::enter_evade(m, drek);
            (
                whirling,
                event_ai::phase(m, drek),
                event_ai::combat_movement(m, drek),
            )
        })
        .await
        .unwrap();
    assert_eq!(state, (av_scripts::WHIRLING, 0, true));
}

/// The real data's Whirlwind: 13736 a two-second aura whose period (2000
/// ms) triggers 15589, a blow of the weapon's plus 300 and 6 a level from
/// its spell level, 1 (vmangos's `CalculateSpellEffectValue`): 654 more at
/// 60, a warmaster's, and 666 at 62, an Elite Guard's.
#[test]
#[ignore = "extended"]
fn the_real_whirlwind_is_a_weapon_blow_and_654_at_sixty() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let aura = d.spells.get(av_scripts::WHIRLWIND).expect("Whirlwind");
    let e = aura.effects[0];
    assert_eq!((e.aura, e.amplitude, e.trigger_spell), (23, 2000, 15589));
    let blow = d.spells.get(15589).expect("Whirlwind's blow");
    assert_eq!(
        blow.effects[0].effect,
        crate::game::map::spells::EFFECT_WEAPON_DAMAGE_NOSCHOOL
    );
    assert_eq!(
        [60, 62].map(|l| crate::game::map::spells::points(blow, 0, l)),
        [654, 666]
    );
}
