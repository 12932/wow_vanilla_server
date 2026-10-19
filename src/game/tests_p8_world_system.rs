//! Round 8's world tests, ported from Thistle Tea's
//! `test/game/world/system/` (docs/parity/matrix.csv): creature groups a
//! script breaks up.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::scripts::{GROUP_AGGRO_TOGETHER, GROUP_FORMATION, GroupMember, ScriptStep};
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::creature_groups::{self, members_of};
use crate::game::map::scripts;
use crate::game::tests_p3_entity::step;
use crate::game::tests_p4_items_quests::{START, player, realm_of, spawn, template, world};

const LEADER: u32 = 997_850;
const FIRST: u32 = 997_851;
const SECOND: u32 = 997_852;

/// A database group: its leader, and two members in formation behind it
/// that fight together.
fn group_world() -> GameData {
    let mut d = world();
    for (i, entry) in [LEADER, FIRST, SECOND].into_iter().enumerate() {
        d.creature_templates
            .insert(entry, template(entry, &format!("Walker {i}")));
        spawn(
            &mut d,
            0,
            entry,
            (START.0 + 10.0 + 3.0 * i as f32, START.1, START.2),
        );
    }
    let member = |guid, distance| GroupMember {
        guid,
        distance,
        angle: std::f32::consts::PI,
        flags: GROUP_FORMATION | GROUP_AGGRO_TOGETHER,
    };
    d.scripts.groups.insert(
        LEADER,
        vec![member(LEADER, 0.0), member(FIRST, 3.0), member(SECOND, 6.0)],
    );
    for g in [LEADER, FIRST, SECOND] {
        d.scripts.group_of.insert(g, LEADER);
    }
    d
}

fn creature(entry: u32) -> Guid {
    guid::unit(entry, entry)
}

/// Where `g` stands in its group: its members here, whether it keeps a
/// place in a formation, and its place there.
fn standing(m: &Map, g: Guid) -> (Vec<Guid>, bool, bool) {
    (
        members_of(m, g),
        creature_groups::following(m, g),
        creature_groups::formation_home(m, g).is_some(),
    )
}

/// Thistle Tea's `CreatureGroups.leave/4` "removal survives reload and
/// invalidates queued commands" and "the original leader disbands every
/// member", and `formation/3` "leaving and disbanding remove formation
/// snapshots": a member's script leaving its database group (79) takes it
/// out, its place in the formation gone, the rest of the group still
/// together; dead and back, it is still out; a script's join (78) puts it
/// by its leader again. The leader's own leaving disbands the group: no
/// member is of it, nor keeps a place behind it, and the one it took back
/// is let go too (vmangos's `SCRIPT_COMMAND_LEAVE_CREATURE_GROUP` by
/// `DisbandGroup`). Justin's walk in Stormwind disbands his so.
#[tokio::test]
async fn a_member_leaves_its_group_and_its_leader_leaving_disbands_it() {
    let realm = realm_of(group_world());
    let (mut a, ga, map) = player(&realm, "Alice").await;
    let (leader, first, second) = (creature(LEADER), creature(FIRST), creature(SECOND));
    a.sees(second).await;
    let out = map
        .query(move |m| {
            let together = standing(m, first);
            let leave = step(79, [0; 4]);
            assert!(scripts::execute(m, &leave, first, first));
            let left = (standing(m, first), standing(m, second));
            crate::game::map::combat::kill(m, ga, first);
            crate::game::map::ai::respawn_now(m, first);
            let back = (
                m.creature(first).is_some_and(|c| c.alive()),
                standing(m, first),
            );
            let join = ScriptStep {
                x: 3.0,
                ..step(78, [GROUP_FORMATION, 0, 0, 0])
            };
            assert!(scripts::execute(m, &join, first, leader));
            let rejoined = members_of(m, first);
            assert!(scripts::execute(m, &leave, leader, leader));
            let disbanded = (
                standing(m, leader).0,
                standing(m, first),
                standing(m, second),
                creature_groups::is_leader(m, leader),
            );
            (together, left, back, rejoined, disbanded)
        })
        .await
        .unwrap();
    let (together, left, back, rejoined, disbanded) = out;
    assert_eq!(
        together,
        (vec![leader, first, second], true, true),
        "in its group and formation"
    );
    assert_eq!(
        left,
        ((vec![], false, false), (vec![leader, second], true, true)),
        "out, the rest together"
    );
    assert_eq!(back, (true, (vec![], false, false)), "back and still out");
    assert_eq!(rejoined, vec![leader, first], "joined again by a script");
    assert_eq!(
        disbanded,
        (
            vec![],
            (vec![], false, false),
            (vec![], false, false),
            false
        ),
        "the leader's leaving disbands it"
    );
}

const TRAP: u32 = 997_855;
const TRAP_SPAWN: u32 = 7_855;
const MARK: u32 = 997_856;

/// Thistle Tea's `SpawnPool` "disarmed and triggered static traps respawn
/// through their pool with fresh state": a trap spawned thirty yards off,
/// sprung once (one charge) and then gone, disarmed, goes for its spawn's
/// minute and comes back armed; sprung by Alice beside it, it marks her
/// and goes, and back after its minute it is armed again, its charge
/// whole, and springs on her once more (mangos-classic's
/// `GameObject::Update` for a trap: used up, it goes and comes back at its
/// spawn's time, as new).
#[tokio::test]
async fn a_spawned_trap_disarmed_or_sprung_comes_back_armed() {
    use crate::game::data::GameObjectSpawn;
    use crate::game::map::{auras, objects};
    use crate::game::tests_p3_cast_kit::{LASTS_LONG, aura, bare, data_with, effect, insert};
    use wow_world_messages::vanilla::Vector3d;
    let mut d = data_with(|d| {
        let mut mark = bare(MARK, 0);
        mark.duration_index = LASTS_LONG;
        effect(&mut mark, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, [mark]);
    });
    d.faction_templates.extend(world().faction_templates);
    d.gameobject_templates.insert(
        TRAP,
        crate::game::tests_p3_entity_objects::object(TRAP, 6, "Trap", &[(2, 3), (3, MARK), (4, 1)]),
    );
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(GameObjectSpawn {
            guid: TRAP_SPAWN,
            entry: TRAP,
            map: 0,
            x: START.0 + 30.0,
            y: START.1,
            z: START.2,
            respawn_min: 60,
            state: 1,
            ..Default::default()
        });
    let realm = realm_of(d);
    let (_a, ga, map) = player(&realm, "Alice").await;
    let trap = guid::game_object(TRAP, TRAP_SPAWN);
    let out = map
        .query(move |m| {
            let look = |m: &mut Map| {
                m.objects.next_trap_look = None;
                objects::update(m);
            };
            let armed = |m: &Map| {
                m.gameobject(trap).is_some()
                    && m.objects.traps.get(&trap).is_none_or(|t| !t.depleted)
            };
            let marks = |m: &Map| {
                auras::auras(m, ga)
                    .map_or(0, |a| a.holders.iter().filter(|h| h.spell == MARK).count())
            };
            look(m);
            let first = armed(m);
            objects::disarm(m, trap);
            let disarmed = m.gameobject(trap).is_some();
            m.now += std::time::Duration::from_secs(61);
            look(m);
            let back = armed(m);
            // She stands on it.
            crate::game::map::movement::teleport_within(
                m,
                ga,
                Vector3d {
                    x: START.0 + 30.0,
                    y: START.1,
                    z: START.2,
                },
                0.0,
            );
            look(m);
            let sprung = (marks(m), m.gameobject(trap).is_some());
            auras::remove_spell(m, ga, MARK, None);
            // Back, it springs on her at once, its one charge whole.
            m.now += std::time::Duration::from_secs(61);
            look(m);
            let again = (marks(m), m.gameobject(trap).is_some());
            (first, disarmed, back, sprung, again)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (true, false, true, (1, false), (1, false)),
        "armed; disarmed, gone; back armed; sprung once, gone; back, and springs again"
    );
}

const STONE: u32 = 997_857;
const STONE_SPAWN: u32 = 7_857;

/// A world of a stone spawned beside where Alice comes in, back five
/// minutes after it goes, with `steps` for its `gameobject_scripts`.
fn stone_world(steps: Vec<ScriptStep>) -> GameData {
    let mut d = world();
    d.gameobject_templates.insert(
        STONE,
        crate::game::tests_p3_entity_objects::object(STONE, 5, "Stone", &[]),
    );
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(crate::game::data::GameObjectSpawn {
            guid: STONE_SPAWN,
            entry: STONE,
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            respawn_min: 300,
            state: 1,
            ..Default::default()
        });
    d.scripts.gameobject.insert(STONE_SPAWN, steps);
    d
}

/// Thistle Tea's `SpawnPool` "executes immediate and delayed scripts
/// through the game object owner": a stone's script, its step at once
/// opening it (state 0) and its step 200 ms on setting it to 2, runs as
/// Alice uses it, the first at once and the second after its time
/// (vmangos's `gameobject_scripts`).
#[tokio::test]
async fn an_objects_script_runs_its_steps_at_once_and_in_their_time() {
    let set = |state: u32, delay_ms: u32| ScriptStep {
        delay_ms,
        ..step(80, [state, 0, 0, 0])
    };
    let realm = realm_of(stone_world(vec![set(0, 0), set(2, 200)]));
    let (_a, ga, map) = player(&realm, "Alice").await;
    let stone = guid::game_object(STONE, STONE_SPAWN);
    let out = map
        .query(move |m| {
            use crate::game::fields::GAMEOBJECT_STATE;
            let state = |m: &Map| {
                m.gameobject(stone)
                    .unwrap()
                    .fields
                    .get_u32(GAMEOBJECT_STATE)
            };
            let before = state(m);
            scripts::object_scripts(m, ga, stone);
            scripts::update(m);
            let at_once = state(m);
            m.now += std::time::Duration::from_millis(100);
            scripts::update(m);
            let early = state(m);
            m.now += std::time::Duration::from_millis(150);
            scripts::update(m);
            (before, at_once, early, state(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (1, 0, 0, 2), "ready; opened at once; 2 after 200 ms");
}

/// Thistle Tea's `SpawnPool` "temporarily respawns a suspended game
/// object" and "suspends a game object by its database guid": a script
/// removes the stone, named by its database guid (41, at an object of a
/// spawn, 14), and it stays gone past its spawn's five minutes; a
/// script's respawn of it (9) brings it out for its ten seconds, after
/// which it goes again and stays gone (vmangos's
/// `SCRIPT_COMMAND_REMOVE_OBJECT` and `SCRIPT_COMMAND_RESPAWN_GAMEOBJECT`).
#[tokio::test]
async fn a_removed_object_stays_gone_but_for_a_scripts_while() {
    let realm = realm_of(stone_world(Vec::new()));
    let (_a, ga, map) = player(&realm, "Alice").await;
    let stone = guid::game_object(STONE, STONE_SPAWN);
    let out = map
        .query(move |m| {
            let here = |m: &Map| m.gameobject(stone).is_some();
            let wait = |m: &mut Map, secs: u64| {
                m.now += std::time::Duration::from_secs(secs);
                scripts::update(m);
                scripts::update_objects(m);
                crate::game::map::objects::update(m);
            };
            let first = here(m);
            let remove = ScriptStep {
                target_type: 14,
                target_param1: STONE_SPAWN,
                ..step(41, [0; 4])
            };
            assert!(scripts::execute(m, &remove, ga, ga));
            let removed = here(m);
            wait(m, 301);
            let past_its_time = here(m);
            assert!(scripts::execute(
                m,
                &step(9, [STONE_SPAWN, 10, 0, 0]),
                ga,
                ga
            ));
            let out_for_a_while = here(m);
            wait(m, 5);
            let still = here(m);
            wait(m, 6);
            let gone = here(m);
            wait(m, 301);
            (
                first,
                removed,
                past_its_time,
                out_for_a_while,
                still,
                gone,
                here(m),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (true, false, false, true, true, false, false),
        "there; removed, and not back at its time; out for ten seconds; gone again for good"
    );
}

/// Thistle Tea's `World.players_in_zone/2` "finds the players whose area
/// lies in the zone": a script for everyone in Elwynn Forest (92) runs on
/// Alice in the forest and on Bob in Goldshire, an area of it, and not on
/// Carol in Westfall (the script command for everyone in a zone, by the
/// zone each player is in, not their area).
#[tokio::test]
async fn a_script_for_a_zone_runs_on_those_in_its_areas_only() {
    use crate::game::fields::UNIT_NPC_EMOTESTATE;
    const ZONE_SCRIPT: u32 = 997_858;
    const ELWYNN: u32 = 12;
    const GOLDSHIRE: u32 = 87;
    const WESTFALL: u32 = 40;
    let mut d = world();
    d.scripts.generic.insert(
        ZONE_SCRIPT,
        vec![step(2, [u32::from(UNIT_NPC_EMOTESTATE), 7, 0, 0])],
    );
    let realm = realm_of(d);
    let (_a, ga, map) = player(&realm, "Alice").await;
    let (_b, gb, _) = player(&realm, "Bob").await;
    let (_c, gc, _) = player(&realm, "Carol").await;
    let out = map
        .query(move |m| {
            for (g, zone, area) in [
                (ga, ELWYNN, ELWYNN),
                (gb, ELWYNN, GOLDSHIRE),
                (gc, WESTFALL, WESTFALL),
            ] {
                let p = m.players.get_mut(&g).unwrap();
                p.zone = zone;
                p.area = area;
            }
            assert!(scripts::execute(
                m,
                &step(92, [ZONE_SCRIPT, ELWYNN, 0, 0]),
                ga,
                ga
            ));
            scripts::update(m);
            [ga, gb, gc].map(|g| m.players[&g].fields.get_u32(UNIT_NPC_EMOTESTATE))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [7, 7, 0],
        "the forest's and Goldshire's, not Westfall's"
    );
}

/// Thistle Tea's `VendorStock.list/4` "isolates merchant spawns and world
/// copies and clears only the expired copy": two clerks of one entry sell
/// a limited ware, six of it each; Alice's buying three from one leaves
/// it three, the other its six (a stock is the vendor's own, by its guid;
/// a copy's vendors are its map's, gone with it).
#[tokio::test]
async fn a_vendors_limited_stock_is_its_own() {
    use crate::game::data::{CreatureTemplate, ItemTemplate, VendorItem};
    use crate::game::map::vendor;
    use crate::game::tests_p3_combat::{HUMANS, data, setup_with, spawn, template};
    const CLERK: u32 = 997_860;
    const WARE: u32 = 997_861;
    let mut d = data();
    d.creature_templates.insert(
        CLERK,
        CreatureTemplate {
            faction: HUMANS,
            npc_flags: vendor::NPC_FLAG_VENDOR,
            ..template(CLERK, 7)
        },
    );
    d.items.insert(
        WARE,
        ItemTemplate {
            entry: WARE,
            name: "Ware".into(),
            display_id: 1000,
            stackable: 5,
            allowable_class: -1,
            allowable_race: -1,
            buy_count: 3,
            buy_price: 2,
            ..Default::default()
        },
    );
    d.vendors.insert(
        CLERK,
        vec![VendorItem {
            item: WARE,
            max_count: 6,
            restock_secs: 600,
            ..Default::default()
        }],
    );
    let (_realm, _a, ga, map) = setup_with(d, wow_world_messages::vanilla::Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().set_money(100);
            let (one, other) = (spawn(m, ga, CLERK, 20, 2.0), spawn(m, ga, CLERK, 20, 3.0));
            let v = m.realm.data.vendor_items(CLERK)[0];
            vendor::buy(m, ga, one, WARE, 1);
            (
                vendor::stock_left(m, one, &v, 3),
                vendor::stock_left(m, other, &v, 3),
                m.players[&ga].count_of(WARE),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (3, 6, 3), "three left of hers, six of the other's");
}

/// Thistle Tea's `Battleground.EffectSink.emit/2` "keeps the offline
/// carrier's name after live metadata disappears": Alice takes the Horde's
/// flag and goes offline with it; it drops where she stood, and Grom is
/// told it was dropped by her, by name.
#[tokio::test]
async fn a_carrier_gone_offline_drops_the_flag_and_is_named_for_it() {
    use crate::game::map::battleground::Flag;
    use crate::game::tests_bg5::{alice_takes_the_horde_flag, horde_flag, said};
    let (a, mut b, map, _alice) = alice_takes_the_horde_flag().await;
    let mut carried = false;
    for _ in 0..200 {
        carried = matches!(
            map.query(|m| horde_flag(m)).await.flatten(),
            Some(Flag::Carried(_))
        );
        if carried {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(carried, "she carries it");
    b.clear();
    a.disconnect();
    let line = said(&mut b, "dropped").await;
    assert_eq!(line, "The Horde Flag was dropped by Alice!");
    assert!(matches!(
        map.query(|m| horde_flag(m)).await.flatten(),
        Some(Flag::Dropped(_))
    ));
}
