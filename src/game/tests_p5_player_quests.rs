//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/{game_object_quests,quest_objects}_test.exs`
//! (docs/parity/matrix.csv): a quest taken from and turned in at an object,
//! by the living beside it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::Quest;
use crate::game::data::{GameData, GameObjectSpawn, GameObjectTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, death};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;
use wow_world_messages::vanilla::UpdateMask;

/// A quest giver object beside the start (its spawn), and its quest.
const BOARD: u32 = 9300;
const BOARD_SPAWN: u32 = 610;
const NOTICE: u32 = 96_600;

fn world() -> GameData {
    let mut d = GameData::empty();
    d.gameobject_templates.insert(
        BOARD,
        GameObjectTemplate {
            entry: BOARD,
            kind: 2,
            display_id: 1,
            name: "Wanted Board".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: BOARD_SPAWN,
            entry: BOARD,
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            state: 1,
            ..Default::default()
        }],
    );
    d.npc.quests.insert(
        NOTICE,
        Quest {
            id: NOTICE,
            title: "Wanted".into(),
            min_level: 1,
            level: 1,
            reward_money: 125,
            ..Default::default()
        },
    );
    d.npc.object_starts.insert(BOARD, vec![NOTICE]);
    d.npc.object_ends.insert(BOARD, vec![NOTICE]);
    d
}

fn board() -> Guid {
    crate::game::guid::game_object(BOARD, BOARD_SPAWN)
}

fn place(m: &mut Map, g: Guid, dx: f32) {
    m.players.get_mut(&g).unwrap().movement.position = Vector3d {
        x: START.0 + dx,
        y: START.1,
        z: START.2,
    };
}

/// Thistle Tea's `Quests.accept` "rejects absent, despawned, distant ...
/// sources" and "rechecks distance after presenting details", and
/// `choose_reward` "rejects a stale source and rewards while dead": a
/// quest is taken from the board only beside it, and turned in there only
/// by the living; then it pays.
#[tokio::test]
async fn an_objects_quest_is_taken_beside_it_and_turned_in_by_the_living() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(board()).await;
    let accept = async |a: &mut Client| {
        a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
            guid: board(),
            quest_id: NOTICE,
        })
        .await;
        handled(a).await;
        on(&map, move |m| m.players[&g].journal.has(NOTICE)).await
    };
    on(&map, move |m| place(m, g, 20.0)).await;
    assert!(!accept(&mut a).await, "twenty yards off");
    on(&map, move |m| place(m, g, 0.0)).await;
    assert!(accept(&mut a).await, "beside it");
    let reward = async |a: &mut Client| {
        a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
            guid: board(),
            quest_id: NOTICE,
            reward: 0,
        })
        .await;
        handled(a).await;
        on(&map, move |m| {
            let p = &m.players[&g];
            (p.journal.rewarded.contains(&NOTICE), p.money())
        })
        .await
    };
    on(&map, move |m| death::kill(m, g)).await;
    assert_eq!(reward(&mut a).await, (false, 0), "dead");
    on(&map, move |m| {
        death::resurrect(m, g, 1.0);
        place(m, g, 20.0);
    })
    .await;
    assert_eq!(reward(&mut a).await, (false, 0), "twenty yards off");
    on(&map, move |m| place(m, g, 0.0)).await;
    assert_eq!(reward(&mut a).await, (true, 125), "beside it, alive");
    let gone = on(&map, move |m| m.players[&g].journal.has(NOTICE)).await;
    assert!(!gone, "out of the log");
}

/// A goober for a quest: usable while the quest is under way, and used
/// for it; its quest, which wants it used once.
const CRATE: u32 = 9301;
const CRATE_SPAWN: u32 = 611;
const SEARCH: u32 = 96_601;

fn goober_world() -> GameData {
    let mut d = world();
    let mut t = GameObjectTemplate {
        entry: CRATE,
        kind: 10,
        display_id: 1,
        name: "Supply Crate".into(),
        size: 1.0,
        ..Default::default()
    };
    t.data[1] = SEARCH;
    d.gameobject_templates.insert(CRATE, t);
    d.gameobject_spawns
        .get_mut(&0)
        .unwrap()
        .push(GameObjectSpawn {
            guid: CRATE_SPAWN,
            entry: CRATE,
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            state: 1,
            ..Default::default()
        });
    d.npc.quests.insert(
        SEARCH,
        Quest {
            id: SEARCH,
            title: "Search the Crate".into(),
            min_level: 1,
            level: 1,
            required_kills: [(-(CRATE as i32), 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d
}

fn crate_guid() -> Guid {
    crate::game::guid::game_object(CRATE, CRATE_SPAWN)
}

/// The next `GAMEOBJECT_DYN_FLAGS` `c` is told for `object`.
async fn shown(c: &mut Client, object: Guid) -> i32 {
    c.wait_for("the object's dynamic flags", |m| {
        harness::values(m)
            .into_iter()
            .find_map(|(g, mask)| match mask {
                UpdateMask::GameObject(o) if g == object => o.gameobject_dyn_flags(),
                _ => None,
            })
    })
    .await
}

/// Thistle Tea's `QuestObjects` "tracks incomplete quests per viewer and
/// clears after completion" and "rejects dead, distant, and
/// foreign-instance users": a crate for a quest shows as usable to one on
/// it, and not again once it is done; used from afar or by the dead, it
/// credits nothing; used beside it, it does.
#[tokio::test]
async fn a_quest_object_shows_usable_to_those_on_its_quest_and_credits_the_near_and_living() {
    let realm = harness::realm_with(goober_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(crate_guid()).await;
    on(&map, move |m| {
        crate::game::tests_p3_entity::take_quest(m, g, SEARCH);
    })
    .await;
    assert_eq!(
        shown(&mut a, crate_guid()).await & 1,
        1,
        "usable, on its quest"
    );
    let used = async |a: &mut Client| {
        a.send(CMSG_GAMEOBJ_USE { guid: crate_guid() }).await;
        handled(a).await;
        on(&map, move |m| m.players[&g].journal.log[0].kills[0]).await
    };
    on(&map, move |m| place(m, g, 20.0)).await;
    assert_eq!(used(&mut a).await, 0, "twenty yards off");
    on(&map, move |m| {
        place(m, g, 0.0);
        death::kill(m, g);
    })
    .await;
    assert_eq!(used(&mut a).await, 0, "dead");
    on(&map, move |m| death::resurrect(m, g, 1.0)).await;
    assert_eq!(used(&mut a).await, 1, "beside it, alive");
    assert_eq!(shown(&mut a, crate_guid()).await & 1, 0, "done: not usable");
}
