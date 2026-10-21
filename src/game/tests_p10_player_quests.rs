//! Round 10's player and quests: progress before the item's create, a
//! battleground NPC's menu by the quest hello, a mount off at an object, a
//! reward cast by its giver, a flag quest's reward, a party quest in a
//! raid (Thistle Tea's `quest_item_progress_test`, `quest_turn_in_test`,
//! `game_object_quests_test`, `quest_rewards_test` and
//! `quest_sharing_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::Quest;
use crate::game::data::spells::SpellEffect;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, auras, inventory, objects, quests};
use crate::game::store::Logged;
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with};
use crate::game::tests_p3_entity::{NONE_WITHIN, aura_effect, on, spell};
use crate::game::tests_p3_entity_objects::object;

const QUEST: u32 = 96_900;
const FLAG_QUEST: u32 = 96_901;
const LOW_FLAG_QUEST: u32 = 96_902;
const PLAIN_QUEST: u32 = 96_903;
const LESSON_QUEST: u32 = 96_904;
const ESCORT: u32 = 96_905;
const RAID_ESCORT: u32 = 96_906;
const THING: u32 = 63_960;
const PRIZE: u32 = 63_961;
const GIVER: u32 = 3_975;
const BOARD: u32 = 98_900;
const MOUNTED_BOARD: u32 = 98_901;
const MOUNT: u32 = 97_970;
const LESSON: u32 = 97_971;
const TAUGHT: u32 = 97_972;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        stackable: 20,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn quest(id: u32) -> Quest {
    Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = data();
    d.items.insert(THING, item(THING));
    d.items.insert(PRIZE, item(PRIZE));
    let mut q = quest(QUEST);
    q.required_items[0] = (THING, 2);
    d.npc.quests.insert(QUEST, q);
    for (id, min_level) in [(FLAG_QUEST, 1), (LOW_FLAG_QUEST, 61)] {
        let mut q = quest(id);
        q.flags = quests::QUEST_FLAGS_AUTO_REWARDED;
        q.min_level = min_level;
        q.reward_money = 123;
        q.rewards[0] = (PRIZE, 1);
        d.npc.quests.insert(id, q);
    }
    d.npc.quests.insert(PLAIN_QUEST, quest(PLAIN_QUEST));
    let mut q = quest(LESSON_QUEST);
    q.reward_spell_cast = LESSON;
    d.npc.quests.insert(LESSON_QUEST, q);
    for (id, kind) in [(ESCORT, 0), (RAID_ESCORT, 62)] {
        let mut q = quest(id);
        q.flags = crate::game::map::questshare::QUEST_FLAGS_PARTY_ACCEPT;
        q.kind = kind;
        d.npc.quests.insert(id, q);
    }
    let mut giver = d.creature_templates[&BRUTE].clone();
    giver.entry = GIVER;
    giver.npc_flags = crate::game::map::npc::NPC_FLAG_QUESTGIVER;
    giver.faction = 35;
    d.creature_templates.insert(GIVER, giver);
    d.npc
        .creature_starts
        .insert(GIVER, vec![ESCORT, RAID_ESCORT]);
    d.npc.creature_ends.insert(GIVER, vec![LESSON_QUEST]);
    d.npc.object_ends.insert(BOARD, vec![LESSON_QUEST]);
    d.gameobject_templates
        .insert(BOARD, object(BOARD, 2, "Board", &[]));
    d.gameobject_templates.insert(
        MOUNTED_BOARD,
        object(MOUNTED_BOARD, 2, "Mounted board", &[(8, 1)]),
    );
    d.spells.spells.insert(
        MOUNT,
        spell(MOUNT, &[aura_effect(auras::AURA_MOUNTED, 0, 2402, 1)]),
    );
    d.spells.spells.insert(
        LESSON,
        spell(
            LESSON,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEARN_SPELL,
                trigger_spell: TAUGHT,
                ..Default::default()
            }],
        ),
    );
    d.spells.spells.insert(
        TAUGHT,
        spell(TAUGHT, &[aura_effect(auras::AURA_DUMMY, 1, 0, 1)]),
    );
    d
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

fn mounted(m: &Map, g: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOUNTED))
}

fn take(m: &mut Map, g: Guid, quest: u32) {
    m.players.get_mut(&g).unwrap().journal.add(Logged {
        quest,
        ..Default::default()
    });
}

/// Thistle Tea's "the progress packet precedes the item's create block"
/// and "a planned batch commits new items before projecting its packets",
/// as mangos's `StoreNewItem` sends them (`ItemAddedQuestCheck`, then the
/// item's create): an item a quest wants, received, is told as progress
/// first, then created.
#[tokio::test]
async fn a_quest_items_progress_comes_before_its_create() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| take(m, g, QUEST)).await;
    a.clear();
    on(&map, move |m| {
        inventory::add_item(m, g, THING, 1, true).unwrap();
    })
    .await;
    let first = a
        .wait_for("the progress or the create", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_ITEM(u) if u.required_item_id == THING => {
                Some("progress")
            }
            _ if harness::creates(m)
                .iter()
                .any(|c| guid::high(*c) == Some(guid::HighGuid::Item)) =>
            {
                Some("create")
            }
            _ => None,
        })
        .await;
    assert_eq!(first, "progress");
    a.wait_for("the create", |m| {
        harness::creates(m)
            .iter()
            .any(|c| guid::high(*c) == Some(guid::HighGuid::Item))
            .then_some(())
    })
    .await;
}

/// Thistle Tea's "questgiver #{entry} exposes match gossip through the
/// quest hello path" and "quest #{quest_id} credits the match after
/// inventory commit and ignores duplicate rewards": Alterac Valley's smith
/// answers a quest giver's hello with its menu (the armor's status), as
/// mangos's `HandleQuestgiverHelloOpcode` tries the script's gossip first;
/// a turn-in of armor scraps by the reward's choice counts twenty scraps
/// once the items are taken, and a second choice counts nothing.
#[tokio::test]
async fn alterac_valleys_smith_answers_a_quest_hello_and_counts_a_turn_in_once() {
    let mut d = crate::game::tests_pvp2::valley();
    d.items.insert(THING, item(THING));
    let mut q = quest(7223);
    q.required_items[0] = (THING, 1);
    d.npc.quests.insert(7223, q);
    d.npc.creature_ends.insert(13257, vec![7223]);
    d.creature_templates.get_mut(&13257).unwrap().npc_flags |=
        crate::game::map::npc::NPC_FLAG_QUESTGIVER;
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = crate::game::tests_pvp::both_into(
        &realm,
        &mut a,
        &mut b,
        wow_world_base::vanilla::Map::AlteracValley,
    )
    .await;
    let smith = map
        .query(|m| crate::game::tests_pvp2::creature(m, 13257))
        .await
        .flatten()
        .unwrap();
    on(&map, move |m| {
        let at = m.creature(smith).unwrap().position;
        m.players.get_mut(&g).unwrap().movement.position = at;
    })
    .await;
    a.send(CMSG_QUESTGIVER_HELLO { guid: smith }).await;
    let menu = a
        .wait_for("the smith's menu", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(gm) if gm.guid == smith => {
                Some((gm.title_text_id, gm.gossips.len()))
            }
            _ => None,
        })
        .await;
    assert_eq!(menu, (6073, 1), "the armor's status");
    let scraps = |m: &Map| {
        m.battleground
            .as_ref()
            .unwrap()
            .av
            .as_ref()
            .unwrap()
            .extras
            .armor[0]
            .scraps
    };
    let choose = CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: smith,
        quest_id: 7223,
        reward: 0,
    };
    // Without the scraps: nothing counted.
    on(&map, move |m| take(m, g, 7223)).await;
    a.send(choose).await;
    harness::an_update(&map).await;
    assert_eq!(on(&map, move |m| scraps(m)).await, 0, "nothing turned in");
    on(&map, move |m| {
        inventory::add_item(m, g, THING, 2, true).unwrap();
    })
    .await;
    a.send(choose).await;
    harness::an_update(&map).await;
    let (counted, left) = on(&map, move |m| (scraps(m), m.players[&g].count_of(THING))).await;
    assert_eq!((counted, left), (20, 1), "twenty, the scraps taken");
    a.send(choose).await;
    harness::an_update(&map).await;
    assert_eq!(on(&map, move |m| scraps(m)).await, 20, "once");
}

/// Thistle Tea's "dismounts through the shared aura lifecycle unless the
/// template permits mounts", as mangos's `GameObject::Use` has it: a
/// mounted player who uses a quest board comes off the mount; one whose
/// template allows mounts (its data 8) leaves them on it.
#[tokio::test]
async fn using_an_object_dismounts_unless_its_template_allows_mounts() {
    let (_realm, a, g, map) = setup_with(world(), Class::Warrior).await;
    for (entry, stays) in [(MOUNTED_BOARD, true), (BOARD, false)] {
        let board = on(&map, move |m| {
            crate::game::map::spells::cast_triggered(m, g, MOUNT, Some(g));
            assert!(mounted(m, g), "mounted");
            objects::summon(m, None, entry, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap()
        })
        .await;
        a.send(CMSG_GAMEOBJ_USE { guid: board }).await;
        harness::an_update(&map).await;
        let mounted = on(&map, move |m| mounted(m, g)).await;
        assert_eq!(mounted, stays, "{entry}");
    }
}

/// Thistle Tea's "routes teaching item creation and friendly-target
/// rewards through the questgiver", as mangos's `RewardQuest` picks the
/// caster: a lesson rewarded by a creature is cast by it on the player;
/// by a quest board, by the player.
#[tokio::test]
async fn a_lesson_rewarded_is_cast_by_its_creature_giver() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let (npc, board) = on(&map, move |m| {
        let npc = crate::game::tests_p3_combat::spawn(m, g, GIVER, 60, 2.0);
        let board = objects::summon(m, None, BOARD, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap();
        (npc, board)
    })
    .await;
    for (giver, caster) in [(npc, npc), (board, g)] {
        on(&map, move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.journal.rewarded.remove(&LESSON_QUEST);
            p.spells.retain(|s| *s != TAUGHT);
            take(m, g, LESSON_QUEST);
        })
        .await;
        a.clear();
        a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
            guid: giver,
            quest_id: LESSON_QUEST,
            reward: 0,
        })
        .await;
        let by = a
            .wait_for("the lesson", |m| match m {
                ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == LESSON => Some(s.caster),
                _ => None,
            })
            .await;
        assert_eq!(by, caster);
        let learned = on(&map, move |m| m.players[&g].spells.contains(&TAUGHT)).await;
        assert!(learned, "the player learns it");
    }
}

/// Thistle Tea's `credit_event`: "rewards a hidden flag once without
/// consuming a log slot or announcing it", "removes an accepted flag while
/// ordinary unaccepted quests remain untouched" and "rejects a low-level
/// flag without rewards or casts", as mangos's `AreaExploredOrEventHappens`
/// and `CompleteQuest` reward an auto-rewarded quest: its money and item,
/// silently, once; one in the log leaves it rewarded; an ordinary quest
/// not in the log is untouched; one above the player's level gives
/// nothing.
#[tokio::test]
async fn a_flag_quests_event_rewards_it_once_silently() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    a.clear();
    let seen = on(&map, move |m| {
        let money = |m: &Map| m.players[&g].money();
        let start = money(m);
        quests::event_happens(m, g, FLAG_QUEST);
        let once = (
            money(m) - start,
            m.players[&g].count_of(PRIZE),
            m.players[&g].journal.rewarded.contains(&FLAG_QUEST),
            m.players[&g].journal.log.len(),
        );
        quests::event_happens(m, g, FLAG_QUEST);
        let twice = (money(m) - start, m.players[&g].count_of(PRIZE));
        quests::event_happens(m, g, LOW_FLAG_QUEST);
        let low = (
            money(m) - start,
            m.players[&g].journal.rewarded.contains(&LOW_FLAG_QUEST),
        );
        quests::event_happens(m, g, PLAIN_QUEST);
        let plain = m.players[&g].journal.rewarded.contains(&PLAIN_QUEST);
        (once, twice, low, plain)
    })
    .await;
    assert_eq!(seen.0, (123, 1, true, 0), "rewarded, out of the log");
    assert_eq!(seen.1, (123, 1), "once");
    assert_eq!(seen.2, (123, false), "not above the player's level");
    assert!(!seen.3, "an ordinary quest untouched");
    let told = a
        .gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_COMPLETE(_)).then_some(())
        })
        .await;
    assert!(told.is_none(), "silently");
    // Taken: rewarded, and out of the log.
    let taken = on(&map, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .rewarded
            .remove(&FLAG_QUEST);
        take(m, g, FLAG_QUEST);
        quests::event_happens(m, g, FLAG_QUEST);
        let p = &m.players[&g];
        (
            p.journal.has(FLAG_QUEST),
            p.journal.rewarded.contains(&FLAG_QUEST),
        )
    })
    .await;
    assert_eq!(taken, (false, true));
}

/// Thistle Tea's "enforces ordinary subgroups and allows raid quest
/// confirmations", as mangos's `HandleQuestConfirmAccept` has it
/// (`IsAllowedInRaid`, else `IsInParty`): in a raid, a member of another
/// subgroup isn't asked to take an escort its starter took, but is asked
/// to take a raid quest's, and takes it.
#[tokio::test]
async fn a_party_quest_in_a_raid_asks_the_starters_subgroup_alone() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let (oa, ob) = (
        realm.registry.online(ga).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &oa, &ob));
    crate::game::social::groups::convert_to_raid(&realm, &oa);
    crate::game::social::groups::change_subgroup(&realm, &oa, "Bob", 1);
    let map = oa.map();
    let npc = on(&map, move |m| {
        crate::game::tests_p3_combat::spawn(m, ga, GIVER, 60, 2.0)
    })
    .await;
    b.clear();
    for (quest, asked) in [(ESCORT, false), (RAID_ESCORT, true)] {
        a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
            guid: npc,
            quest_id: quest,
        })
        .await;
        let prompt = b
            .gets(Duration::from_millis(400), |m| match m {
                ServerOpcodeMessage::SMSG_QUEST_CONFIRM_ACCEPT(c) if c.quest_id == quest => {
                    Some(())
                }
                _ => None,
            })
            .await;
        assert_eq!(prompt.is_some(), asked, "{quest}");
        b.send(CMSG_QUEST_CONFIRM_ACCEPT { quest_id: quest }).await;
        harness::an_update(&map).await;
        let has = on(&map, move |m| m.players[&gb].journal.has(quest)).await;
        assert_eq!(has, asked, "{quest} taken");
    }
}
