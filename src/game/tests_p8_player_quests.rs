//! Round 8's player: quests an item starts, and what taking one casts.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::Quest;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

/// Starts `FROM_LETTER`, which gives `PACKAGE`.
const LETTER: u32 = 20_400;
const PACKAGE: u32 = 20_401;
const FROM_LETTER: u32 = 9_500;
/// Wants a `PACKAGE`.
const DELIVERY: u32 = 9_501;
/// Wants the `LETTER` itself.
const KEEP_LETTER: u32 = 9_502;
/// Casts `BLESSING` on its taker.
const BLESSED: u32 = 9_503;
const BLESSING: u32 = 94_500;
/// Started by `NOTE`; casts nothing.
const NOTE: u32 = 20_402;
const PLAIN: u32 = 9_504;
/// Starts `BLESSED`.
const SCROLL: u32 = 20_403;
/// Starts `TOGETHER`, a quest every member is asked to take.
const ORDERS: u32 = 20_404;
const TOGETHER: u32 = 9_505;

fn data() -> GameData {
    let mut d = GameData::empty();
    let item = |entry: u32, starts: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable: 1,
        start_quest: starts,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    d.items.insert(LETTER, item(LETTER, FROM_LETTER));
    d.items.insert(PACKAGE, item(PACKAGE, 0));
    d.items.insert(NOTE, item(NOTE, PLAIN));
    d.items.insert(SCROLL, item(SCROLL, BLESSED));
    d.items.insert(ORDERS, item(ORDERS, TOGETHER));
    let quest = |id: u32| Quest {
        id,
        level: 1,
        min_level: 1,
        title: format!("Quest {id}"),
        ..Default::default()
    };
    d.npc.quests.insert(
        FROM_LETTER,
        Quest {
            source_item: PACKAGE,
            source_item_count: 1,
            ..quest(FROM_LETTER)
        },
    );
    let mut wants = |id: u32, entry: u32| {
        let mut q = quest(id);
        q.required_items[0] = (entry, 1);
        d.npc.quests.insert(id, q);
    };
    wants(DELIVERY, PACKAGE);
    wants(KEEP_LETTER, LETTER);
    d.npc.quests.insert(
        BLESSED,
        Quest {
            source_spell: BLESSING,
            ..quest(BLESSED)
        },
    );
    d.npc.quests.insert(PLAIN, quest(PLAIN));
    d.npc.quests.insert(
        TOGETHER,
        Quest {
            flags: crate::game::map::questshare::QUEST_FLAGS_PARTY_ACCEPT,
            ..quest(TOGETHER)
        },
    );
    let mut blessing = SpellTemplate {
        id: BLESSING,
        name: "Blessing".into(),
        range_index: 1,
        duration_index: 21,
        equipped_item_class: -1,
        ..Default::default()
    };
    blessing.effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(BLESSING, blessing);
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    (realm, a, g, map)
}

/// Give one of `entry`: its guid.
async fn give(map: &MapHandle, g: Guid, entry: u32) -> Guid {
    map.query(move |m| {
        crate::game::map::inventory::add_item(m, g, entry, 1, false).unwrap();
        let p = &m.players[&g];
        p.items
            .iter()
            .rev()
            .find(|i| i.entry == entry)
            .unwrap()
            .guid
    })
    .await
    .unwrap()
}

/// Put `quest` in the log as a giver would.
async fn take(map: &MapHandle, g: Guid, quest: u32) {
    map.query(move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .add(crate::game::map::quests::Logged {
                quest,
                ..Default::default()
            });
        crate::game::map::quests::check(m, g, quest);
    })
    .await
    .unwrap();
}

/// Accept `quest` from what `giver` is, and wait until it is in the log.
async fn accept_from(a: &Client, map: &MapHandle, g: Guid, giver: Guid, quest: u32) {
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: quest,
    })
    .await;
    for _ in 0..200 {
        if map
            .query(move |m| m.players[&g].journal.has(quest))
            .await
            .unwrap()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{quest} never taken");
}

/// Whether each of `quests` is done.
async fn done(map: &MapHandle, g: Guid, quests: &'static [u32]) -> Vec<bool> {
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = &m.players[&g];
        quests
            .iter()
            .map(|id| {
                let q = &data.npc.quests[id];
                p.journal
                    .slot_of(*id)
                    .is_some_and(|s| crate::game::map::quests::complete(p, q, &p.journal.log[s]))
            })
            .collect()
    })
    .await
    .unwrap()
}

/// Thistle Tea's item_quests_test "keeps other quests' new completion when
/// granting a source item" and "keeps other quests' lost completion when
/// consuming their objective item": a quest taken from a letter gives its
/// package and uses the letter up (mangos's `HandleQuestgiverAcceptQuest`
/// destroys a starter its quest doesn't ask for); another quest wanting
/// the package is done by it, and one wanting the letter undone by its
/// going.
#[tokio::test]
async fn an_items_quest_gives_and_takes_for_the_other_quests_too() {
    let (_realm, a, g, map) = setup().await;
    let letter = give(&map, g, LETTER).await;
    take(&map, g, DELIVERY).await;
    take(&map, g, KEEP_LETTER).await;
    assert_eq!(
        done(&map, g, &[DELIVERY, KEEP_LETTER]).await,
        [false, true],
        "the letter kept"
    );
    accept_from(&a, &map, g, letter, FROM_LETTER).await;
    let (package, letters) = map
        .query(move |m| {
            let p = &m.players[&g];
            (p.count_of(PACKAGE), p.count_of(LETTER))
        })
        .await
        .unwrap();
    assert_eq!((package, letters), (1, 0), "the package for the letter");
    assert_eq!(
        done(&map, g, &[DELIVERY, KEEP_LETTER, FROM_LETTER]).await,
        [true, false, true],
        "the delivery done, the letter's quest undone"
    );
}

/// Thistle Tea's quest_rewards_test cast_source_spell/2: taking a quest
/// with a source spell has the taker cast it on themselves (mangos's
/// `AddQuest` and its `SrcSpell`); one without casts nothing.
#[tokio::test]
async fn taking_a_quest_casts_its_source_spell_on_the_taker() {
    let (_realm, a, g, map) = setup().await;
    let note = give(&map, g, NOTE).await;
    accept_from(&a, &map, g, note, PLAIN).await;
    let auras = map
        .query(move |m| m.players[&g].auras.holders.len())
        .await
        .unwrap();
    assert_eq!(auras, 0, "nothing cast for a quest without one");
    let scroll = give(&map, g, SCROLL).await;
    accept_from(&a, &map, g, scroll, BLESSED).await;
    let blessed = map
        .query(move |m| {
            let p = &m.players[&g];
            (
                p.journal.has(BLESSED),
                p.auras
                    .holders
                    .iter()
                    .any(|h| h.spell == BLESSING && h.caster == g),
            )
        })
        .await
        .unwrap();
    assert_eq!(blessed, (true, true));
}

/// Thistle Tea's item_quests_test "prompts group members when an item
/// starts a party-accept quest": a quest every member must accept, taken
/// from an item, asks the rest of the group to take it too.
#[tokio::test]
async fn a_party_quest_from_an_item_asks_the_group() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let (realm, a, g, map) = setup().await;
    let mut a = a;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Priest).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    let orders = give(&map, g, ORDERS).await;
    accept_from(&a, &map, g, orders, TOGETHER).await;
    let (quest, by) = b
        .wait_for("SMSG_QUEST_CONFIRM_ACCEPT", |m| match m {
            ServerOpcodeMessage::SMSG_QUEST_CONFIRM_ACCEPT(c) => Some((c.quest_id, c.guid)),
            _ => None,
        })
        .await;
    assert_eq!((quest, by), (TOGETHER, g));
}
