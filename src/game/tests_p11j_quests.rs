//! Round 11j's quest and gossip tests: what wvs already did, shown through
//! the protocol (Thistle Tea's `gossip_test`, `quest_objects_test` and
//! `quest_required_condition_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::npc::{GossipMenu, GossipOption};
use crate::game::data::scripts::{Condition, ScriptStep};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{HUMANS, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, take_quest};
use crate::game::tests_p3_player2_ammo::handled;

const NONE_WITHIN: Duration = Duration::from_millis(300);

/// Quest givers, one a test.
const EVENT_GIVER: u32 = 97_100;
const BREAD_GIVER: u32 = 97_101;
const PREREQ_GIVER: u32 = 97_102;
const COND_GIVER: u32 = 97_103;
const TALKER: u32 = 97_104;
/// An event's quest, and its event.
const EVENT_QUEST: u32 = 97_001;
const EVENT: u32 = 77;
/// A breadcrumb to a quest wanting the token carried.
const TARGET: u32 = 97_010;
const BREADCRUMB: u32 = 97_011;
/// A quest wanting its parent under way and 50 Cooking.
const PARENT: u32 = 97_020;
const CHILD: u32 = 97_021;
const COOKING: u32 = 185;
/// +1 Cooking, while it lasts.
const COOKING_BONUS: u32 = 97_900;
/// A quest wanting the token carried.
const COND_QUEST: u32 = 97_030;
const TOKEN: u32 = 97_500;
const HAS_TOKEN: u32 = 97_600;
/// The talker's menu, its reply's, and the reply's script.
const MENU: u32 = 97_700;
const REPLY: u32 = 97_701;
const REPLY_TEXT: u32 = 738;
const REPLY_SCRIPT: u32 = 97_702;
const BOW: u32 = 2;
/// A goober with a menu, and one with a page and the menu.
const MENU_STONE: u32 = 97_800;
const PAGE_STONE: u32 = 97_801;
const STONE_MENU: u32 = 97_703;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for entry in [EVENT_GIVER, BREAD_GIVER, PREREQ_GIVER, COND_GIVER, TALKER] {
        d.creature_templates.insert(
            entry,
            crate::game::data::CreatureTemplate {
                faction: HUMANS,
                npc_flags: crate::game::map::npc::NPC_FLAG_GOSSIP
                    | crate::game::map::npc::NPC_FLAG_QUESTGIVER,
                gossip_menu_id: if entry == TALKER { MENU } else { 0 },
                ..template(entry, 7)
            },
        );
    }
    let quest = |id: u32| crate::game::data::npc::Quest {
        id,
        title: format!("Quest {id}"),
        level: 1,
        ..Default::default()
    };
    for id in [TARGET, PARENT, COND_QUEST] {
        d.npc.quests.insert(id, quest(id));
    }
    // Something to bring, so that it is under way once taken.
    d.npc.quests.insert(
        EVENT_QUEST,
        crate::game::data::npc::Quest {
            required_items: [(TOKEN, 1), (0, 0), (0, 0), (0, 0)],
            ..quest(EVENT_QUEST)
        },
    );
    d.npc.quests.insert(
        BREADCRUMB,
        crate::game::data::npc::Quest {
            breadcrumb_for: TARGET,
            ..quest(BREADCRUMB)
        },
    );
    d.npc.quests.insert(
        CHILD,
        crate::game::data::npc::Quest {
            prev_quest: -(PARENT as i32),
            required_skill: COOKING,
            required_skill_value: 50,
            ..quest(CHILD)
        },
    );
    d.world.event_quests.insert(EVENT_QUEST, EVENT);
    d.npc.creature_starts.insert(EVENT_GIVER, vec![EVENT_QUEST]);
    d.npc.creature_ends.insert(EVENT_GIVER, vec![EVENT_QUEST]);
    d.npc.creature_starts.insert(BREAD_GIVER, vec![BREADCRUMB]);
    d.npc.creature_starts.insert(PREREQ_GIVER, vec![CHILD]);
    d.npc.creature_starts.insert(COND_GIVER, vec![COND_QUEST]);
    d.items.insert(
        TOKEN,
        crate::game::data::ItemTemplate {
            entry: TOKEN,
            name: "Token".into(),
            stackable: 1,
            ..Default::default()
        },
    );
    d.scripts.conditions.insert(
        HAS_TOKEN,
        Condition {
            entry: HAS_TOKEN,
            kind: 2,
            values: [TOKEN as i32, 1, 0, 0],
            flags: 0,
        },
    );
    d.scripts.quest_condition.insert(TARGET, HAS_TOKEN);
    d.scripts.quest_condition.insert(COND_QUEST, HAS_TOKEN);
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_MOD_SKILL,
        base_points: 0,
        base_dice: 1,
        misc_value: COOKING as i32,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        COOKING_BONUS,
        SpellTemplate {
            id: COOKING_BONUS,
            effects,
            name: "Cook's Hat".into(),
            ..Default::default()
        },
    );
    // The talker's option shows a reply, and bows.
    d.npc.gossip_menus.insert(
        MENU,
        GossipMenu {
            texts: vec![(4995, 0)],
            options: vec![GossipOption {
                id: 0,
                kind: 1,
                action_menu: REPLY as i32,
                script: REPLY_SCRIPT,
                text: "Tell me.".into(),
                ..Default::default()
            }],
        },
    );
    d.npc.gossip_menus.insert(
        REPLY,
        GossipMenu {
            texts: vec![(REPLY_TEXT, 0)],
            options: Vec::new(),
        },
    );
    d.scripts.gossip.insert(
        REPLY_SCRIPT,
        vec![ScriptStep {
            command: 1,
            datalong: [BOW, 0, 0, 0],
            ..Default::default()
        }],
    );
    // The stones' menu: an option that closes it.
    d.npc.gossip_menus.insert(
        STONE_MENU,
        GossipMenu {
            texts: vec![(68, 0)],
            options: vec![GossipOption {
                id: 0,
                kind: 1,
                action_menu: -1,
                text: "Read".into(),
                ..Default::default()
            }],
        },
    );
    for (entry, page) in [(MENU_STONE, 0), (PAGE_STONE, 15)] {
        let mut t = crate::game::data::GameObjectTemplate {
            entry,
            kind: 10,
            display_id: 1,
            name: format!("Stone {entry}"),
            size: 1.0,
            ..Default::default()
        };
        t.data[7] = page;
        t.data[19] = STONE_MENU;
        d.gameobject_templates.insert(entry, t);
    }
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    setup_with(world(), Class::Warrior).await
}

async fn giver(map: &MapHandle, a: &mut Client, g: Guid, entry: u32) -> Guid {
    let c = on(map, move |m| spawn(m, g, entry, 20, 2.0)).await;
    a.sees(c).await;
    c
}

async fn status(c: &mut Client, giver: Guid) -> QuestGiverStatus {
    c.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver }).await;
    c.wait_for("the quest status", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) if s.guid == giver => Some(s.status),
        _ => None,
    })
    .await
}

/// The quest a giver's hello shows the details of, or none: a list then.
async fn hello(c: &mut Client, giver: Guid) -> Option<u32> {
    use crate::game::map::quests::SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE;
    let before = c.captured().len();
    c.send(CMSG_QUESTGIVER_HELLO { guid: giver }).await;
    handled(c).await;
    let mut details = None;
    for f in &c.captured()[before..] {
        let opcode = u16::from_le_bytes([f[2], f[3]]);
        if opcode == SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE {
            details = Some(u32::from_le_bytes(f[12..16].try_into().unwrap()));
        }
    }
    details
}

/// The quests a giver's gossip lists.
async fn gossip_quests(c: &mut Client, giver: Guid) -> Vec<u32> {
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: giver }).await;
    c.wait_for("the gossip", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == giver => {
            Some(g.quests.iter().map(|q| q.quest_id).collect())
        }
        // A giver of quests and no menu answers with its quest list.
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_LIST(l) if l.npc == giver => {
            Some(l.quest_items.iter().map(|q| q.quest_id).collect())
        }
        _ => None,
    })
    .await
}

/// Whether accepting `quest` from `giver` put it in the log.
async fn accept(c: &mut Client, map: &MapHandle, giver: Guid, quest: u32) -> bool {
    c.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: quest,
    })
    .await;
    handled(c).await;
    let g = c.guid;
    on(map, move |m| m.players[&g].journal.has(quest)).await
}

async fn event(realm: &std::sync::Arc<crate::game::Realm>, map: &MapHandle, running: bool) {
    let on_now: ahash::AHashSet<u32> = if running {
        [EVENT].into_iter().collect()
    } else {
        ahash::AHashSet::new()
    };
    *realm.social.events.lock() = on_now.clone();
    on(map, move |m| crate::game::events::apply(m, &on_now)).await;
}

fn give(m: &mut Map, g: Guid, item: u32) {
    let _ = crate::game::map::inventory::add_item(m, g, item, 1, false);
}

/// Thistle Tea's "rechecks event state after details while retaining
/// accepted quest progress": an event's quest is offered only while the
/// event runs; its details shown and the event over, accepting it is
/// refused; taken while it runs, it stays in the log after it, though its
/// giver no longer lists it (mangos's `IsActive` of the quest in
/// `PrepareQuestMenu`, `GetDialogStatus` and `CanTakeQuest`); the event
/// back, it is listed again, under way.
#[tokio::test]
async fn an_event_quest_is_offered_and_taken_while_its_event_runs_and_kept_after() {
    let (realm, mut a, g, map) = setup().await;
    let giver = giver(&map, &mut a, g, EVENT_GIVER).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::None);
    assert!(!accept(&mut a, &map, giver, EVENT_QUEST).await);
    event(&realm, &map, true).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::Available);
    assert_eq!(hello(&mut a, giver).await, Some(EVENT_QUEST), "its details");
    event(&realm, &map, false).await;
    assert!(
        !accept(&mut a, &map, giver, EVENT_QUEST).await,
        "the event over since its details"
    );
    event(&realm, &map, true).await;
    assert!(accept(&mut a, &map, giver, EVENT_QUEST).await);
    event(&realm, &map, false).await;
    assert_eq!(gossip_quests(&mut a, giver).await, Vec::<u32>::new());
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::None);
    assert!(
        on(&map, move |m| m.players[&g].journal.has(EVENT_QUEST)).await,
        "kept in the log"
    );
    event(&realm, &map, true).await;
    let listed = on(&map, move |m| {
        crate::game::map::quests::giver_quests(m, g, giver)
            .iter()
            .map(|q| (q.quest_id, q.quest_icon))
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(listed, vec![(EVENT_QUEST, 3)], "under way");
}

/// Thistle Tea's "evaluates breadcrumb target conditions for dialog and
/// acceptance": a breadcrumb to a quest whose condition (the token
/// carried) isn't met is neither shown nor taken; met, it is shown,
/// listed and taken (Thistle Tea's `breadcrumb_conditions_met?`).
#[tokio::test]
async fn a_breadcrumb_is_had_only_while_its_targets_condition_is_met() {
    let (_realm, mut a, g, map) = setup().await;
    let giver = giver(&map, &mut a, g, BREAD_GIVER).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::None);
    assert_eq!(gossip_quests(&mut a, giver).await, Vec::<u32>::new());
    assert!(!accept(&mut a, &map, giver, BREADCRUMB).await);
    on(&map, move |m| give(m, g, TOKEN)).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::Available);
    assert_eq!(gossip_quests(&mut a, giver).await, vec![BREADCRUMB]);
    assert!(accept(&mut a, &map, giver, BREADCRUMB).await);
}

/// Thistle Tea's "prerequisite abandonment closes stale dialogs and
/// profession bonuses reach eligibility": a quest wanting its parent under
/// way (a negative `PrevQuestId`) and 50 Cooking is offered at 49 Cooking
/// and a +1 bonus (mangos's `GetSkillValue` with its bonuses), not
/// without it; the parent abandoned, it is refused and not offered; the
/// parent back, taken.
#[tokio::test]
async fn a_quest_wanting_its_parent_on_and_a_skill_counts_the_bonus_and_the_parent_now() {
    let (_realm, mut a, g, map) = setup().await;
    let giver = giver(&map, &mut a, g, PREREQ_GIVER).await;
    on(&map, move |m| {
        take_quest(m, g, PARENT);
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, COOKING, 1);
        crate::game::map::skills::set_value(p, COOKING, 49);
    })
    .await;
    assert_eq!(
        status(&mut a, giver).await,
        QuestGiverStatus::None,
        "49 Cooking"
    );
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, COOKING_BONUS, Some(g));
    })
    .await;
    let with_bonus = on(&map, move |m| {
        crate::game::map::skills::value_with_bonuses(&m.players[&g], COOKING)
    })
    .await;
    assert_eq!(with_bonus, 50);
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::Available);
    assert_eq!(gossip_quests(&mut a, giver).await, vec![CHILD]);
    // The parent abandoned: the window shown is stale.
    let slot = on(&map, move |m| m.players[&g].journal.slot_of(PARENT)).await;
    a.send(CMSG_QUESTLOG_REMOVE_QUEST {
        slot: slot.expect("the parent's slot") as u8,
    })
    .await;
    handled(&mut a).await;
    assert!(!on(&map, move |m| m.players[&g].journal.has(PARENT)).await);
    assert!(!accept(&mut a, &map, giver, CHILD).await);
    assert_eq!(gossip_quests(&mut a, giver).await, Vec::<u32>::new());
    on(&map, move |m| take_quest(m, g, PARENT)).await;
    assert!(accept(&mut a, &map, giver, CHILD).await);
}

/// Thistle Tea's "status, native hello, and gossip share conditioned
/// visibility": a quest whose `RequiredCondition` (the token carried)
/// isn't met shows no "!", its giver's hello and gossip list nothing; met,
/// the "!", the hello's details of it, and the gossip's list of it.
#[tokio::test]
async fn a_conditioned_quest_shows_alike_in_the_status_the_hello_and_the_gossip() {
    let (_realm, mut a, g, map) = setup().await;
    let giver = giver(&map, &mut a, g, COND_GIVER).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::None);
    assert_eq!(hello(&mut a, giver).await, None);
    assert_eq!(gossip_quests(&mut a, giver).await, Vec::<u32>::new());
    on(&map, move |m| give(m, g, TOKEN)).await;
    assert_eq!(status(&mut a, giver).await, QuestGiverStatus::Available);
    assert_eq!(hello(&mut a, giver).await, Some(COND_QUEST));
    assert_eq!(gossip_quests(&mut a, giver).await, vec![COND_QUEST]);
}

/// Thistle Tea's "a scripted reply shows its text and runs the option's
/// steps on the speaker": an option with a script and a menu below it
/// runs the script on the creature (it bows) and shows that menu, its
/// text and nothing else (mangos's `GOSSIP_OPTION_GOSSIP` with an
/// `action_menu_id` and `action_script_id`).
#[tokio::test]
async fn a_scripted_option_runs_its_script_on_the_speaker_and_shows_its_reply() {
    let (_realm, mut a, g, map) = setup().await;
    let talker = giver(&map, &mut a, g, TALKER).await;
    a.send(CMSG_GOSSIP_HELLO { guid: talker }).await;
    a.wait_for("the menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == talker => Some(()),
        _ => None,
    })
    .await;
    a.clear();
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: talker,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    let reply = a
        .wait_for("the reply", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == talker => {
                Some((g.title_text_id, g.gossips.len(), g.quests.len()))
            }
            _ => None,
        })
        .await;
    assert_eq!(reply, (REPLY_TEXT, 0, 0));
    a.wait_for("the talker's bow", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == talker => Some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's "gossip selection validates object range and page text
/// takes precedence": a stone's menu (its `data19`) is shown on use, and
/// its option chosen from out of reach does nothing, from beside it closes
/// the window; a stone with a page (its `data7`) as well shows the page,
/// not the menu (mangos's `GAMEOBJECT_TYPE_GOOBER`: the page text first).
#[tokio::test]
async fn an_objects_menu_is_chosen_from_beside_it_and_its_page_comes_first() {
    let (_realm, mut a, g, map) = setup().await;
    let (stone, paged, at) = on(&map, move |m| {
        let p = m.players[&g].position();
        let near = |dy: f32| Vector3d { y: p.y + dy, ..p };
        let summon = |m: &mut Map, entry, at| {
            crate::game::map::objects::summon(m, None, entry, at, 0.0, 0, 0, 0, None).unwrap()
        };
        (
            summon(m, MENU_STONE, near(1.5)),
            summon(m, PAGE_STONE, near(-1.5)),
            p,
        )
    })
    .await;
    a.sees(stone).await;
    a.sees(paged).await;
    a.send(CMSG_GAMEOBJ_USE { guid: stone }).await;
    a.wait_for("the stone's menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == stone => Some(()),
        _ => None,
    })
    .await;
    // Thirty yards off: nothing.
    a.heartbeat(at.x + 30.0, at.y, at.z).await;
    a.clear();
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: stone,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    let far = a
        .gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
        })
        .await;
    assert!(far.is_none(), "chosen from out of reach");
    a.heartbeat(at.x, at.y, at.z).await;
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: stone,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    a.wait_for("the window closed", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    // The page before the menu.
    a.clear();
    a.send(CMSG_GAMEOBJ_USE { guid: paged }).await;
    a.wait_for("the page", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_PAGETEXT(p) if p.guid == paged => Some(()),
        _ => None,
    })
    .await;
    handled(&mut a).await;
    let menu = a
        .gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == paged => Some(()),
            _ => None,
        })
        .await;
    assert!(menu.is_none(), "the page, not the menu");
}
