//! Round 11's test-only rows on quest-giving objects and their gossip
//! (Thistle Tea's `game_object_quests_test.exs`): an event's quests shown
//! and hidden with it, an exclusive quest's object going dark and a stale
//! accept refused, the flag refreshed as quests are taken and dropped and
//! forgotten as the object is, an accept's item and start script, a
//! turn-in's costs, rewards, end script and next quest, and an object's
//! gossip, bound to it and checked again at the choice.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::{BroadcastText, GossipMenu, GossipOption, Quest};
use crate::game::data::scripts::{Condition, ScriptStep};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, grid};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::object;

const EVENT_GIVER: u32 = 98_920;
const EXCLUSIVE_GIVER: u32 = 98_921;
const STARTER: u32 = 98_922;
const ENDER: u32 = 98_923;
const BOOK: u32 = 98_924;
const SPAWNS: [(u32, u32, f32, f32); 5] = [
    (9_920, EVENT_GIVER, 2.0, 0.0),
    (9_921, EXCLUSIVE_GIVER, 0.0, 2.0),
    (9_922, STARTER, -2.0, 0.0),
    (9_923, ENDER, 0.0, -2.0),
    (9_924, BOOK, 1.5, 1.5),
];

/// A quest of an event, started and ended at its object.
const EVENT_QUEST: u32 = 96_910;
const EVENT: u32 = 77;
/// A quest and its exclusive alternative.
const EXCLUSIVE: u32 = 96_911;
const ALTERNATIVE: u32 = 96_912;
/// A quest the starter gives with an item, its start script greeting.
const STARTED: u32 = 96_913;
const LETTER: u32 = 64_301;
const START_SCRIPT: u32 = 9_930;
const HELLO: u32 = 930;
/// A quest wanting two pelts, its reward a ring and 125 copper, its end
/// script waving, and the next of its chain at the same object.
const TURNED_IN: u32 = 96_914;
const NEXT: u32 = 96_915;
const PELT: u32 = 64_302;
const RING: u32 = 64_303;
const END_SCRIPT: u32 = 9_931;
const WELL_DONE: u32 = 931;
/// The book's menu: a gossip option running a script, a vendor's, and a
/// gossip option for the level 50 and over.
const MENU: u32 = 9_940;
const GOSSIP_SCRIPT: u32 = 9_941;
const LEVEL_50: u32 = 9_942;
const GOSSIP_IMP: u32 = 3_950;

fn quest(id: u32, title: &str) -> Quest {
    Quest {
        id,
        title: title.into(),
        min_level: 1,
        level: 1,
        ..Default::default()
    }
}

fn item(entry: u32, name: &str, stack: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        stackable: stack,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn talk(id: u32, text: u32) -> ScriptStep {
    ScriptStep {
        id,
        command: 0,
        dataint: [text as i32, 0, 0, 0],
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = data();
    for (entry, name) in [
        (EVENT_GIVER, "Event Box"),
        (EXCLUSIVE_GIVER, "Choice Box"),
        (STARTER, "Starter"),
        (ENDER, "Ender"),
    ] {
        let t = object(entry, 2, name, &[]);
        d.gameobject_templates.insert(entry, t);
    }
    d.gameobject_templates
        .insert(BOOK, object(BOOK, 2, "Book", &[(3, MENU)]));
    for (guid, entry, dx, dy) in SPAWNS {
        d.gameobject_spawns.entry(0).or_default().push(GameObjectSpawn {
            guid,
            entry,
            map: 0,
            x: START.0 + dx,
            y: START.1 + dy,
            z: START.2,
            state: 1,
            ..Default::default()
        });
    }
    for (id, title) in [
        (EVENT_QUEST, "Seasonal"),
        (STARTED, "Started"),
        (NEXT, "Next"),
    ] {
        d.npc.quests.insert(id, quest(id, title));
    }
    for id in [EXCLUSIVE, ALTERNATIVE] {
        d.npc.quests.insert(
            id,
            Quest {
                exclusive_group: EXCLUSIVE as i32,
                ..quest(id, "Either")
            },
        );
    }
    d.npc.quests.get_mut(&STARTED).unwrap().source_item = LETTER;
    d.npc.quests.get_mut(&STARTED).unwrap().source_item_count = 1;
    // Wanting the letter, so under way once taken.
    d.npc.quests.get_mut(&STARTED).unwrap().required_items[0] = (LETTER, 2);
    d.npc.quests.insert(
        TURNED_IN,
        Quest {
            required_items: [(PELT, 2), (0, 0), (0, 0), (0, 0)],
            rewards: [(RING, 1), (0, 0), (0, 0), (0, 0)],
            reward_money: 125,
            next_in_chain: NEXT,
            ..quest(TURNED_IN, "Pelts")
        },
    );
    d.npc.quests.get_mut(&NEXT).unwrap().prev_quest = TURNED_IN as i32;
    d.world.event_quests.insert(EVENT_QUEST, EVENT);
    d.npc.object_starts.extend([
        (EVENT_GIVER, vec![EVENT_QUEST]),
        (EXCLUSIVE_GIVER, vec![EXCLUSIVE]),
        (STARTER, vec![STARTED]),
        (ENDER, vec![TURNED_IN, NEXT]),
    ]);
    d.npc.object_ends.extend([
        (EVENT_GIVER, vec![EVENT_QUEST]),
        (ENDER, vec![TURNED_IN]),
    ]);
    for (entry, name, stack) in [(LETTER, "Letter", 1), (PELT, "Pelt", 20), (RING, "Ring", 1)] {
        d.items.insert(entry, item(entry, name, stack));
    }
    for (id, text) in [(HELLO, "Greetings, $N."), (WELL_DONE, "Well done, $N.")] {
        d.npc.broadcast_texts.insert(
            id,
            BroadcastText {
                male: text.into(),
                female: text.into(),
                ..Default::default()
            },
        );
    }
    d.scripts.quest_start_script.insert(STARTED, START_SCRIPT);
    d.scripts
        .quest_start
        .insert(START_SCRIPT, vec![talk(START_SCRIPT, HELLO)]);
    d.scripts.quest_complete_script.insert(TURNED_IN, END_SCRIPT);
    d.scripts
        .quest_end
        .insert(END_SCRIPT, vec![talk(END_SCRIPT, WELL_DONE)]);
    // The book's menu.
    let mut imp = d.creature_templates[&BRUTE].clone();
    imp.entry = GOSSIP_IMP;
    imp.faction = 35;
    d.creature_templates.insert(GOSSIP_IMP, imp);
    let option = |id, kind, condition| GossipOption {
        id,
        kind,
        npc_flag: 1,
        text: format!("Option {id}"),
        action_menu: -1,
        condition,
        script: GOSSIP_SCRIPT,
        ..Default::default()
    };
    d.npc.gossip_menus.insert(
        MENU,
        GossipMenu {
            texts: vec![(68, 0)],
            options: vec![option(0, 1, 0), option(1, 3, 0), option(2, 1, LEVEL_50)],
        },
    );
    d.scripts.conditions.insert(
        LEVEL_50,
        Condition {
            entry: LEVEL_50,
            kind: 15,
            values: [50, 1, 0, 0],
            flags: 0,
        },
    );
    d.scripts.gossip.insert(
        GOSSIP_SCRIPT,
        vec![ScriptStep {
            id: GOSSIP_SCRIPT,
            command: 10,
            datalong: [GOSSIP_IMP, 60_000, 0, 0],
            x: START.0 + 4.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    d
}

fn spawned(entry: u32) -> Guid {
    let (spawn, ..) = SPAWNS.iter().find(|s| s.1 == entry).unwrap();
    guid::game_object(entry, *spawn)
}

/// Whether `o` is shown usable to `g` (`Player::quest_objects`), once the
/// map's look at it (every ten updates) agrees with `want`.
async fn usable_becomes(map: &MapHandle, g: Guid, o: Guid, want: bool) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        if on(map, move |m| m.players[&g].quest_objects.contains(&o)).await == want {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{o:?} never {}",
            if want { "usable" } else { "unusable" }
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The next `GAMEOBJECT_DYN_FLAGS` this client is told for `object`.
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

async fn ready() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let (realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    for (_, entry, ..) in SPAWNS {
        a.sees(spawned(entry)).await;
    }
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    (realm, a, g, map)
}

/// Thistle Tea's "event transitions hide offerings and retained turn-ins
/// on quest objects": an event's quest object is usable only while the
/// event runs (`events::quest_offered` filtering both its starts and
/// ends), to take the quest or, with it taken, to turn it in; the event
/// over, it goes dark with the quest still in the log.
#[tokio::test]
async fn an_events_quest_object_is_usable_only_while_it_runs() {
    let (realm, mut a, g, map) = ready().await;
    let o = spawned(EVENT_GIVER);
    usable_becomes(&map, g, o, false).await;
    realm.social.events.lock().insert(EVENT);
    usable_becomes(&map, g, o, true).await;
    assert_eq!(shown(&mut a, o).await, 1, "told it is usable");
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: o,
        quest_id: EVENT_QUEST,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert!(on(&map, move |m| m.players[&g].journal.has(EVENT_QUEST)).await);
    // Taken: to turn in, still usable.
    tokio::time::sleep(Duration::from_millis(700)).await;
    usable_becomes(&map, g, o, true).await;
    realm.social.events.lock().remove(&EVENT);
    usable_becomes(&map, g, o, false).await;
    assert_eq!(shown(&mut a, o).await, 0, "told it is not");
    let (logged, offered) = on(&map, move |m| {
        (
            m.players[&g].journal.has(EVENT_QUEST),
            crate::game::map::quests::giver_quests(m, g, o).len(),
        )
    })
    .await;
    assert!(logged, "the quest kept");
    assert_eq!(offered, 0, "nothing to take or turn in");
}

/// Thistle Tea's "exclusive rewards disable object activation and reject
/// stale acceptance": a quest object whose quest's alternative of its
/// exclusive group is rewarded goes dark (`giver_quests` by `can_take`),
/// and an accept of it asked anyway is refused, logging nothing, giving
/// no item and running no script.
#[tokio::test]
async fn an_exclusive_quests_object_goes_dark_once_its_alternative_is_rewarded() {
    let (_realm, mut a, g, map) = ready().await;
    let o = spawned(EXCLUSIVE_GIVER);
    usable_becomes(&map, g, o, true).await;
    on(&map, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .rewarded
            .insert(ALTERNATIVE);
    })
    .await;
    usable_becomes(&map, g, o, false).await;
    a.clear();
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: o,
        quest_id: EXCLUSIVE,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert!(!on(&map, move |m| m.players[&g].journal.has(EXCLUSIVE)).await, "refused");
}

/// Thistle Tea's "refreshes accepted and abandoned quests and forgets
/// objects that leave view": a starter whose quest is taken goes dark
/// (it ends none), and lights again when the quest is dropped
/// (`quests::update_objects`, every ten updates); one that goes out of
/// sight is forgotten from the player's quest objects, and looked at
/// afresh when it comes back.
#[tokio::test]
async fn a_starters_flag_follows_its_quest_taken_and_dropped_and_goes_with_it() {
    let (_realm, mut a, g, map) = ready().await;
    let o = spawned(STARTER);
    usable_becomes(&map, g, o, true).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: o,
        quest_id: STARTED,
    })
    .await;
    usable_becomes(&map, g, o, false).await;
    let slot = on(&map, move |m| {
        m.players[&g]
            .journal
            .log
            .iter()
            .position(|l| l.quest == STARTED)
    })
    .await
    .expect("taken");
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: slot as u8 }).await;
    usable_becomes(&map, g, o, true).await;
    // Out of sight, then back.
    let away = |dx: f32| {
        move |m: &mut Map| {
            let at = Vector3d {
                x: START.0 + dx,
                y: START.1,
                z: START.2,
            };
            let p = m.players.get_mut(&g).unwrap();
            p.movement.position = at;
            p.move_check = None;
            let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
            p.cell = new;
            m.grid.relocate(g, old, new);
            m.visibility_dirty.insert(g);
        }
    };
    on(&map, away(300.0)).await;
    a.forgets(o).await;
    usable_becomes(&map, g, o, false).await;
    on(&map, away(0.0)).await;
    a.sees(o).await;
    usable_becomes(&map, g, o, true).await;
}

fn said_by(m: &ServerOpcodeMessage, by: Guid) -> Option<String> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
            SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. } if *sender1 == by => {
                Some(c.message.clone())
            }
            _ => None,
        },
        _ => None,
    }
}

/// Thistle Tea's "grants the source item and dispatches the start script
/// to the object owner": a quest taken at an object gives its item, and
/// runs its start script with the object as its source and the player
/// its target (mangos's `AddQuest` and `ScriptsStart`): the object greets
/// her by name.
#[tokio::test]
async fn a_quest_taken_at_an_object_gives_its_item_and_runs_its_start_script() {
    let (_realm, mut a, g, map) = ready().await;
    let o = spawned(STARTER);
    a.clear();
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: o,
        quest_id: STARTED,
    })
    .await;
    let line = a.wait_for("the object's greeting", |m| said_by(m, o)).await;
    assert_eq!(line, "Greetings, Alice.");
    let (logged, letters) = on(&map, move |m| {
        let p = &m.players[&g];
        (p.journal.has(STARTED), p.count_of(LETTER))
    })
    .await;
    assert!(logged);
    assert_eq!(letters, 1, "its item");
}

/// Thistle Tea's "consumes objectives, grants rewards, runs the end
/// script, and offers the object chain": a quest turned in at an object
/// takes the items it wants (two of three pelts), gives its ring and
/// money, is rewarded and out of the log, runs its end script from the
/// object, and offers the next quest of its chain there; a second choice
/// of its reward gives nothing more.
#[tokio::test]
async fn a_quest_turned_in_at_an_object_takes_its_items_and_offers_the_next() {
    let (_realm, mut a, g, map) = ready().await;
    let o = spawned(ENDER);
    let money = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, g, PELT, 3, false).unwrap();
        m.players.get_mut(&g).unwrap().journal.add(crate::game::store::Logged {
            quest: TURNED_IN,
            ..Default::default()
        });
        crate::game::map::quests::check(m, g, TURNED_IN);
        m.players[&g].fields.get_u32(PLAYER_FIELD_COINAGE)
    })
    .await;
    a.clear();
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: o,
        quest_id: TURNED_IN,
        reward: 0,
    })
    .await;
    let line = a.wait_for("the object's end script", |m| said_by(m, o)).await;
    assert_eq!(line, "Well done, Alice.");
    let next = a
        .wait_frame(
            "the next quest's details",
            crate::game::map::quests::SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE,
            |b| b.len() >= 12,
        )
        .await;
    assert_eq!(u64::from_le_bytes(next[0..8].try_into().unwrap()), o.guid());
    assert_eq!(u32::from_le_bytes(next[8..12].try_into().unwrap()), NEXT);
    let after = move |m: &mut Map| {
        let p = &m.players[&g];
        (
            p.count_of(PELT),
            p.count_of(RING),
            p.fields.get_u32(PLAYER_FIELD_COINAGE),
            p.journal.has(TURNED_IN),
            p.journal.rewarded.contains(&TURNED_IN),
        )
    };
    let once = on(&map, after).await;
    assert_eq!(once, (1, 1, money + 125, false, true));
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: o,
        quest_id: TURNED_IN,
        reward: 0,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(on(&map, after).await, once, "nothing more");
}

fn imps(m: &mut Map) -> usize {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == GOSSIP_IMP && c.alive())
        .count()
}

/// The options of the next `SMSG_GOSSIP_MESSAGE` from `of`.
async fn menu(c: &mut Client, of: Guid) -> Vec<u32> {
    c.wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == of => {
            Some(g.gossips.iter().map(|o| o.id).collect())
        }
        _ => None,
    })
    .await
}

fn select(o: Guid, option: u32) -> CMSG_GOSSIP_SELECT_OPTION {
    CMSG_GOSSIP_SELECT_OPTION {
        guid: o,
        gossip_list_id: option,
        unknown: None,
    }
}

/// Thistle Tea's "shows conditioned object gossip, excludes NPC services,
/// and binds selections to the source": an object's menu shows its gossip
/// options alone, not a vendor's, nor one whose condition isn't met
/// (`npc::menu_options`); a choice named for another object does nothing,
/// and one for the object runs its script.
#[tokio::test]
async fn an_objects_menu_shows_its_own_gossip_and_runs_it_only_for_itself() {
    let (_realm, mut a, _g, map) = ready().await;
    let book = spawned(BOOK);
    a.clear();
    a.send(CMSG_GAMEOBJ_USE { guid: book }).await;
    assert_eq!(menu(&mut a, book).await, vec![0], "its gossip alone");
    a.send(select(spawned(STARTER), 0)).await;
    a.send(select(guid::game_object(BOOK, 1), 0)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(on(&map, imps).await, 0, "not for another");
    a.send(select(book, 0)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(on(&map, imps).await, 1, "its script");
}

/// Thistle Tea's "revalidates distance and option conditions before
/// executing object gossip": an option offered is checked again as it is
/// chosen: from out of reach of the object, or no longer meeting its
/// condition, it does nothing (`npc::select`: `objects::within_reach` and
/// each option's condition).
#[tokio::test]
async fn an_objects_option_is_checked_again_as_it_is_chosen() {
    let (_realm, mut a, g, map) = ready().await;
    let book = spawned(BOOK);
    on(&map, move |m| crate::game::tests_p3_combat::level(m, g, 60)).await;
    a.clear();
    a.send(CMSG_GAMEOBJ_USE { guid: book }).await;
    assert_eq!(menu(&mut a, book).await, vec![0, 2], "the level 50 option too");
    let put = |dx: f32| {
        move |m: &mut Map| {
            m.players.get_mut(&g).unwrap().movement.position = Vector3d {
                x: START.0 + dx,
                y: START.1,
                z: START.2,
            };
        }
    };
    on(&map, put(20.0)).await;
    a.send(select(book, 2)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(on(&map, imps).await, 0, "out of reach");
    on(&map, put(0.0)).await;
    on(&map, move |m| crate::game::tests_p3_combat::level(m, g, 1)).await;
    a.send(select(book, 2)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(on(&map, imps).await, 0, "the condition no longer met");
    on(&map, move |m| crate::game::tests_p3_combat::level(m, g, 60)).await;
    a.send(select(book, 2)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(on(&map, imps).await, 1, "met and near: its script");
}
