//! Conditions as Thistle Tea judges them (map/conditions.rs): three-valued
//! logic, the leaves the sweep found apart, the greeting a menu picks, the
//! gated loot references, spell script targets' conditions, and the same on
//! the real world database.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::GossipMenu;
use crate::game::data::scripts::{CF_REVERSE_RESULT, CF_SWAP_TARGETS, Condition, ScriptStep};
use crate::game::data::spells::{ATTR_ABILITY, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::conditions::{Truth, check, met};
use crate::game::map::creature::Life;
use crate::game::map::gameobject::GameObject;
use crate::game::map::player::Item;
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const RAT: u32 = 3100;
const GUARD: u32 = 3101;
const FIEND: u32 = 3102;
const CHEST: u32 = 4000;
const BUTTON: u32 = 4001;
const CHEST_SPAWN: u32 = 77_001;
const GONE_SPAWN: u32 = 77_002;
const BUTTON_SPAWN: u32 = 77_003;
const TOKEN: u32 = 52_001;
const QUEST: u32 = 9_001;
const COMMISSION: u32 = 17_670;
const SHOT: u32 = 91_100;

fn cond(d: &mut GameData, id: u32, kind: i32, values: [i32; 4], flags: u32) {
    d.scripts.conditions.insert(
        id,
        Condition {
            entry: id,
            kind,
            values,
            flags,
        },
    );
}

fn creature(entry: u32, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        ..Default::default()
    }
}

fn object_spawn(guid: u32, entry: u32, dx: f32, despawned: bool) -> GameObjectSpawn {
    GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        state: 1,
        anim_progress: 255,
        despawned,
        ..Default::default()
    }
}

fn faction(id: u32, faction: u32, ours: u32, friends: u32, enemies: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction,
        ours,
        friend_mask: friends,
        enemy_mask: enemies,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(RAT, creature(RAT, 14));
    d.creature_templates.insert(GUARD, creature(GUARD, 11));
    d.creature_templates.insert(FIEND, creature(FIEND, 14));
    // A human's template, a guard's of the same side, a monster's.
    d.faction_templates.insert(1, faction(1, 1, 0x3, 0x2, 0x8));
    d.faction_templates
        .insert(11, faction(11, 72, 0x2, 0x2, 0xC));
    d.faction_templates.insert(14, faction(14, 14, 0x8, 0, 0x7));
    for (entry, kind) in [(CHEST, 3), (BUTTON, 1)] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind,
                name: format!("Object {entry}"),
                display_id: 1,
                ..Default::default()
            },
        );
    }
    d.gameobject_spawns.insert(
        0,
        vec![
            object_spawn(CHEST_SPAWN, CHEST, 3.0, false),
            object_spawn(GONE_SPAWN, CHEST, 6.0, true),
            object_spawn(BUTTON_SPAWN, BUTTON, 9.0, false),
        ],
    );
    d.items.insert(
        TOKEN,
        ItemTemplate {
            entry: TOKEN,
            name: "Token".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    let mut quest = crate::game::data::npc::Quest {
        id: QUEST,
        ..Default::default()
    };
    quest.required_kills[0] = (RAT as i32, 1);
    d.npc.quests.insert(QUEST, quest);
    let aura = SpellEffect {
        effect: 6,
        aura: 4,
        target_a: 1,
        ..Default::default()
    };
    let mut commission = SpellTemplate {
        id: COMMISSION,
        name: "Argent Dawn Commission".into(),
        range_index: 1,
        equipped_item_class: -1,
        spell_visual: 3580,
        effects: [aura, SpellEffect::default(), SpellEffect::default()],
        ..Default::default()
    };
    commission.attributes[0] = ATTR_ABILITY;
    d.spells.spells.insert(COMMISSION, commission);
    d
}

async fn enter(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

fn summon(m: &mut Map, entry: u32, dx: f32, dy: f32) -> Guid {
    crate::game::map::scripts::summon(m, Guid::zero(), entry, at(dx, dy), 0.0, None)
        .expect("the creature")
}

fn kill(m: &mut Map, g: Guid) {
    let now = Instant::now();
    if let Some(c) = m.creature_mut(g) {
        c.life = Life::Dead {
            corpse_until: now + Duration::from_secs(600),
            respawn_at: now + Duration::from_secs(600),
        };
    }
}

fn put(m: &mut Map, g: Guid, dx: f32, dy: f32) {
    if let Some(p) = m.players.get_mut(&g) {
        p.movement.position = at(dx, dy);
    }
}

/// What can't be judged stays unknown through a not and a reverse, and an
/// and or an or comes out as Thistle Tea's `Result` has it; a terminate
/// step doesn't end on one; a step with no target is judged on its source.
#[tokio::test]
async fn unknown_stays_unknown_through_not_and_reverse() {
    let mut d = data();
    cond(&mut d, 1, 18, [309, 1, 0, 0], 0);
    cond(&mut d, 2, 18, [309, 1, 0, 0], CF_REVERSE_RESULT);
    cond(&mut d, 3, -3, [1, 0, 0, 0], 0);
    cond(&mut d, 4, 0, [0; 4], 0);
    cond(&mut d, 5, -2, [1, 4, 0, 0], 0);
    cond(&mut d, 6, -1, [1, 4, 0, 0], 0);
    cond(&mut d, 7, -3, [4, 0, 0, 0], 0);
    cond(&mut d, 8, -1, [1, 7, 0, 0], 0);
    cond(&mut d, 9, -2, [1, 7, 0, 0], 0);
    // A child with no row, and an or with no children.
    cond(&mut d, 10, -3, [99_999, 0, 0, 0], 0);
    cond(&mut d, 11, -2, [0; 4], 0);
    // A player's item, of a creature: unknown either way round.
    cond(&mut d, 12, 2, [TOKEN as i32, 1, 0, 0], 0);
    cond(&mut d, 13, 2, [TOKEN as i32, 1, 0, 0], CF_REVERSE_RESULT);
    // Has no pet, of the source (Bloodsail Warlock's 1564).
    cond(&mut d, 14, 40, [0; 4], CF_REVERSE_RESULT | CF_SWAP_TARGETS);
    cond(&mut d, 15, 40, [0; 4], 0);
    // The types not judged: a path to the victim, the last waypoint, an
    // inline type Thistle Tea doesn't know.
    cond(&mut d, 16, 13, [0; 4], CF_REVERSE_RESULT);
    cond(&mut d, 17, 32, [0; 4], CF_REVERSE_RESULT);
    cond(&mut d, 18, i32::MIN, [0; 4], CF_REVERSE_RESULT);
    // Alive.
    cond(&mut d, 19, 46, [0; 4], 0);
    let realm = harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "Ada").await;
    let out = map
        .query(move |m| {
            let rat = summon(m, RAT, 5.0, 0.0);
            let pet = summon(m, RAT, 6.0, 0.0);
            let c = |m: &Map, id| check(m, id, Some(rat), Some(me));
            let logic: Vec<Truth> = (1..=11).map(|id| c(m, id)).collect();
            let on_rat = (
                check(m, 12, Some(me), Some(rat)),
                check(m, 13, Some(me), Some(rat)),
                check(m, 13, Some(rat), Some(me)),
                met(m, 2, Some(rat), Some(me)),
            );
            let no_pet = check(m, 14, Some(rat), Some(me));
            m.players.get_mut(&me).unwrap().owned.pet = Some(pet);
            let has_pet = check(m, 15, Some(rat), Some(me));
            let unjudged: Vec<Truth> = (16..=18).map(|id| c(m, id)).collect();
            // A terminate step: unknown ends nothing, either way round.
            let terminate = |cond: u32, flags: u32| ScriptStep {
                command: 32,
                datalong: [cond, 0, flags, 0],
                ..Default::default()
            };
            let go_on = (
                crate::game::map::scripts::execute(m, &terminate(1, 0), rat, me),
                crate::game::map::scripts::execute(m, &terminate(1, 1), rat, me),
                crate::game::map::scripts::execute(m, &terminate(4, 0), rat, me),
            );
            // No target: the rat is the one judged alive, and it ends.
            let on_source =
                crate::game::map::scripts::execute(m, &terminate(19, 0), rat, Guid::zero());
            (logic, on_rat, no_pet, has_pet, unjudged, go_on, on_source)
        })
        .await
        .unwrap();
    let (logic, on_rat, no_pet, has_pet, unjudged, go_on, on_source) = out;
    use Truth::*;
    assert_eq!(
        logic,
        vec![
            Unknown, Unknown, Unknown, Met, Met, Unknown, Unmet, Unmet, Unknown, Unknown, Unknown
        ]
    );
    assert_eq!(on_rat, (Unknown, Unknown, Met, false));
    assert_eq!(
        (no_pet, has_pet),
        (Met, Met),
        "no pet of its own; the player's"
    );
    assert_eq!(unjudged, vec![Unknown, Unknown, Unknown]);
    assert_eq!(go_on, (true, true, false), "only a met condition ends it");
    assert!(!on_source, "judged on the source when there is no target");
}

/// The player's own facts: items carried and banked, a skill below, a
/// failed quest, the rank as shown, the Argent Dawn commission, and the
/// greeting a menu picks.
#[tokio::test]
async fn the_players_own_facts() {
    let mut d = data();
    let t = TOKEN as i32;
    cond(&mut d, 20, 2, [t, 1, 0, 0], 0);
    cond(&mut d, 21, 2, [t, 3, 0, 0], 0);
    cond(&mut d, 22, 23, [t, 7, 0, 0], 0);
    cond(&mut d, 23, 23, [t, 8, 0, 0], 0);
    // Doesn't know tailoring; below 300 in it.
    cond(&mut d, 24, 29, [197, 1, 0, 0], 0);
    cond(&mut d, 25, 29, [197, 300, 0, 0], 0);
    // The quest taken: either way, incomplete, complete, a mode with no
    // meaning.
    for (id, mode) in [(26, 0), (27, 1), (28, 2), (29, 3)] {
        cond(&mut d, id, 9, [QUEST as i32, mode, 0, 0], 0);
    }
    // The rank shown at least 6 (Knight-Lieutenant).
    cond(&mut d, 30, 51, [6, 1, 0, 0], 0);
    cond(&mut d, 31, 10, [0; 4], 0);
    // Menu 4264's: tailoring 50 and up, tailoring at all.
    cond(&mut d, 372, 7, [197, 50, 0, 0], 0);
    cond(&mut d, 14, 7, [197, 1, 0, 0], 0);
    d.npc.gossip_menus.insert(
        500,
        GossipMenu {
            texts: vec![(5422, 0), (5424, 14), (5423, 372)],
            options: Vec::new(),
        },
    );
    d.npc.gossip_menus.insert(
        501,
        GossipMenu {
            texts: vec![(600, 14)],
            options: Vec::new(),
        },
    );
    // A condition with no row lets its text be.
    d.npc.gossip_menus.insert(
        502,
        GossipMenu {
            texts: vec![(700, 0), (701, 88_888)],
            options: Vec::new(),
        },
    );
    let realm = harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "Bea").await;
    let out = map
        .query(move |m| {
            let rat = summon(m, RAT, 5.0, 0.0);
            let c = |m: &Map, id| check(m, id, Some(rat), Some(me));
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&me).unwrap();
            for (n, (bag, slot, count)) in [
                (crate::game::store::BAG_PLAYER, 23, 1),
                (crate::game::store::BAG_PLAYER, 81, 1),
                (crate::game::store::BAG_PLAYER, 39, 3),
                (63, 0, 2),
            ]
            .into_iter()
            .enumerate()
            {
                let item = Item::new(
                    &data,
                    guid::item(900_001 + n as u32),
                    me,
                    TOKEN,
                    bag,
                    slot,
                    count,
                );
                p.items.push(item);
            }
            let items: Vec<Truth> = (20..=23).map(|id| c(m, id)).collect();
            let unskilled = (c(m, 24), c(m, 25));
            let greet = |m: &Map, menu| crate::game::map::npc::greeting(m, menu, rat, me);
            let no_tailoring = (greet(m, 500), greet(m, 501), greet(m, 502));
            let p = m.players.get_mut(&me).unwrap();
            crate::game::map::skills::learn_skill(p, 197, 4);
            crate::game::map::skills::raise(p, 197, 9);
            let tailor_10 = greet(m, 500);
            let skilled = (c(m, 24), c(m, 25));
            let p = m.players.get_mut(&me).unwrap();
            crate::game::map::skills::raise(p, 197, 40);
            let tailor_50 = greet(m, 500);
            let p = m.players.get_mut(&me).unwrap();
            p.journal.log.push(crate::game::store::Logged {
                quest: QUEST,
                failed: true,
                ..Default::default()
            });
            let failed: Vec<Truth> = (26..=29).map(|id| c(m, id)).collect();
            m.players.get_mut(&me).unwrap().journal.log[0].failed = false;
            let taken: Vec<Truth> = (26..=29).map(|id| c(m, id)).collect();
            let p = m.players.get_mut(&me).unwrap();
            p.fields.set_byte(PLAYER_BYTES_3, 3, 10);
            let knight = c(m, 30);
            let p = m.players.get_mut(&me).unwrap();
            p.fields.set_byte(PLAYER_BYTES_3, 3, 6);
            let corporal = c(m, 30);
            let rank_of_rat = check(m, 30, Some(me), Some(rat));
            let uncommissioned = c(m, 31);
            let spell = m.realm.data.spells.get(COMMISSION).unwrap().clone();
            crate::game::map::auras::apply_spell(m, me, me, &spell, &[(0, 0)], Default::default())
                .unwrap();
            let commissioned = (c(m, 31), check(m, 31, Some(me), Some(rat)));
            (
                items,
                unskilled,
                skilled,
                (no_tailoring, tailor_10, tailor_50),
                failed,
                taken,
                (knight, corporal, rank_of_rat),
                uncommissioned,
                commissioned,
            )
        })
        .await
        .unwrap();
    use Truth::*;
    let (items, unskilled, skilled, greetings, failed, taken, ranks, uncommissioned, commissioned) =
        out;
    // Two carried (one on the keyring), five banked.
    assert_eq!(
        items,
        vec![Met, Unmet, Met, Unmet],
        "carried, then with the bank"
    );
    assert_eq!(unskilled, (Met, Unmet), "not known: only value 1 holds");
    assert_eq!(skilled, (Unmet, Met));
    assert_eq!(greetings, ((5422, 68, 701), 5424, 5423));
    assert_eq!(
        failed,
        vec![Unmet, Unmet, Unmet, Unmet],
        "a failed quest is neither"
    );
    assert_eq!(taken, vec![Met, Met, Unmet, Unmet], "incomplete");
    assert_eq!(
        ranks,
        (Met, Unmet, Unmet),
        "the rank shown is the number less 4"
    );
    assert_eq!(uncommissioned, Unmet);
    assert_eq!(commissioned, (Met, Unknown), "a creature has none to judge");
}

/// Where things stand: the source's area, creatures, objects and players
/// near the target, a point, reaction, object fit and loot state, a map
/// event's targets gone, the open world's instance data, an escort's
/// missing unit, a player moving backwards.
#[tokio::test]
async fn where_things_stand() {
    let mut d = data();
    cond(&mut d, 69, 4, [493, 0, 0, 0], 0);
    cond(&mut d, 70, 4, [493, 0, 0, 0], CF_REVERSE_RESULT);
    cond(&mut d, 71, 4, [12, 0, 0, 0], 0);
    // A living rat within 20 of the target, not the target; a dead one.
    cond(&mut d, 40, 20, [RAT as i32, 20, 0, 1], 0);
    cond(&mut d, 41, 20, [RAT as i32, 20, 1, 0], 0);
    // A chest within exactly no yards, and within 1.
    cond(&mut d, 42, 21, [CHEST as i32, 0, 0, 0], 0);
    cond(&mut d, 43, 21, [CHEST as i32, 1, 0, 0], 0);
    // Within 1.5 of a point with fractions.
    cond(&mut d, 44, 54, [0; 4], 0);
    d.scripts
        .condition_floats
        .insert(44, [START.0 + 0.4, START.1, START.2, 1.5]);
    // A player within 50 of the target, within 2, none within 50 (237's
    // reverse), any hostile, any friendly, a mode with no meaning.
    cond(&mut d, 45, 56, [0, 50, 0, 0], 0);
    cond(&mut d, 46, 56, [0, 2, 0, 0], 0);
    cond(&mut d, 47, 56, [0, 50, 0, 0], CF_REVERSE_RESULT);
    cond(&mut d, 48, 56, [1, 50, 0, 0], 0);
    cond(&mut d, 49, 56, [2, 50, 0, 0], 0);
    cond(&mut d, 50, 56, [7, 50, 0, 0], 0);
    // The target friendly or better to the source.
    cond(&mut d, 51, 44, [4, 1, 0, 0], 0);
    // The source's spawn.
    cond(&mut d, 52, 52, [CHEST_SPAWN as i32, 0, 0, 0], 0);
    // Spawned, its reverse, loot state ready, in use.
    cond(&mut d, 122, 48, [0; 4], 0);
    cond(&mut d, 61, 48, [0; 4], CF_REVERSE_RESULT);
    cond(&mut d, 123, 49, [1, 0, 0, 0], 0);
    cond(&mut d, 124, 49, [2, 0, 0, 0], 0);
    cond(&mut d, 53, 50, [GONE_SPAWN as i32, 61, 0, 0], 0);
    cond(&mut d, 54, 50, [GONE_SPAWN as i32, 122, 0, 0], 0);
    cond(&mut d, 55, 50, [CHEST_SPAWN as i32, 122, 0, 0], 0);
    cond(&mut d, 56, 50, [999_999, 61, 0, 0], 0);
    // Every target of map event 5 still here is alive.
    cond(&mut d, 57, 46, [0; 4], 0);
    cond(&mut d, 58, 47, [5, 57, 0, 0], 0);
    // Instance data in the open world.
    cond(&mut d, 59, 34, [1, 0, 0, 0], 0);
    // An escort fails with its target dead.
    cond(&mut d, 60, 25, [0x2, 0, 0, 0], 0);
    cond(&mut d, 62, 39, [0; 4], 0);
    let realm = harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "Cid").await;
    let out = map
        .query(move |m| {
            let rat = summon(m, RAT, 5.0, 0.0);
            let other = summon(m, RAT, 10.0, 0.0);
            let far = summon(m, RAT, 200.0, 0.0);
            let guard = summon(m, GUARD, 4.0, 4.0);
            let fiend = summon(m, FIEND, 4.0, -4.0);
            if let Some(p) = m.players.get_mut(&me) {
                p.zone = 12;
                p.area = 9;
            }
            // The rat's area once worked out; the other's not yet.
            m.creature_mut(rat).unwrap().zone_area = Some((493, 493));
            let area = (
                check(m, 69, Some(rat), Some(rat)),
                check(m, 70, Some(rat), Some(rat)),
                check(m, 69, Some(other), Some(other)),
                check(m, 71, Some(other), Some(me)),
                check(m, 71, None, Some(me)),
            );
            crate::game::map::conditions::locate(m, other, true);
            let unlocated = m.creature(other).unwrap().zone_area;
            // Around the target first; the target itself left out.
            let near = (
                check(m, 40, Some(far), Some(rat)),
                check(m, 41, Some(far), Some(rat)),
            );
            kill(m, other);
            let near_dead = (
                check(m, 40, Some(far), Some(rat)),
                check(m, 41, Some(far), Some(rat)),
                check(m, 40, Some(far), None),
            );
            let chest = crate::game::map::scripts::object_of_spawn(m, CHEST_SPAWN).unwrap();
            put(m, me, 3.5, 0.0);
            let objects = (check(m, 42, None, Some(me)), check(m, 43, None, Some(me)));
            put(m, me, 1.6, 0.0);
            let point_in = check(m, 44, None, Some(me));
            put(m, me, 2.1, 0.0);
            let point_out = check(m, 44, None, Some(me));
            put(m, me, 0.0, 0.0);
            let players: Vec<Truth> = (45..=50).map(|id| check(m, id, None, Some(rat))).collect();
            let reaction = (
                check(m, 49, None, Some(guard)),
                check(m, 48, None, Some(fiend)),
                check(m, 51, Some(guard), Some(me)),
                check(m, 51, Some(fiend), Some(me)),
                check(m, 51, Some(fiend), None),
            );
            let db_guid = (
                check(m, 52, Some(chest), Some(me)),
                check(m, 52, Some(rat), Some(me)),
            );
            let fit: Vec<Truth> = (53..=56)
                .map(|id| check(m, id, Some(me), Some(me)))
                .collect();
            let button = crate::game::map::scripts::object_of_spawn(m, BUTTON_SPAWN).unwrap();
            let ready = (
                check(m, 123, None, Some(chest)),
                check(m, 123, None, Some(button)),
                check(m, 123, None, Some(rat)),
            );
            m.gameobject_mut(chest).unwrap().loot = Some(Default::default());
            m.gameobject_mut(button)
                .unwrap()
                .fields
                .set_u32(GAMEOBJECT_STATE, 0);
            let in_use = (
                check(m, 124, None, Some(chest)),
                check(m, 124, None, Some(button)),
            );
            m.gameobject_mut(chest).unwrap().zone_area = Some((12, 87));
            let object_area = check(m, 71, Some(chest), Some(rat));
            m.map_events.active.insert(
                5,
                crate::game::map::map_events::ScriptedEvent {
                    id: 5,
                    source: me,
                    target: me,
                    expires: Instant::now() + Duration::from_secs(600),
                    success_condition: 0,
                    failure_condition: 0,
                    success_script: 0,
                    failure_script: 0,
                    data: Default::default(),
                    targets: [rat, guid::unit(RAT, 999_999)]
                        .into_iter()
                        .map(|guid| crate::game::map::map_events::EventTarget {
                            guid,
                            ..Default::default()
                        })
                        .collect(),
                },
            );
            let targets_alive = check(m, 58, Some(me), Some(me));
            kill(m, rat);
            let a_target_dead = check(m, 58, Some(me), Some(me));
            let open_world = check(m, 59, None, Some(me));
            let escort = (
                check(m, 60, Some(me), Some(guid::unit(RAT, 999_999))),
                check(m, 60, Some(me), Some(rat)),
            );
            let p = m.players.get_mut(&me).unwrap();
            p.movement.flags = p.movement.flags.clone().set_backward();
            let moving = check(m, 62, None, Some(me));
            (
                area,
                unlocated,
                near,
                near_dead,
                objects,
                (point_in, point_out),
                players,
                reaction,
                db_guid,
                fit,
                ready,
                in_use,
                object_area,
                (targets_alive, a_target_dead),
                open_world,
                escort,
                moving,
            )
        })
        .await
        .unwrap();
    use Truth::*;
    let (
        area,
        unlocated,
        near,
        near_dead,
        objects,
        point,
        players,
        reaction,
        db_guid,
        fit,
        ready,
        in_use,
        object_area,
        map_event,
        open_world,
        escort,
        moving,
    ) = out;
    assert_eq!(
        area,
        (Met, Unmet, Unknown, Met, Met),
        "the source's own; not yet known, the target's"
    );
    assert_eq!(unlocated, None, "no navmesh or terrain to say");
    assert_eq!(near, (Met, Unmet), "the other rat, not the target");
    assert_eq!(
        near_dead,
        (Unmet, Met, Met),
        "dead; around the source with no target"
    );
    assert_eq!(objects, (Unmet, Met), "the radius as it is");
    assert_eq!(point, (Met, Unmet), "a fraction of a yard counts");
    assert_eq!(
        players,
        vec![Met, Unmet, Unmet, Met, Unmet, Unknown],
        "the rat is hostile to the player"
    );
    assert_eq!(reaction.0, Met, "a player the guard is friendly to");
    assert_eq!(reaction.1, Met, "a player the fiend is hostile to");
    assert_eq!(
        (reaction.2, reaction.3, reaction.4),
        (Met, Unmet, Unknown),
        "the player toward the guard, the fiend, nobody"
    );
    assert_eq!(
        db_guid,
        (Met, Unknown),
        "an object's spawn; a summons has none"
    );
    assert_eq!(
        fit,
        vec![Met, Unmet, Met, Unmet],
        "gone, gone, there, not this map's"
    );
    assert_eq!(ready, (Met, Met, Unknown));
    assert_eq!(in_use, (Met, Met), "opened; not at rest");
    assert_eq!(object_area, Met, "an object's own zone");
    assert_eq!(map_event, (Met, Unmet), "a target gone counts as met");
    assert_eq!(open_world, Unmet, "no instance script");
    assert_eq!(escort, (Unknown, Met), "a missing unit isn't dead");
    assert_eq!(moving, Met, "backwards too");
}

/// A spell's script target with a condition: the candidate meets it.
#[tokio::test]
async fn a_spell_script_target_meets_its_condition() {
    let mut d = data();
    cond(&mut d, 6492, 41, [5, 2, 0, 0], 0);
    d.spells.script_targets.insert(SHOT, vec![(1, RAT)]);
    d.spells
        .script_target_conditions
        .insert((SHOT, 1, RAT), 6492);
    let realm = harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "Dot").await;
    let (whole, low) = map
        .query(move |m| {
            let rat = summon(m, RAT, 5.0, 0.0);
            let fits = |m: &Map| {
                crate::game::map::spells::targets::script_target(m, me, SHOT, &[1, 2], RAT, rat)
            };
            let f = m.unit_fields_mut(rat).unwrap();
            f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
            f.set_u32(UNIT_FIELD_HEALTH, 1000);
            let whole = fits(m);
            m.unit_fields_mut(rat)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 40);
            (whole, fits(m))
        })
        .await
        .unwrap();
    assert!(!whole, "not at full health");
    assert!(low, "at 4 percent");
}

/// A loot reference with a condition rolls only if the looter meets it;
/// with nobody to judge it, not at all.
#[test]
fn a_conditioned_reference_rolls_only_when_its_condition_holds() {
    use rand::SeedableRng;
    let mut d = GameData::empty();
    let entry = |item, min: i32, condition| LootEntry {
        item,
        chance: 100.0,
        group: 0,
        min,
        max: 1,
        condition,
    };
    d.loot.reference.insert(100, vec![entry(1, 1, 0)]);
    d.loot.reference.insert(200, vec![entry(2, 1, 0)]);
    let table = vec![entry(100, -100, 7), entry(200, -200, 8), entry(3, 1, 9)];
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let items = |out: Vec<crate::game::map::loot::LootSlot>| -> Vec<u32> {
        out.iter().map(|s| s.item).collect()
    };
    let mut out = Vec::new();
    crate::game::map::loot::roll_for(&d, &table, &[], &mut rng, &mut out, 0, &|c| c == 7);
    assert_eq!(
        items(out),
        vec![1, 3],
        "the first reference; the slot keeps its own"
    );
    let mut out = Vec::new();
    crate::game::map::loot::roll(&d, &table, &[], &mut rng, &mut out, 0);
    assert_eq!(items(out), vec![3]);
}

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

/// On the real world database: the Gordok Tribute's tiers, the
/// Scourgestones' commission, the heads' turn-in gates, menu 4264's
/// greeting, and the nearby-player failure conditions.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_conditions() {
    let dir = DB.as_path();
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let realm = harness::realm_with(crate::game::data::load::load(dir));
    let data = realm.data.clone();
    // Each tier of the tribute is a reference with its condition on
    // instance data field 15.
    let tiers: Vec<(u32, u32)> = data.loot.gameobject[&16577]
        .iter()
        .filter(|e| e.min < 0)
        .map(|e| (e.min.unsigned_abs(), e.condition))
        .collect();
    assert_eq!(tiers.len(), 5, "{tiers:?}");
    let tribute_items: Vec<u32> = [12008, 12015, 12016, 12017]
        .iter()
        .flat_map(|r| data.loot.reference[r].iter().map(|e| e.item))
        .collect();
    // Scourgestones drop under condition 17, the commission.
    assert!(
        data.loot
            .creature
            .values()
            .flatten()
            .any(|e| e.item == 12840 && e.condition == 17),
        "a Minion's Scourgestone gated on the commission"
    );
    let (_a, me, map) = enter(&realm, "Eve").await;
    let out = map
        .query(move |m| {
            let commission = check(m, 17, None, Some(me));
            let spell = m.realm.data.spells.get(COMMISSION).unwrap().clone();
            crate::game::map::auras::apply_spell(m, me, me, &spell, &[(0, 0)], Default::default())
                .unwrap();
            let commissioned = check(m, 17, None, Some(me));
            // Menu 4264: a tailor of 50 and up, of less, none.
            let npc = summon_any(m);
            let greet = |m: &Map| crate::game::map::npc::greeting(m, 4264, npc, me);
            let none = greet(m);
            let p = m.players.get_mut(&me).unwrap();
            crate::game::map::skills::learn_skill(p, 197, 4);
            crate::game::map::skills::raise(p, 197, 9);
            let ten = greet(m);
            let p = m.players.get_mut(&me).unwrap();
            crate::game::map::skills::raise(p, 197, 40);
            let fifty = greet(m);
            // 237 to 239, reversed: no player within 50, 60, 100 of the
            // target.
            let near: Vec<Truth> = (237..=239)
                .map(|id| check(m, id, Some(npc), Some(npc)))
                .collect();
            put(m, me, 70.0, 0.0);
            let off: Vec<Truth> = (237..=239)
                .map(|id| check(m, id, Some(npc), Some(npc)))
                .collect();
            (commission, commissioned, (none, ten, fifty), near, off)
        })
        .await
        .unwrap();
    use Truth::*;
    let (commission, commissioned, greetings, near, off) = out;
    assert_eq!((commission, commissioned), (Unmet, Met));
    assert_eq!(greetings, (5422, 5424, 5423));
    assert_eq!(near, vec![Unmet, Unmet, Unmet]);
    assert_eq!(off, vec![Met, Met, Unmet]);
    // Kalimdor: For The Horde's stake ready, and the heads not out yet, so
    // their start steps go; Nefarian's out, his doesn't; Stormwind's head
    // isn't this map's.
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let heads = kalimdor
        .query(move |m| {
            let before = (
                check(m, 4974, Some(me), Some(me)),
                check(m, 7491, Some(me), Some(me)),
                check(m, 7784, Some(me), Some(me)),
                check(m, 7782, Some(me), Some(me)),
            );
            let s = listed(m, 40150).unwrap();
            let t = m.realm.data.gameobject_templates[&s.entry].clone();
            m.add_gameobject(GameObject::from_spawn(&t, &s));
            (before, check(m, 7784, Some(me), Some(me)))
        })
        .await
        .unwrap();
    assert_eq!(
        heads,
        ((Met, Met, Met, Unmet), Unmet),
        "stake ready, heads not out, Stormwind's not here; then Nefarian's out"
    );
    // Dire Maul: the tribute's tiers by field 15.
    let dm = realm.maps.get(
        &realm,
        MapKey {
            map: 429,
            instance: 7,
        },
    );
    let rolled = dm
        .query(move |m| {
            let s = listed(m, 396_409).unwrap();
            let t = m.realm.data.gameobject_templates[&s.entry].clone();
            let mut counts = Vec::new();
            for field in [6, 3, 0] {
                m.instance.data.insert(15, field);
                let g = m.add_gameobject(GameObject::from_spawn(&t, &s));
                crate::game::map::loot::open_object(m, me, g);
                let n = m
                    .gameobject(g)
                    .and_then(|o| o.loot.as_ref())
                    .map_or(0, |l| {
                        l.items
                            .iter()
                            .filter(|i| tribute_items.contains(&i.item))
                            .count()
                    });
                counts.push(n);
                m.remove_gameobject(g);
            }
            counts
        })
        .await
        .unwrap();
    assert_eq!(rolled, vec![5, 2, 0], "a tier for each of 1600 to 1604 met");
}

fn listed(m: &Map, spawn: u32) -> Option<GameObjectSpawn> {
    m.realm
        .data
        .gameobject_spawns
        .get(&m.key.map)
        .and_then(|v| v.iter().find(|s| s.guid == spawn))
        .cloned()
}

/// A creature to talk to: any the database has.
fn summon_any(m: &mut Map) -> Guid {
    let entry = *m
        .realm
        .data
        .creature_templates
        .keys()
        .min()
        .expect("a creature");
    summon(m, entry, 3.0, 0.0)
}
