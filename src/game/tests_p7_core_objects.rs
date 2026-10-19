//! Round 7, against Thistle Tea's `test/game/core/entity/game_object_test.exs`
//! (docs/parity/matrix.csv): what a chest and a trap carry of their
//! template and spawn, ownerless or spawned.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use crate::game::Realm;
use crate::game::data::{GameData, GameObjectSpawn, ItemTemplate, LootEntry};
use crate::game::fields::OBJECT_FIELD_CREATED_BY;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, auras, objects};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::object;

const LOCKED: u32 = 997_901;
const CHEST: u32 = 997_902;
const TRAP: u32 = 997_903;
const SPAWNED_TRAP: u32 = 997_904;
const SPAWNED_CHEST: u32 = 997_905;
const LOOT: u32 = 997_910;
const COIN: u32 = 997_911;
const RELIC: u32 = 997_912;
const QUEST: u32 = 997_920;
const MARK: u32 = 997_930;
const START: (f32, f32, f32) = crate::game::tests_p3_combat::START;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 20,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn world(d: &mut GameData) {
    // A locked chest; a quest chest (usable only so, flag 4) of three to
    // nine copper, a coin and the quest's relic; an environmental trap of
    // fifteen yards, no limit, three seconds between, four before it arms;
    // and a stealthed trap and a chest spawned, back in ninety seconds and
    // three minutes.
    let mut locked = object(LOCKED, 3, "Locked Chest", &[(0, 57), (1, LOOT)]);
    locked.gold_min = 3;
    locked.gold_max = 9;
    let mut chest = object(CHEST, 3, "Quest Chest", &[(1, LOOT)]);
    chest.flags = 4;
    chest.gold_min = 3;
    chest.gold_max = 9;
    let trap = object(TRAP, 6, "Trap", &[(2, 15), (3, MARK), (5, 3), (7, 4)]);
    let hidden = object(
        SPAWNED_TRAP,
        6,
        "Hidden Trap",
        &[(0, 12), (2, 5), (3, MARK), (9, 1)],
    );
    let spawned = object(SPAWNED_CHEST, 3, "Bucket", &[(1, LOOT)]);
    for t in [locked, chest, trap, hidden, spawned] {
        d.gameobject_templates.insert(t.entry, t);
    }
    let spawn = |guid, entry, respawn_min, dx: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min,
        state: 1,
        ..Default::default()
    };
    d.gameobject_spawns.entry(0).or_default().extend([
        spawn(7_001, SPAWNED_TRAP, 90, 30.0),
        spawn(7_002, SPAWNED_CHEST, 180, 2.0),
    ]);
    let drop = |item, chance| LootEntry {
        item,
        chance,
        min: 1,
        max: 1,
        ..Default::default()
    };
    d.loot
        .gameobject
        .insert(LOOT, vec![drop(COIN, 100.0), drop(RELIC, -100.0)]);
    d.items.insert(COIN, item(COIN));
    d.items.insert(RELIC, item(RELIC));
    d.npc.quests.insert(
        QUEST,
        crate::game::data::npc::Quest {
            id: QUEST,
            title: "The Relic".into(),
            min_level: 1,
            level: 1,
            required_items: [(RELIC, 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    let mut mark = bare(MARK, 0);
    mark.duration_index = LASTS_LONG;
    effect(&mut mark, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
    insert(d, [mark]);
}

/// A realm of [`world`], with no GMs (whose hands open locks).
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data_with(world), None)
}

/// The loot window `c` is shown: its gold and its items.
async fn window(c: &mut Client) -> (u32, Vec<u32>) {
    let b = c
        .wait_frame(
            "SMSG_LOOT_RESPONSE",
            crate::game::map::loot::SMSG_LOOT_RESPONSE,
            |_| true,
        )
        .await;
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let items = (0..usize::from(b[13]))
        .map(|n| u32_at(14 + n * 22 + 1))
        .collect();
    (u32_at(9), items)
}

/// The next `GAMEOBJECT_DYN_FLAGS` `c` is told for `object`, if within
/// `within`.
async fn dyn_flags(c: &mut Client, object: Guid, within: Duration) -> Option<i32> {
    c.gets(within, |m| {
        harness::values(m)
            .into_iter()
            .find_map(|(g, mask)| match mask {
                UpdateMask::GameObject(o) if g == object => o.gameobject_dyn_flags(),
                _ => None,
            })
    })
    .await
}

fn here(m: &Map, g: Guid) -> Vector3d {
    let p = m.players[&g].position();
    Vector3d { x: p.x + 2.0, ..p }
}

/// Thistle Tea's `build_summoned/4` "ownerless chests retain loot, lock,
/// gold and activation fields": chests summoned by no one (a script's, a
/// wild object's) are no one's; a locked one opens to no hand; an open
/// one's window has its table's loot and three to nine copper; and a
/// quest chest shows as usable to one whose quest wants what it holds,
/// not before (mangos's `ActivateToQuest` for a chest).
#[tokio::test]
async fn an_ownerless_chest_keeps_its_loot_gold_and_lock_and_a_quest_chest_activates_for_its_quest()
{
    chests().await;
}

/// The quest chest's flag, and its window, pass the 1.12 client's own
/// parsers (`tools/client_check`).
#[tokio::test]
#[ignore = "extended"]
async fn a_quest_chests_flag_passes_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let (check_py, wow_exe) = checker().expect("the client checker");
    let mut a = chests().await;
    let (frames, _) = capture(vec![&mut a]);
    run_checker(&check_py, &wow_exe, &frames);
}

async fn chests() -> Client {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = world_map(&realm);
    let (locked, chest) = on(&map, move |m| {
        let at = here(m, g);
        (
            objects::summon(m, None, LOCKED, at, 0.0, 0, 0, 0, None).unwrap(),
            objects::summon(m, None, CHEST, at, 0.0, 0, 0, 0, None).unwrap(),
        )
    })
    .await;
    a.sees(locked).await;
    a.sees(chest).await;
    let owners = on(&map, move |m| {
        [locked, chest].map(|o| {
            m.gameobject(o)
                .unwrap()
                .fields
                .get_guid(OBJECT_FIELD_CREATED_BY)
        })
    })
    .await;
    assert_eq!(owners, [Guid::zero(); 2], "no one's");
    assert_eq!(
        dyn_flags(&mut a, chest, Duration::from_millis(1_500))
            .await
            .unwrap_or(0)
            & 1,
        0,
        "not usable off its quest"
    );
    a.clear();
    a.send(CMSG_GAMEOBJ_USE { guid: locked }).await;
    assert!(
        !crate::game::tests_p3_entity::gets_frame(&mut a, 0x160, |_| true).await,
        "the lock holds against a hand"
    );
    on(&map, move |m| {
        crate::game::tests_p3_entity::take_quest(m, g, QUEST)
    })
    .await;
    assert_eq!(
        dyn_flags(&mut a, chest, harness::WAIT).await.map(|f| f & 1),
        Some(1),
        "usable on its quest"
    );
    a.send(CMSG_GAMEOBJ_USE { guid: chest }).await;
    let (gold, mut items) = window(&mut a).await;
    items.sort_unstable();
    assert!((3..=9).contains(&gold), "{gold} copper");
    assert_eq!(items, vec![COIN, RELIC]);
    a
}

fn world_map(realm: &Arc<Realm>) -> crate::game::map::MapHandle {
    realm.maps.get(realm, crate::game::map::MapKey::world(0))
}

/// Look about the traps now.
fn look(m: &mut Map) {
    m.objects.next_trap_look = None;
    objects::update(m);
}

/// Thistle Tea's `build_summoned/4` "environmental traps keep their
/// radius, repeat count and cooldown" and `build/2` "static traps retain
/// their spawn's respawn delay and stealth flag": a trap summoned by no one
/// springs on a player fifteen yards off (its radius) only once its four
/// seconds' start are up, again and again (no limit) but three seconds
/// apart; a stealthed trap spawned hides from a player it is hostile to,
/// and disarmed it comes back at its spawn's ninety seconds.
#[tokio::test]
async fn an_ownerless_trap_keeps_its_radius_start_and_cooldown_and_a_spawned_one_its_stealth_and_respawn()
 {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = world_map(&realm);
    let out = on(&map, move |m| {
        let p = m.players[&g].position();
        let at = Vector3d { x: p.x + 14.0, ..p };
        let trap = objects::summon(m, None, TRAP, at, 0.0, 0, 0, 0, None).unwrap();
        let marks = |m: &Map| {
            auras::auras(m, g).map_or(0, |a| a.holders.iter().filter(|h| h.spell == MARK).count())
        };
        let mut seen = Vec::new();
        for secs in [0, 3, 2, 2, 2] {
            m.now += Duration::from_secs(secs);
            look(m);
            seen.push(marks(m));
            auras::remove_spell(m, g, MARK, None);
        }
        let still = m.gameobject(trap).is_some();
        (seen, still)
    })
    .await;
    assert_eq!(
        out,
        (vec![0, 0, 1, 0, 1], true),
        "armed after four seconds, then every three, with no limit"
    );
    // The spawned trap: stealthed, against Alice.
    let hidden = crate::game::guid::game_object(SPAWNED_TRAP, 7_001);
    let out = on(&map, move |m| {
        let seen = objects::seen_by(m, g, hidden);
        let lock = m
            .realm
            .data
            .gameobject_templates
            .get(&SPAWNED_TRAP)
            .map(objects::lock_of);
        objects::disarm(m, hidden);
        let back = m
            .objects
            .respawns
            .iter()
            .find(|(_, s)| *s == 7_001)
            .map(|(t, _)| (*t - m.now).as_secs());
        (seen, lock, m.gameobject(hidden).is_some(), back)
    })
    .await;
    assert_eq!(
        out,
        (false, Some(12), false, Some(90)),
        "hidden, locked, disarmed and gone for its spawn's ninety seconds"
    );
}

/// Thistle Tea's `build/2 chests` "carries loot config, respawn delay, and
/// the activate dynamic flag": a spawned chest gives its table's loot, and
/// emptied it goes for its spawn's three minutes (its quest flag is
/// `an_ownerless_chest_keeps_its_loot_gold_and_lock_and_a_quest_chest_activates_for_its_quest`'s).
#[tokio::test]
async fn a_spawned_chest_gives_its_loot_and_comes_back_at_its_spawns_time() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    let _ = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = world_map(&realm);
    let chest = crate::game::guid::game_object(SPAWNED_CHEST, 7_002);
    a.sees(chest).await;
    a.send(CMSG_GAMEOBJ_USE { guid: chest }).await;
    let (_, items) = window(&mut a).await;
    assert_eq!(items, vec![COIN], "its loot, off the quest");
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: chest }).await;
    crate::game::tests_p3_entity_objects::until(&map, "the chest gone", move |m| {
        m.gameobject(chest).is_none()
    })
    .await;
    let back = on(&map, move |m| {
        m.loot_state
            .respawns
            .iter()
            .find(|(_, s)| *s == 7_002)
            .map(|(t, _)| (*t - m.now).as_secs())
    })
    .await;
    assert!(back.is_some_and(|s| (178..=180).contains(&s)), "{back:?}");
}
