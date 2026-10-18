//! Phase 3's second loader stage, the creatures (Thistle Tea's
//! `world/loader/mob_vmangos_test.exs`): what the real creatures come with
//! (their stats, spell lists and charm spells, addons, EventAI and scripts),
//! over the real data when it is here.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::conditions::{self, Truth};
use crate::game::map::spells;
use crate::game::map::{Map, MapHandle, MapKey, pets};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// The spawn `spawn` of `entry` on its map, as the map has it.
fn spawned(m: &Map, entry: u32, spawn: u32) -> Guid {
    let g = guid::unit(entry, spawn);
    assert!(m.creature(g).is_some(), "spawn {spawn} of {entry}");
    g
}

fn ai_event(d: &GameData, creature: u32, kind: u32) -> &AiEvent {
    d.scripts.ai_events[&creature]
        .iter()
        .find(|e| e.kind == kind)
        .unwrap_or_else(|| panic!("{creature}: event {kind}"))
}

fn ai_steps(d: &GameData, e: &AiEvent) -> Vec<ScriptStep> {
    e.actions
        .iter()
        .filter(|a| **a != 0)
        .flat_map(|a| d.scripts.ai_scripts.get(a).cloned().unwrap_or_default())
        .collect()
}

/// A talk step's broadcast texts, each with words.
fn texts(d: &GameData, s: &ScriptStep) -> usize {
    assert_eq!(s.command, 0, "a talk");
    s.dataint
        .iter()
        .filter(|t| **t > 0)
        .inspect(|t| {
            assert!(
                d.npc
                    .broadcast_line(**t as u32, false)
                    .is_some_and(|l| !l.is_empty()),
                "broadcast text {t}"
            )
        })
        .count()
}

const DEFIAS_PILLAGER: u32 = 589;
const EMBERSTRIFE: u32 = 10_321;

/// Thistle Tea's Defias Pillager and Emberstrife (`mob_vmangos_test.exs`):
/// the Pillager's spell list is its fireball, it is of level 14 or 15
/// with 220 to 320 health, hits for something, and is of a human's size;
/// charmed, it offers its two charm spells, the first recovering at once
/// and the second over 9 to 12 seconds rolled, in place of the spell's
/// own; charmed Emberstrife's Flames of the Black Flight recovers over the
/// ten seconds (10000, already milliseconds) its row means.
#[tokio::test]
#[ignore = "extended"]
async fn real_creatures_have_their_stats_and_charm_spells() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let list = d.creature_template(DEFIAS_PILLAGER).unwrap().spell_list_id;
    assert_eq!(
        d.spells.creature_spells[&list]
            .iter()
            .map(|s| s.spell)
            .collect::<Vec<_>>(),
        vec![19_816]
    );
    let mut c = Client::connect(&realm, "P3OCHARM");
    let priest = c.enter("Charma", Race::Human, Class::Priest).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&priest).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        let at = m.players[&priest].position();
        let mut out = Vec::new();
        for entry in [DEFIAS_PILLAGER, EMBERSTRIFE] {
            let g = crate::game::map::scripts::summon(m, Guid::zero(), entry, at, 0.0, None)
                .expect("the creature");
            let f = &m.creature(g).unwrap().fields;
            let stats = (
                f.get_u32(UNIT_FIELD_LEVEL),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
                f.get_f32(UNIT_FIELD_MINDAMAGE),
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
                f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
                f.get_f32(UNIT_FIELD_COMBATREACH),
                m.creature(g).unwrap().auras.holders.len(),
            );
            pets::charmed(m, priest, g, false);
            let state = m.creature(g).unwrap().pet.as_ref().unwrap();
            let picked = (state.spells.clone(), state.charm_cooldowns.clone());
            // Each charm spell cast: its cooldown.
            let mut cooldowns = Vec::new();
            for spell in picked.0.clone() {
                let now = m.now;
                if let Some(c) = m.caster_mut(g) {
                    c.cooldowns.clear();
                    c.gcd_until = None;
                }
                let s = data.spells.get(spell).unwrap().clone();
                let mods = spells::mods::for_spell(m, g, &s);
                spells::cooldowns::start(m, g, &s, &mods, None);
                // Frost Armor's starts as its aura goes.
                spells::cooldowns::release(m, g, spell, true);
                cooldowns.push(
                    m.caster(g)
                        .and_then(|c| c.cooldowns.get(&spell))
                        .map(|t| t.duration_since(now)),
                );
            }
            out.push((stats, picked, cooldowns));
        }
        out
    })
    .await;
    let ((level, health, low, high, radius, reach, auras), (spells, rolled), cooldowns) =
        out[0].clone();
    assert!((14..=15).contains(&level), "{level}");
    assert!((220..=320).contains(&health), "{health}");
    assert!(low > 0.0 && high > low, "{low} {high}");
    assert!([0.208, 0.306].contains(&radius), "{radius}");
    assert_eq!((reach, auras), (1.5, 0));
    assert_eq!(spells, vec![20_793, 12_544]);
    assert_eq!(rolled[0], (20_793, 0));
    assert!((9_000..=12_000).contains(&rolled[1].1), "{rolled:?}");
    assert_eq!(cooldowns[0], None, "at once");
    assert_eq!(
        cooldowns[1],
        Some(Duration::from_millis(u64::from(rolled[1].1)))
    );
    let (_, (spells, rolled), cooldowns) = out[1].clone();
    assert_eq!(spells[0], 16_054);
    assert_eq!(rolled[0], (16_054, 10_000));
    assert_eq!(cooldowns[0], Some(Duration::from_secs(10)));
}

/// Thistle Tea's addons (`mob_vmangos_test.exs`): Onyxia's spawn sleeps
/// (stand state 3), Kalimdor's 18583 (a Lunar Festival spawn) wears its addon's model, and the
/// Blackrock spawn 90658 its own equipment rather than its template's.
#[tokio::test]
#[ignore = "extended"]
async fn real_spawn_addons_set_posture_model_and_equipment() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let lair = realm.maps.get(
        &realm,
        MapKey {
            map: 249,
            instance: 3,
        },
    );
    let stand = on(&lair, |m| {
        let g = spawned(m, 10_184, 47_572);
        m.creature(g).unwrap().fields.get_u32(UNIT_FIELD_BYTES_1) & 0xFF
    })
    .await;
    assert_eq!(stand, 3);
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let display = on(&kalimdor, |m| {
        // A Lunar Festival spawn, out of its event: brought in.
        crate::game::map::ai::spawn_creature_of(m, 18_583);
        let g = spawned(m, 15_917, 18_583);
        m.creature(g).unwrap().fields.get_u32(UNIT_FIELD_DISPLAYID)
    })
    .await;
    assert_eq!(display, 15_863);
    let brd = realm.maps.get(
        &realm,
        MapKey {
            map: 230,
            instance: 3,
        },
    );
    let (own, template) = on(&brd, |m| {
        let g = spawned(m, 8895, 90_658);
        let at = m.creature(g).unwrap().position;
        let own: Vec<u32> = (0..3)
            .map(|i| {
                m.creature(g)
                    .unwrap()
                    .fields
                    .get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i)
            })
            .collect();
        let other =
            crate::game::map::scripts::summon(m, Guid::zero(), 8895, at, 0.0, None).unwrap();
        let template: Vec<u32> = (0..3)
            .map(|i| {
                m.creature(other)
                    .unwrap()
                    .fields
                    .get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i)
            })
            .collect();
        (own, template)
    })
    .await;
    assert_ne!(own, template);
}

/// Thistle Tea's EventAI and scripts on the real creatures
/// (`mob_vmangos_test.exs`): Vincent stands dead (7) on reaching point 9
/// of path 1, retreats by a point move, and evades on a timer; the
/// Blackrock grunt's emotes (Thistle Tea's port) are gated on quest 3861
/// not taken and taken; Ironforge's 108 talks at point 7 in one of four
/// lines; the Defias Messenger's aggro starts one of two scripts, each a
/// line of four, or of its own; the Eastvale peasant's walk starts its
/// supervisor's script (spawn 81251) with the targets swapped, which
/// talks and emotes.
#[test]
#[ignore = "extended"]
fn the_real_creature_scripts_load_with_what_they_name() {
    let Some(d) = real() else {
        return;
    };
    // Vincent.
    let arrival = ai_event(d, 4444, EVENT_MOVEMENT_INFORM);
    assert_eq!(arrival.params[..2], [9, 1]);
    let stand: Vec<(u32, u32)> = ai_steps(d, arrival)
        .iter()
        .map(|s| (s.command, s.datalong[0]))
        .collect();
    assert_eq!(stand, vec![(28, 7)]);
    let retreat = ai_steps(d, ai_event(d, 4444, EVENT_HP));
    let to = retreat.iter().find(|s| s.command == 3).unwrap();
    assert_eq!((to.datalong[2], to.datalong[3], to.dataint[0]), (68, 3, 1));
    let cleanup = ai_steps(d, ai_event(d, 4444, EVENT_TIMER_IN_COMBAT));
    assert_eq!(
        cleanup.iter().map(|s| s.command).collect::<Vec<_>>(),
        vec![33]
    );
    // The grunt's emotes: quest 3861 not taken (22) and taken (9).
    let emotes: Vec<(i32, i32)> = d.scripts.ai_events[&620]
        .iter()
        .filter(|e| e.kind == EVENT_RECEIVE_EMOTE)
        .map(|e| {
            let c = d.scripts.conditions[&e.condition];
            (c.kind, c.values[0])
        })
        .collect();
    assert!(emotes.contains(&(22, 3861)), "{emotes:?}");
    assert!(emotes.contains(&(9, 3861)), "{emotes:?}");
    // 108's point 7.
    let point = d.waypoints[&108].iter().find(|w| w.point == 7).unwrap();
    let talk = &d.scripts.movement[&point.script_id];
    assert_eq!(talk.len(), 1);
    assert_eq!(texts(d, &talk[0]), 4);
    // The Messenger's two scripts at even odds.
    let aggro = ai_steps(d, ai_event(d, 550, EVENT_AGGRO));
    let start = aggro.iter().find(|s| s.command == 39).unwrap();
    assert_eq!(
        (start.datalong[..2].to_vec(), start.dataint[..2].to_vec()),
        (vec![55_001, 55_002], vec![50, 50])
    );
    assert_eq!(texts(d, &d.scripts.generic[&55_001][0]), 4);
    assert!(texts(d, &d.scripts.generic[&55_002][0]) > 0);
    // The peasant's supervisor, by spawn (target 11), the targets swapped
    // (data flag 2).
    let walk = d.waypoints[&81_249]
        .iter()
        .flat_map(|w| {
            d.scripts
                .movement
                .get(&w.script_id)
                .cloned()
                .unwrap_or_default()
        })
        .find(|s| s.command == 39 && s.datalong[0] == 1_061_601)
        .unwrap();
    assert_eq!(
        (walk.target_type, walk.target_param1, walk.data_flags & 2),
        (11, 81_251, 2)
    );
    let sub = &d.scripts.generic[&1_061_601];
    assert_eq!(
        sub.iter().map(|s| s.command).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(sub[0].dataint.iter().any(|t| {
        *t > 0
            && d.npc
                .broadcast_line(*t as u32, false)
                .is_some_and(|l| l.starts_with("Daylight is still upon us"))
    }));
}

/// Thistle Tea's guid-scoped condition (`mob_vmangos_test.exs`): the
/// Defias Thug's emote event is gated on condition 3804, which holds for
/// its spawns 80152 and 80151 (and their fellows), not for 80147.
#[tokio::test]
#[ignore = "extended"]
async fn real_guid_scoped_conditions_hold_for_their_spawns() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    assert!(d.scripts.ai_events[&38].iter().any(|e| e.condition == 3804));
    let realm = realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let judged = on(&map, |m| {
        [80_152, 80_151, 80_147]
            .iter()
            .map(|s| conditions::check(m, 3804, Some(spawned(m, 38, *s)), None))
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(judged, vec![Truth::Met, Truth::Met, Truth::Unmet]);
}
