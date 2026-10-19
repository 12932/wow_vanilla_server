//! Phase 3's quests, Thistle Tea's quest tests ported: the log's slots and
//! fields, kill and item credit, a quest coming undone, what a giver shows
//! and lists, auto-complete quests, turn-ins, abandoning and the items a
//! quest gives and takes.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::scripts::Condition;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, quests};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// A quest giver beside the start, which ends what it starts.
const GIVER: u32 = 3100;
const BOAR: u32 = 3101;
const STONE: u32 = 3102;
/// Givers no one stands beside, for what they show.
const SHOWS: u32 = 3110;
/// Stackable, for objectives.
const SCALE: u32 = 9100;
const PELT: u32 = 9101;
const BOX: u32 = 9102;
const MAP: u32 = 9103;
const SATCHEL: u32 = 9104;
const BOMB: u32 = 9105;
/// Starts quest 75, and quest 76, which wants it.
const LETTER: u32 = 9106;
const ORDERS: u32 = 9107;
const FILLER: u32 = 9108;
const STORMWIND: u32 = 72;
/// Level at least 5: a fresh character fails it.
const LEVEL_FIVE: u32 = 1;
/// Level at least 1: anyone meets it.
const LEVEL_ONE: u32 = 2;
const EVENT: u32 = 12;
const BLESSING: u32 = 900_800;

fn quest(id: u32) -> Quest {
    Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        ..Default::default()
    }
}

fn item(entry: u32, stack: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: stack,
        allowable_class: -1,
        allowable_race: -1,
        buy_count: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let creature = |entry, name: &str| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 2,
        level_max: 2,
        faction: 35,
        unit_class: 1,
        health_max: 50,
        npc_flags: 0x2,
        ..Default::default()
    };
    d.creature_templates.insert(GIVER, creature(GIVER, "Giver"));
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![GIVER],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        }],
    );
    for (entry, stack) in [
        (SCALE, 20),
        (PELT, 20),
        (BOX, 1),
        (MAP, 1),
        (BOMB, 20),
        (LETTER, 1),
        (ORDERS, 1),
        (FILLER, 1),
    ] {
        d.items.insert(entry, item(entry, stack));
    }
    let mut satchel = item(SATCHEL, 1);
    satchel.class = 1;
    satchel.inventory_type = 18;
    satchel.container_slots = 4;
    d.items.insert(SATCHEL, satchel);
    // A map that starts quest 70, which gives it too.
    d.items.get_mut(&MAP).unwrap().start_quest = 70;
    d.items.get_mut(&LETTER).unwrap().start_quest = 75;
    d.items.get_mut(&ORDERS).unwrap().start_quest = 76;
    let mut starts = Vec::new();
    let mut add = |d: &mut GameData, q: Quest| {
        starts.push(q.id);
        d.npc.quests.insert(q.id, q);
    };
    for id in [1, 2, 3, 4] {
        add(&mut d, quest(id));
    }
    // Kills: two boars.
    let mut q = quest(20);
    q.required_kills[0] = (BOAR as i32, 2);
    add(&mut d, q);
    // Items: two scales.
    let mut q = quest(30);
    q.required_items[0] = (SCALE, 2);
    add(&mut d, q);
    // An exploration that wants scales too.
    let mut q = quest(31);
    q.special_flags = 2;
    q.required_items[0] = (SCALE, 2);
    add(&mut d, q);
    d.npc.area_trigger_quests.insert(77, 31);
    // A creature to cast at (counter 0) and a stone to use (counter 2).
    let mut q = quest(32);
    q.required_kills[0] = (BOAR as i32, 1);
    q.required_spell_casts[0] = 17_166;
    q.required_kills[2] = (-(STONE as i32), 1);
    add(&mut d, q);
    // A standing to reach.
    let mut q = quest(33);
    q.rep_objective = (STORMWIND, 3000);
    add(&mut d, q);
    // Turn-ins: two scales and a silver.
    let mut q = quest(60);
    q.required_items[0] = (SCALE, 2);
    q.reward_money = -100;
    q.rewards[0] = (PELT, 1);
    add(&mut d, q);
    // Two scales for a pelt, a box and a blessing.
    let mut q = quest(62);
    q.required_items[0] = (SCALE, 2);
    q.rewards[0] = (PELT, 1);
    q.rewards[1] = (BOX, 1);
    q.reward_spell_cast = BLESSING;
    add(&mut d, q);
    let mut bless = crate::game::data::spells::SpellTemplate {
        id: BLESSING,
        name: "Blessing".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    bless.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: 29,
        target_a: 1,
        base_points: 4,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(BLESSING, bless);
    // Auto-complete: two scales for a pelt; and one that repeats.
    let mut q = quest(61);
    q.auto_complete = true;
    q.required_items[0] = (SCALE, 2);
    q.rewards[0] = (PELT, 1);
    add(&mut d, q);
    // A condition there is no row for; chains to a quest whose condition
    // fails, to one that may be had, and to one given elsewhere.
    add(&mut d, quest(85));
    add(
        &mut d,
        Quest {
            next_in_chain: 87,
            ..quest(86)
        },
    );
    add(&mut d, quest(87));
    add(
        &mut d,
        Quest {
            next_in_chain: 89,
            ..quest(88)
        },
    );
    add(&mut d, quest(89));
    add(
        &mut d,
        Quest {
            next_in_chain: 96,
            ..quest(95)
        },
    );
    d.npc.quests.insert(96, quest(96));
    d.scripts.quest_condition.insert(85, 999);
    d.scripts.quest_condition.insert(87, LEVEL_FIVE);
    // Starters and the items quests give.
    let mut q = quest(70);
    q.source_item = MAP;
    add(&mut d, q);
    let mut q = quest(71);
    q.source_item = BOX;
    add(&mut d, q);
    let mut q = quest(72);
    q.source_item = BOMB;
    q.source_item_count = 5;
    add(&mut d, q);
    let mut q = quest(73);
    q.source_item = SATCHEL;
    add(&mut d, q);
    let mut q = quest(74);
    q.source_item = PELT;
    add(&mut d, q);
    add(&mut d, quest(75));
    let mut q = quest(76);
    q.required_items[0] = (ORDERS, 1);
    add(&mut d, q);
    d.npc.creature_ends.insert(GIVER, starts.clone());
    // 42, 51 and 53 are given here too, for their refusals.
    starts.extend([42, 51, 53]);
    d.npc.creature_starts.insert(GIVER, starts);
    // The givers no one stands beside, each with its own quest.
    let shows: [(u32, Quest); 11] = [
        // Too low, by two levels.
        (
            0,
            Quest {
                min_level: 3,
                ..quest(40)
            },
        ),
        // Too low, and a condition unmet.
        (
            1,
            Quest {
                min_level: 3,
                ..quest(41)
            },
        ),
        // Too low, and a condition met.
        (
            2,
            Quest {
                min_level: 3,
                ..quest(42)
            },
        ),
        // Too low, and for orcs.
        (
            3,
            Quest {
                min_level: 3,
                required_races: 2,
                ..quest(43)
            },
        ),
        // To bring a scale: under way.
        (
            4,
            Quest {
                required_items: [(SCALE, 1), (0, 0), (0, 0), (0, 0)],
                ..quest(44)
            },
        ),
        // Nothing to do: done once taken.
        (5, quest(45)),
        // Auto-complete, once and again.
        (
            6,
            Quest {
                auto_complete: true,
                ..quest(46)
            },
        ),
        (
            7,
            Quest {
                auto_complete: true,
                special_flags: 1,
                ..quest(47)
            },
        ),
        // A condition there is no row for.
        (8, quest(48)),
        // An event's, and a condition unmet: to bring a scale.
        (
            9,
            Quest {
                required_items: [(SCALE, 1), (0, 0), (0, 0), (0, 0)],
                ..quest(49)
            },
        ),
        (
            10,
            Quest {
                required_items: [(SCALE, 1), (0, 0), (0, 0), (0, 0)],
                ..quest(50)
            },
        ),
    ];
    for (n, q) in shows {
        d.npc.creature_starts.insert(SHOWS + n, vec![q.id]);
        if n != 4 {
            d.npc.creature_ends.insert(SHOWS + n, vec![q.id]);
        }
        d.npc.quests.insert(q.id, q);
    }
    // Quest 44 ends elsewhere.
    d.npc.creature_ends.insert(SHOWS + 20, vec![44]);
    d.scripts.quest_condition.insert(41, LEVEL_FIVE);
    d.scripts.quest_condition.insert(42, LEVEL_ONE);
    d.scripts.quest_condition.insert(48, 999);
    d.scripts.quest_condition.insert(50, LEVEL_FIVE);
    d.world.event_quests.insert(49, EVENT);
    for (entry, level) in [(LEVEL_FIVE, 5), (LEVEL_ONE, 1)] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind: 15,
                values: [level, 1, 0, 0],
                flags: 0,
            },
        );
    }
    // Race and class masks.
    for (id, races, classes) in [(51, 2, 0), (52, 1, 0), (53, 0, 256), (54, 0, 1)] {
        d.npc.quests.insert(
            id,
            Quest {
                required_races: races,
                required_classes: classes,
                ..quest(id)
            },
        );
    }
    // Raid quests: one ordinary, one for raids, and a spell to cast.
    let mut q = quest(80);
    q.required_kills[0] = (BOAR as i32, 2);
    q.required_items[0] = (SCALE, 1);
    d.npc.quests.insert(80, q);
    let mut q = quest(81);
    q.kind = 62;
    q.required_kills[0] = (BOAR as i32, 2);
    q.required_items[0] = (PELT, 1);
    d.npc.quests.insert(81, q);
    let mut q = quest(82);
    q.required_kills[0] = (BOAR as i32, 2);
    q.required_spell_casts[0] = 10_292;
    d.npc.quests.insert(82, q);
    // To share: timed, giving two bombs; one not shareable; one for level 5.
    let share = crate::game::map::questshare::QUEST_FLAGS_SHARABLE;
    for (id, flags, min_level) in [(90, share, 1), (91, 0, 1), (92, share, 5)] {
        let mut q = quest(id);
        q.flags = flags;
        q.min_level = min_level;
        q.required_items[0] = (SCALE, 1);
        if id == 90 {
            q.limit_time = 600;
            q.source_item = BOMB;
            q.source_item_count = 2;
        }
        d.npc.quests.insert(id, q);
    }
    // Many, for a full log.
    for id in 100..120 {
        d.npc.quests.insert(id, quest(id));
    }
    d.misc.factions.insert(
        STORMWIND,
        crate::game::data::misc::FactionDef {
            id: STORMWIND,
            index: 0,
            name: "Stormwind".into(),
            variants: [(1, 0, 0, 0x1), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(0, STORMWIND);
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Wait until `f` says so, as the map has it.
async fn until<T: Send + 'static>(
    realm: &Arc<Realm>,
    what: &str,
    f: impl Fn(&mut Map) -> Option<T> + Send + Sync + Clone + 'static,
) -> T {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        if let Some(t) = on_map(realm, f.clone()).await {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "never {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(guid::unit(GIVER, 1)).await;
    (realm, a, g)
}

fn giver() -> Guid {
    guid::unit(GIVER, 1)
}

/// A log slot's three fields: the quest, its counters and state, its timer.
fn slot(m: &Map, g: Guid, slot: u16) -> (u32, u32, u32) {
    let f = &m.players[&g].fields;
    let base = PLAYER_QUEST_LOG_1_1 + slot * QUEST_LOG_ENTRY_SIZE;
    (f.get_u32(base), f.get_u32(base + 1), f.get_u32(base + 2))
}

async fn accept(a: &Client, quest_id: u32) {
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver(),
        quest_id,
    })
    .await;
}

async fn in_log(realm: &Arc<Realm>, g: Guid, quest: u32) {
    until(realm, "in the log", move |m| {
        m.players[&g].journal.has(quest).then_some(())
    })
    .await;
}

async fn give(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32) {
    on_map(realm, move |m| {
        crate::game::map::inventory::add_item(m, g, entry, count, false).expect("room")
    })
    .await;
}

fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

/// The guids of a player's `entry`, in the order they came.
fn guids(m: &Map, g: Guid, entry: u32) -> Vec<Guid> {
    m.players[&g]
        .items
        .iter()
        .filter(|i| i.entry == entry)
        .map(|i| i.guid)
        .collect()
}

/// Wait for this client's packets so far to have been handled: its status
/// query answered.
async fn handled(a: &mut Client) {
    a.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver() }).await;
    a.wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(_) => Some(()),
        _ => None,
    })
    .await;
}

async fn request_items(a: &mut Client, quest_id: u32) -> Option<QuestCompletable> {
    a.send(CMSG_QUESTGIVER_COMPLETE_QUEST {
        guid: giver(),
        quest_id,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_REQUEST_ITEMS(r) if r.quest_id == quest_id => {
            Some(r.completable)
        }
        _ => None,
    })
    .await
}

async fn offered(a: &mut Client, quest_id: u32) -> bool {
    a.send(CMSG_QUESTGIVER_REQUEST_REWARD {
        guid: giver(),
        quest_id,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_OFFER_REWARD(r) => {
            (r.quest_id == quest_id).then_some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

async fn rewarded(a: &mut Client, quest_id: u32) -> bool {
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: giver(),
        quest_id,
        reward: 0,
    })
    .await;
    handled(a).await;
    a.gets(Duration::from_millis(1), |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_COMPLETE(r) => {
            (r.quest_id == quest_id).then_some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

const COMPLETE: u32 = 1 << 24;
const FAILED: u32 = 1 << 25;

/// Thistle Tea's `QuestLog` `add/2`, `remove/2` and `swap/3`: a quest goes
/// in the lowest free slot, an abandoned one leaves its slot empty and the
/// others where they were, slots past twenty are nothing, and a twenty-first
/// quest finds the log full.
#[tokio::test]
async fn the_quest_log_keeps_its_slots() {
    let (realm, mut a, g) = setup().await;
    for q in [1, 2] {
        accept(&a, q).await;
        in_log(&realm, g, q).await;
    }
    let both = on_map(&realm, move |m| (slot(m, g, 0).0, slot(m, g, 1).0)).await;
    assert_eq!(both, (1, 2));
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: 0 }).await;
    until(&realm, "abandoned", move |m| {
        (!m.players[&g].journal.has(1)).then_some(())
    })
    .await;
    let after = on_map(&realm, move |m| (slot(m, g, 0), slot(m, g, 1).0)).await;
    assert_eq!(after, ((0, 0, 0), 2), "an empty slot, and 2 where it was");
    accept(&a, 3).await;
    in_log(&realm, g, 3).await;
    let reused = on_map(&realm, move |m| (slot(m, g, 0).0, slot(m, g, 1).0)).await;
    assert_eq!(reused, (3, 2), "the cleared slot taken again");
    // Abandoning an empty slot, and swapping past the log, change nothing.
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: 7 }).await;
    a.send(CMSG_QUESTLOG_SWAP_QUEST {
        slot1: 0,
        slot2: 20,
    })
    .await;
    accept(&a, 4).await;
    in_log(&realm, g, 4).await;
    let still = on_map(&realm, move |m| {
        (0..4).map(|s| slot(m, g, s).0).collect::<Vec<_>>()
    })
    .await;
    assert_eq!(still, vec![3, 2, 4, 0]);
    // Seventeen more fill it; a twenty-first doesn't go in.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        for q in 100..117 {
            p.journal
                .add(crate::game::store::Logged {
                    quest: q,
                    ..Default::default()
                })
                .unwrap();
        }
    })
    .await;
    a.clear();
    accept(&a, 30).await;
    a.wait_for("SMSG_QUESTLOG_FULL", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUESTLOG_FULL).then_some(())
    })
    .await;
    let full = on_map(&realm, move |m| {
        (m.players[&g].journal.count(), m.players[&g].journal.has(30))
    })
    .await;
    assert_eq!(full, (20, false));
}

/// Thistle Tea's `slot_binary/1` and `fail/2`: the counters six bits each
/// and capped at 63, the state's complete and failed bits, a failed
/// quest's timer of 1; a failure told with the timer for a timed quest,
/// and not for one missing, failed or done.
#[tokio::test]
async fn the_quest_log_fields_and_failures() {
    let (realm, mut a, g) = setup().await;
    let fields = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let logged = |quest, kills, failed| crate::game::store::Logged {
            quest,
            kills,
            failed,
            ..Default::default()
        };
        // 20 wants two boars; 100 nothing (done); 30 two scales.
        p.journal.log = vec![
            logged(30, [0; 4], false),
            logged(20, [3, 1, 63, 2], false),
            logged(20, [100, 0, 0, 0], false),
            logged(100, [0; 4], false),
            logged(30, [0; 4], true),
        ];
        quests::refresh_log(p, &d);
        (0..5).map(|s| slot(m, g, s)).collect::<Vec<_>>()
    })
    .await;
    assert_eq!(fields[0], (30, 0, 0), "nothing counted");
    assert_eq!(
        fields[1].1,
        3 | (1 << 6) | (63 << 12) | (2 << 18) | COMPLETE,
        "six bits a counter"
    );
    assert_eq!(fields[2].1 & 0xFF_FFFF, 63, "a count past 63 shown as 63");
    assert_eq!(fields[3], (100, COMPLETE, 0));
    assert_eq!(fields[4], (30, FAILED, 1), "failed, and its timer 1");
    // Failing: a timed quest says so with its timer, an untimed one plainly.
    let failed = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log = vec![
            crate::game::store::Logged {
                quest: 30,
                expires: crate::game::social::mail::now() + 600,
                ..Default::default()
            },
            crate::game::store::Logged {
                quest: 20,
                ..Default::default()
            },
            crate::game::store::Logged {
                quest: 100,
                ..Default::default()
            },
        ];
        let timed = quests::fail(m, g, 30);
        let again = quests::fail(m, g, 30);
        let plain = quests::fail(m, g, 20);
        let missing = quests::fail(m, g, 1);
        let done = quests::fail(m, g, 100);
        let p = &m.players[&g];
        (
            (timed, again, plain, missing, done),
            slot(m, g, 0),
            p.journal.log[2].failed,
            slot(m, g, 2).1,
        )
    })
    .await;
    assert_eq!(failed.0, (true, false, true, false, false));
    assert_eq!(failed.1, (30, FAILED, 1));
    assert!(!failed.2, "a done quest isn't failed back");
    assert_eq!(failed.3, COMPLETE);
    a.wait_for("SMSG_QUESTUPDATE_FAILEDTIMER", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILEDTIMER(f) => (f.quest_id == 30).then_some(()),
        _ => None,
    })
    .await;
    a.wait_for("SMSG_QUESTUPDATE_FAILED", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILED(f) => (f.quest_id == 20).then_some(()),
        _ => None,
    })
    .await;
    let others = a
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILED(f) => Some(f.quest_id),
            ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILEDTIMER(f) => Some(f.quest_id),
            _ => None,
        })
        .await;
    assert_eq!(others, None, "one failure each");
}

fn kills(a: &mut Client) -> Vec<u32> {
    let mut out = Vec::new();
    for m in std::mem::take(&mut a.backlog) {
        if let ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) = &m {
            out.push(k.kill_count);
        } else {
            a.backlog.push(m);
        }
    }
    out
}

/// Thistle Tea's `increment_kill/3`: no credit for a quest not taken, a
/// kill counted to the required count and no further, and none for a quest
/// done.
#[tokio::test]
async fn kill_credit_stops_at_the_count() {
    let (realm, mut a, g) = setup().await;
    let boar = guid::unit(BOAR, 77);
    on_map(&realm, move |m| quests::killed(m, g, boar)).await;
    accept(&a, 20).await;
    in_log(&realm, g, 20).await;
    for _ in 0..3 {
        on_map(&realm, move |m| quests::killed(m, g, boar)).await;
    }
    a.wait_for("SMSG_QUESTUPDATE_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_COMPLETE(c) => (c.quest_id == 20).then_some(()),
        _ => None,
    })
    .await;
    let _ = a.gets(Duration::from_millis(50), |_| None::<()>).await;
    assert_eq!(kills(&mut a), vec![1, 2], "two kills counted, then none");
    let counted = on_map(&realm, move |m| m.players[&g].journal.log[0].kills[0]).await;
    assert_eq!(counted, 2);
}

/// Thistle Tea's `evaluate/3`: a quest whose items are lost is under way
/// again; an exploration with items wants both; a creature's and an
/// object's objectives keep their own counters; a standing objective comes
/// and goes with the standing.
#[tokio::test]
async fn a_quest_comes_undone_and_wants_all_it_asks() {
    let (realm, mut a, g) = setup().await;
    for q in [30, 31, 32, 33] {
        accept(&a, q).await;
        in_log(&realm, g, q).await;
    }
    let state = |q: u32| {
        move |m: &mut Map| {
            let s = m.players[&g].journal.slot_of(q).unwrap() as u16;
            Some(slot(m, g, s).1)
        }
    };
    give(&realm, g, SCALE, 1).await;
    // Explored, but one scale short.
    on_map(&realm, move |m| quests::explored(m, g, 77)).await;
    assert_eq!(on_map(&realm, state(31)).await.unwrap() & COMPLETE, 0);
    give(&realm, g, SCALE, 1).await;
    assert_ne!(on_map(&realm, state(30)).await.unwrap() & COMPLETE, 0);
    assert_ne!(on_map(&realm, state(31)).await.unwrap() & COMPLETE, 0);
    // One of the two scales goes: both quests are under way again.
    on_map(&realm, move |m| {
        let p = &m.players[&g];
        let i = p.items.iter().position(|i| i.entry == SCALE).unwrap();
        let at = (p.items[i].bag, p.items[i].slot);
        crate::game::map::inventory::destroy(m, g, at, 1);
    })
    .await;
    assert_eq!(on_map(&realm, state(30)).await.unwrap() & COMPLETE, 0);
    assert_eq!(on_map(&realm, state(31)).await.unwrap() & COMPLETE, 0);
    // The spell at the boar counts in the first counter, the stone in the
    // third.
    let boar = guid::unit(BOAR, 78);
    let stone = guid::game_object(STONE, 5);
    on_map(&realm, move |m| quests::cast_credit(m, g, &[boar], 17_166)).await;
    assert_eq!(on_map(&realm, state(32)).await, Some(1));
    on_map(&realm, move |m| quests::object_used(m, g, stone, false)).await;
    assert_eq!(
        on_map(&realm, state(32)).await,
        Some(1 | (1 << 12) | COMPLETE)
    );
    // A standing reached, then lost.
    let rep = on_map(&realm, move |m| {
        let s = m.players[&g].journal.slot_of(33).unwrap() as u16;
        crate::game::map::reputation::modify(m, g, STORMWIND, 3000, false);
        let reached = slot(m, g, s).1;
        crate::game::map::reputation::modify(m, g, STORMWIND, -1, false);
        (reached, slot(m, g, s).1)
    })
    .await;
    assert_eq!(rep, (COMPLETE, 0));
    // A scale's progress is told as the scales that count.
    a.clear();
    give(&realm, g, SCALE, 3).await;
    let told = a
        .wait_for("SMSG_QUESTUPDATE_ADD_ITEM", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_ITEM(u) => Some(u.items_required),
            _ => None,
        })
        .await;
    assert_eq!(told, 1, "one more of the two counts");
    let stacks = on_map(&realm, move |m| guids(m, g, SCALE).len()).await;
    assert_eq!(stacks, 1, "onto the stack, no new item");
}

/// What givers no one stands beside show a level 1 human warrior (Thistle
/// Tea's `QuestDialogStatus`): a quest two levels off as unavailable, but
/// not one whose condition fails or for another race; one under way at its
/// ender, not its giver; an auto-complete quest as done, blue when it
/// repeats; a condition with no row as unmet; an event's quests, taken or
/// not, only while it runs; an ender's quest whatever its start condition.
#[tokio::test]
async fn a_giver_shows_what_there_is_to_take_and_turn_in() {
    let (realm, _a, g) = setup().await;
    let at = |n: u32| guid::unit(SHOWS + n, 1);
    let look = move |m: &mut Map, n: u32| {
        let icons: Vec<(u32, u32)> = quests::giver_quests(m, g, at(n))
            .iter()
            .map(|q| (q.quest_id, q.quest_icon))
            .collect();
        (quests::status(m, g, at(n)), icons)
    };
    let shown = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        for q in [44, 45, 49, 50] {
            p.journal.add(crate::game::store::Logged {
                quest: q,
                ..Default::default()
            });
        }
        (0..11).map(|n| look(m, n)).collect::<Vec<_>>()
    })
    .await;
    use QuestGiverStatus as S;
    assert_eq!(shown[0], (S::Unavailable, vec![]), "two levels short");
    assert_eq!(shown[1], (S::None, vec![]), "its condition hides it");
    assert_eq!(shown[2], (S::Unavailable, vec![]), "its condition met");
    assert_eq!(shown[3], (S::None, vec![]), "for orcs");
    assert_eq!(shown[4], (S::None, vec![]), "under way, at its giver");
    assert_eq!(shown[5], (S::Reward2, vec![(45, 4)]), "done, at its ender");
    assert_eq!(shown[6], (S::Reward2, vec![(46, 4)]), "auto-complete");
    assert_eq!(shown[7], (S::RewardRep, vec![(47, 4)]), "it repeats");
    assert_eq!(shown[8], (S::None, vec![]), "a condition with no row");
    assert_eq!(shown[9], (S::None, vec![]), "its event isn't on");
    assert_eq!(shown[10], (S::Incomplete, vec![(50, 3)]), "an ender's");
    let ender = on_map(&realm, move |m| look(m, 20)).await;
    assert_eq!(
        ender,
        (S::Incomplete, vec![(44, 3)]),
        "under way, at its ender"
    );
    // Rewarded: the once is done with, the repeatable still there.
    let rewarded = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.journal.rewarded.extend([46, 47]);
        (look(m, 6), look(m, 7))
    })
    .await;
    assert_eq!(rewarded.0, (S::None, vec![]));
    assert_eq!(rewarded.1, (S::RewardRep, vec![(47, 4)]));
    // The event on: its quest is shown, its progress kept.
    let event = on_map(&realm, move |m| {
        m.realm.social.events.lock().insert(EVENT);
        let shown = look(m, 9);
        m.realm.social.events.lock().remove(&EVENT);
        (shown, m.players[&g].journal.has(49))
    })
    .await;
    assert_eq!(event, ((S::Incomplete, vec![(49, 3)]), true));
}

/// Thistle Tea's `can_take`: the race and class masks; and a met condition
/// leaves the level's refusal as it is.
#[tokio::test]
async fn a_quest_is_for_its_races_and_classes() {
    let (realm, mut a, g) = setup().await;
    let takes = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        [51, 52, 53, 54].map(|id| quests::can_take(p, &d.npc.quests[&id], &d))
    })
    .await;
    assert_eq!(
        takes,
        [false, true, false, true],
        "orcs, humans; warlocks, warriors"
    );
    // 42 asks for level 3, its condition met: too low, as told.
    accept(&a, 42).await;
    let why = a
        .wait_for("SMSG_QUESTGIVER_QUEST_INVALID", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(i) => Some(i.msg),
            _ => None,
        })
        .await;
    assert_eq!(why, QuestFailedReason::QuestFailedLowLevel);
    // For orcs: the race told; for warlocks, the requirements.
    for (q, told) in [
        (51, QuestFailedReason::QuestFailedWrongRace),
        (53, QuestFailedReason::DontHaveReq),
    ] {
        accept(&a, q).await;
        let why = a
            .wait_for("SMSG_QUESTGIVER_QUEST_INVALID", |m| match m {
                ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(i) => Some(i.msg),
                _ => None,
            })
            .await;
        assert_eq!(why, told, "quest {q}");
    }
    // A giver no longer a quest giver lists nothing and gives nothing.
    let listed = on_map(&realm, move |m| {
        let before = quests::giver_quests(m, g, giver()).len();
        let c = m.creature_mut(giver()).unwrap();
        c.fields.set_u32(UNIT_NPC_FLAGS, 0);
        (before, quests::giver_quests(m, g, giver()).len())
    })
    .await;
    assert!(listed.0 > 0);
    assert_eq!(listed.1, 0);
    accept(&a, 1).await;
    handled(&mut a).await;
    assert!(!on_map(&realm, move |m| m.players[&g].journal.has(1)).await);
}

/// Thistle Tea's auto-complete quests: turned in without being taken, once
/// the items are carried, and not again.
#[tokio::test]
async fn an_auto_complete_quest_is_turned_in_without_the_log() {
    let (realm, mut a, g) = setup().await;
    assert_eq!(
        request_items(&mut a, 61).await,
        Some(QuestCompletable::NotCompletable)
    );
    assert!(!offered(&mut a, 61).await);
    assert!(!rewarded(&mut a, 61).await);
    give(&realm, g, SCALE, 2).await;
    assert_eq!(
        request_items(&mut a, 61).await,
        Some(QuestCompletable::Completeable)
    );
    assert!(offered(&mut a, 61).await);
    assert!(rewarded(&mut a, 61).await);
    let after = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.journal.rewarded.contains(&61),
            p.journal.count(),
            count(m, g, SCALE),
            count(m, g, PELT),
        )
    })
    .await;
    assert_eq!(after, (true, 0, 0, 1), "taken, rewarded, never in the log");
    assert_eq!(request_items(&mut a, 61).await, None, "done with");
}

/// Thistle Tea's `delivered?/3` and the turn-in's commit: the items and
/// the money a turn-in costs, all taken with every reward given, or
/// nothing.
#[tokio::test]
async fn a_turn_in_takes_its_items_and_money() {
    let (realm, mut a, g) = setup().await;
    accept(&a, 60).await;
    in_log(&realm, g, 60).await;
    give(&realm, g, SCALE, 2).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(99)
    })
    .await;
    assert!(!rewarded(&mut a, 60).await, "a copper short");
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().set_money(100)
    })
    .await;
    assert!(rewarded(&mut a, 60).await);
    let after = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.journal.rewarded.contains(&60),
            p.journal.has(60),
            count(m, g, SCALE),
            count(m, g, PELT),
            p.money(),
        )
    })
    .await;
    assert_eq!(after, (true, false, 0, 1, 0));
}

/// Thistle Tea's `QuestItems`: a quest gives what of its items isn't
/// carried or banked, and takes them back from both when abandoned, but
/// not one that starts it, nor a bag with something in it; a starter is
/// used up, that one copy, unless the quest gives it or wants it; and
/// with no room for what it gives, a quest isn't taken.
#[tokio::test]
async fn a_quest_gives_and_takes_back_its_items() {
    let (realm, mut a, g) = setup().await;
    // Three bombs, two of them banked: two more make the five it gives.
    give(&realm, g, BOMB, 1).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let bank = crate::game::store::ItemRow {
            guid: 900_001,
            entry: BOMB,
            bag: crate::game::store::BAG_PLAYER,
            slot: crate::game::map::inventory::BANK_START,
            count: 2,
            ..Default::default()
        };
        p.items
            .push(crate::game::map::player::Item::from_row(&d, &bank, g));
    })
    .await;
    accept(&a, 72).await;
    in_log(&realm, g, 72).await;
    assert_eq!(on_map(&realm, move |m| count(m, g, BOMB)).await, 5);
    let slot72 = on_map(&realm, move |m| {
        m.players[&g].journal.slot_of(72).unwrap() as u8
    })
    .await;
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: slot72 }).await;
    until(&realm, "72 abandoned", move |m| {
        (!m.players[&g].journal.has(72)).then_some(())
    })
    .await;
    assert_eq!(
        on_map(&realm, move |m| count(m, g, BOMB)).await,
        0,
        "taken from the bags and the bank"
    );
    // The box 71 gives goes back with it.
    accept(&a, 71).await;
    in_log(&realm, g, 71).await;
    assert_eq!(on_map(&realm, move |m| count(m, g, BOX)).await, 1);
    let s = on_map(&realm, move |m| {
        m.players[&g].journal.slot_of(71).unwrap() as u8
    })
    .await;
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: s }).await;
    until(&realm, "71 abandoned", move |m| {
        (count(m, g, BOX) == 0 && !m.players[&g].journal.has(71)).then_some(())
    })
    .await;
    // The map starts 70 and is what it gives: kept on taking it, and on
    // abandoning it.
    give(&realm, g, MAP, 1).await;
    let map_item = on_map(&realm, move |m| guids(m, g, MAP)[0]).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: map_item,
        quest_id: 70,
    })
    .await;
    in_log(&realm, g, 70).await;
    assert_eq!(
        on_map(&realm, move |m| count(m, g, MAP)).await,
        1,
        "one map"
    );
    let s = on_map(&realm, move |m| {
        m.players[&g].journal.slot_of(70).unwrap() as u8
    })
    .await;
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: s }).await;
    until(&realm, "70 abandoned", move |m| {
        (!m.players[&g].journal.has(70)).then_some(())
    })
    .await;
    assert_eq!(on_map(&realm, move |m| count(m, g, MAP)).await, 1, "still");
    // Two letters: the one 75 is taken from goes, banked as it is, and the
    // other stays; a letter doesn't start another quest.
    give(&realm, g, LETTER, 2).await;
    let letters = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.items.iter().rposition(|i| i.entry == LETTER).unwrap();
        p.items[i].slot = crate::game::map::inventory::BANK_START + 1;
        guids(m, g, LETTER)
    })
    .await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: letters[0],
        quest_id: 76,
    })
    .await;
    handled(&mut a).await;
    assert!(!on_map(&realm, move |m| m.players[&g].journal.has(76)).await);
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: letters[1],
        quest_id: 75,
    })
    .await;
    in_log(&realm, g, 75).await;
    let left = until(&realm, "a letter used", move |m| {
        let left = guids(m, g, LETTER);
        (left.len() == 1).then_some(left)
    })
    .await;
    assert_eq!(left, vec![letters[0]]);
    // Orders that 76 wants are kept.
    give(&realm, g, ORDERS, 1).await;
    let orders = on_map(&realm, move |m| guids(m, g, ORDERS)[0]).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: orders,
        quest_id: 76,
    })
    .await;
    in_log(&realm, g, 76).await;
    assert_eq!(on_map(&realm, move |m| count(m, g, ORDERS)).await, 1);
    // The satchel 73 gives, worn with something in it, keeps the quest.
    accept(&a, 73).await;
    in_log(&realm, g, 73).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.items.iter().position(|i| i.entry == SATCHEL).unwrap();
        p.items[i].slot = crate::game::map::inventory::BAG_START;
        let b = p.items.iter().position(|i| i.entry == ORDERS).unwrap();
        p.items[b].bag = crate::game::map::inventory::BAG_START;
        p.items[b].slot = 0;
    })
    .await;
    a.clear();
    let s = on_map(&realm, move |m| {
        m.players[&g].journal.slot_of(73).unwrap() as u8
    })
    .await;
    a.send(CMSG_QUESTLOG_REMOVE_QUEST { slot: s }).await;
    let why = a
        .wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    assert!(why.starts_with("CanOnlyDoWithEmptyBags"), "{why}");
    assert!(on_map(&realm, move |m| m.players[&g].journal.has(73)).await);
    // Full bags: 74's pelts don't fit, so it isn't taken; room, and it is.
    let free = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        p.carry_slots(&d)
            .iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count() as u32
    })
    .await;
    give(&realm, g, FILLER, free).await;
    a.clear();
    accept(&a, 74).await;
    let why = a
        .wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    assert!(why.starts_with("InventoryFull"), "{why}");
    assert!(!on_map(&realm, move |m| m.players[&g].journal.has(74)).await);
    on_map(&realm, move |m| {
        let p = &m.players[&g];
        let i = p.items.iter().position(|i| i.entry == FILLER).unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
    accept(&a, 74).await;
    in_log(&realm, g, 74).await;
    assert_eq!(on_map(&realm, move |m| count(m, g, PELT)).await, 1);
}

/// Thistle Tea's raid quests: in a raid, only raid quests' drops and kills
/// count, a spell's credit is for creatures alone, and outside it all
/// count again.
#[tokio::test]
async fn in_a_raid_only_raid_quests_count() {
    let (realm, a, g) = setup().await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Priest).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        for q in [80, 81, 82] {
            p.journal.add(crate::game::store::Logged {
                quest: q,
                ..Default::default()
            });
        }
    })
    .await;
    until(&realm, "a group", move |m| {
        m.realm.social.groups.lock().group_of(g).map(|_| ())
    })
    .await;
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    until(&realm, "a raid", move |m| {
        quests::in_raid(m, g).then_some(())
    })
    .await;
    let boar = guid::unit(BOAR, 90);
    let raid = on_map(&realm, move |m| {
        let wanted = quests::wanted(m, g);
        quests::killed(m, g, boar);
        quests::cast_credit(m, g, &[g], 10_292);
        let on_self = m.players[&g].journal.log[2].kills[0];
        quests::cast_credit(m, g, &[g, boar], 10_292);
        let l = &m.players[&g].journal.log;
        (wanted, l[0].kills[0], l[1].kills[0], on_self, l[2].kills[0])
    })
    .await;
    assert_eq!(raid, (vec![PELT], 0, 1, 0, 1));
    b.send(CMSG_GROUP_DISBAND {}).await;
    until(&realm, "out of the raid", move |m| {
        (!quests::in_raid(m, g)).then_some(())
    })
    .await;
    let after = on_map(&realm, move |m| {
        let mut wanted = quests::wanted(m, g);
        wanted.sort();
        quests::killed(m, g, boar);
        (wanted, m.players[&g].journal.log[0].kills[0])
    })
    .await;
    assert_eq!(after, (vec![SCALE, PELT], 1));
    // A scale had, it's wanted no more.
    give(&realm, g, SCALE, 1).await;
    let wanted = on_map(&realm, move |m| quests::wanted(m, g)).await;
    assert_eq!(wanted, vec![PELT]);
}

/// `CMSG_QUESTGIVER_QUERY_QUEST` from `from`: how many details came, and
/// whether the dialog closed.
async fn details(a: &mut Client, from: Guid, quest_id: u32) -> (usize, bool) {
    a.clear();
    a.raw.clear();
    a.send(CMSG_QUESTGIVER_QUERY_QUEST {
        guid: from,
        quest_id,
    })
    .await;
    handled(a).await;
    let opcode = quests::SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE;
    let raw = a
        .raw
        .iter()
        .filter(|f| u16::from_le_bytes([f[2], f[3]]) == opcode)
        .count();
    let parsed = a
        .backlog
        .iter()
        .filter(|m| matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(_)))
        .count();
    let closed = a
        .backlog
        .iter()
        .any(|m| matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE));
    (raw + parsed, closed)
}

/// Thistle Tea's item quests and dialogs: a cancel closes the dialog; a
/// quest's details only from an item of the player's that starts it, or a
/// giver of it; a second copy of a starter doesn't take the quest again
/// and stays; a full log and the dead take nothing, the starter kept.
#[tokio::test]
async fn quest_dialogs_check_the_item_and_the_taker() {
    let (realm, mut a, g) = setup().await;
    let closed = |m: &ServerOpcodeMessage| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    };
    a.send(CMSG_QUESTGIVER_CANCEL {}).await;
    a.wait_for("SMSG_GOSSIP_COMPLETE", closed).await;
    give(&realm, g, LETTER, 2).await;
    let letters = on_map(&realm, move |m| guids(m, g, LETTER)).await;
    assert_eq!(details(&mut a, giver(), 1).await, (1, false), "its giver's");
    assert_eq!(details(&mut a, giver(), 999).await, (0, true), "not its");
    assert_eq!(
        details(&mut a, letters[0], 75).await,
        (1, false),
        "the letter's"
    );
    assert_eq!(
        details(&mut a, letters[0], 76).await,
        (0, true),
        "not the letter's"
    );
    // The dead take nothing, from a giver or a letter.
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Dead {
            since: std::time::Instant::now(),
        };
    })
    .await;
    accept(&a, 1).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: letters[0],
        quest_id: 75,
    })
    .await;
    handled(&mut a).await;
    let dead = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let took = p.journal.count();
        p.death = crate::game::map::death::Death::Alive;
        (took, guids(m, g, LETTER).len())
    })
    .await;
    assert_eq!(dead, (0, 2));
    // Taken from one letter; the other doesn't take it again, and stays.
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: letters[0],
        quest_id: 75,
    })
    .await;
    in_log(&realm, g, 75).await;
    a.clear();
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: letters[1],
        quest_id: 75,
    })
    .await;
    let why = a
        .wait_for("SMSG_QUESTGIVER_QUEST_INVALID", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(i) => Some(i.msg),
            _ => None,
        })
        .await;
    assert_eq!(why, QuestFailedReason::QuestAlreadyOn);
    assert_eq!(
        on_map(&realm, move |m| guids(m, g, LETTER)).await,
        vec![letters[1]]
    );
    // A full log: 76's orders stay where they are.
    give(&realm, g, ORDERS, 1).await;
    let orders = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        for q in 100..119 {
            p.journal.add(crate::game::store::Logged {
                quest: q,
                ..Default::default()
            });
        }
        guids(m, g, ORDERS)[0]
    })
    .await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: orders,
        quest_id: 76,
    })
    .await;
    a.wait_for("SMSG_QUESTLOG_FULL", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUESTLOG_FULL).then_some(())
    })
    .await;
    assert_eq!(on_map(&realm, move |m| count(m, g, ORDERS)).await, 1);
}

/// The results the sharer hears on sharing `quest`.
async fn share(a: &mut Client, quest_id: u32) -> Vec<QuestPartyMessage> {
    a.clear();
    a.send(CMSG_PUSHQUESTTOPARTY { quest_id }).await;
    handled(a).await;
    let mut out = Vec::new();
    for m in std::mem::take(&mut a.backlog) {
        if let ServerOpcodeMessage::MSG_QUEST_PUSH_RESULT(r) = m {
            out.push(r.message);
        }
    }
    out
}

/// Thistle Tea's `QuestSharing` (and mangos's `HandlePushQuestToParty`):
/// only a shareable quest under way is shared, the sharer told of each
/// member first and then why not: too far, finished, on it or done with
/// it, unable to take it, a full log, an offer already; taken, it has the
/// items it gives and the sharer's time left, and taken from afar, it
/// isn't.
#[tokio::test]
async fn a_quest_is_shared_as_mangos_shares_it() {
    use QuestPartyMessage as R;
    let (realm, mut a, ga) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Priest).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    until(&realm, "a group", move |m| {
        m.realm.social.groups.lock().group_of(ga).map(|_| ())
    })
    .await;
    let expires = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let expires = crate::game::social::mail::now() + 300;
        for (quest, expires) in [(90, expires), (91, 0), (92, 0)] {
            p.journal.add(crate::game::store::Logged {
                quest,
                expires,
                ..Default::default()
            });
        }
        expires
    })
    .await;
    assert_eq!(share(&mut a, 91).await, vec![], "not shareable");
    let set = |f: fn(&mut crate::game::map::player::Player)| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| f(m.players.get_mut(&gb).unwrap())).await;
        }
    };
    set(|p| p.movement.position.x += 15.0).await;
    assert_eq!(share(&mut a, 90).await, vec![R::SharingQuest, R::TooFar]);
    set(|p| p.movement.position.x -= 15.0).await;
    give(&realm, gb, SCALE, 1).await;
    set(|p| {
        p.journal.add(crate::game::store::Logged {
            quest: 90,
            ..Default::default()
        });
    })
    .await;
    assert_eq!(
        share(&mut a, 90).await,
        vec![R::SharingQuest, R::FinishQuest]
    );
    set(|p| {
        p.journal.clear(0);
        p.journal.rewarded.insert(90);
    })
    .await;
    assert_eq!(share(&mut a, 90).await, vec![R::SharingQuest, R::HaveQuest]);
    set(|p| {
        p.journal.rewarded.clear();
    })
    .await;
    assert_eq!(
        share(&mut a, 92).await,
        vec![R::SharingQuest, R::CantTakeQuest]
    );
    set(|p| {
        for q in 100..120 {
            p.journal.add(crate::game::store::Logged {
                quest: q,
                ..Default::default()
            });
        }
    })
    .await;
    assert_eq!(share(&mut a, 90).await, vec![R::SharingQuest, R::LogFull]);
    set(|p| {
        p.journal.log.clear();
        p.misc.quest_shared = Some((Guid::new(1), 1));
    })
    .await;
    assert_eq!(share(&mut a, 90).await, vec![R::SharingQuest, R::Busy]);
    set(|p| p.misc.quest_shared = None).await;
    assert_eq!(share(&mut a, 90).await, vec![R::SharingQuest]);
    // Taken from afar, it isn't; the sharer hears.
    set(|p| p.movement.position.x += 15.0).await;
    b.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: ga,
        quest_id: 90,
    })
    .await;
    let why = a
        .wait_for("MSG_QUEST_PUSH_RESULT", |m| match m {
            ServerOpcodeMessage::MSG_QUEST_PUSH_RESULT(r) => Some(r.message),
            _ => None,
        })
        .await;
    assert_eq!(why, R::TooFar);
    let gone = on_map(&realm, move |m| {
        let p = &m.players[&gb];
        (p.journal.has(90), p.misc.quest_shared)
    })
    .await;
    assert_eq!(gone, (false, None));
    // At hand: taken, with its bombs and Alice's time.
    set(|p| p.movement.position.x -= 15.0).await;
    assert_eq!(share(&mut a, 90).await, vec![R::SharingQuest]);
    b.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: ga,
        quest_id: 90,
    })
    .await;
    in_log(&realm, gb, 90).await;
    let taken = on_map(&realm, move |m| {
        let p = &m.players[&gb];
        let l = &p.journal.log[p.journal.slot_of(90).unwrap()];
        (l.expires, count(m, gb, BOMB))
    })
    .await;
    assert_eq!(taken, (expires, 2));
}

/// The quest list a hello at the giver brings, as `d` has it: its title,
/// emote delay and emote.
async fn quest_list(d: GameData) -> (String, u32, u32) {
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(giver()).await;
    a.send(CMSG_QUESTGIVER_HELLO { guid: giver() }).await;
    a.wait_for("SMSG_QUESTGIVER_QUEST_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_LIST(l) => {
            Some((l.title.clone(), l.emote_delay, l.emote))
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's `greeting/2`: the giver's quest greeting, else its gossip
/// text, else nothing for the default text.
#[tokio::test]
async fn a_quest_list_greets_with_the_givers_words() {
    let mut d = data();
    d.npc
        .quest_greetings
        .insert((GIVER, 0), ("Hello there, $c.".into(), 1, 0));
    assert_eq!(quest_list(d).await, ("Hello there, $c.".into(), 0, 1));
    let with_menu = |text: u32| {
        let mut d = data();
        d.creature_templates.get_mut(&GIVER).unwrap().gossip_menu_id = 500;
        d.npc.gossip_menus.insert(
            500,
            crate::game::data::npc::GossipMenu {
                texts: vec![(text, 0)],
                options: Vec::new(),
            },
        );
        let mut choices: [crate::game::data::npc::NpcTextChoice; 8] = Default::default();
        choices[0].broadcast = 7000;
        choices[0].probability = 1.0;
        d.npc.npc_texts.insert(text, choices);
        d.npc.broadcast_texts.insert(
            7000,
            crate::game::data::npc::BroadcastText {
                female: "Welcome, traveler.".into(),
                emotes: [(100, 2), (0, 0), (0, 0)],
                ..Default::default()
            },
        );
        d
    };
    assert_eq!(
        quest_list(with_menu(6000)).await,
        ("Welcome, traveler.".into(), 100, 2)
    );
    assert_eq!(
        quest_list(with_menu(crate::game::map::npc::DEFAULT_TEXT)).await,
        (String::new(), 0, 0)
    );
}

/// Quests depending on one another, for Thistle Tea's `QuestDependencies`.
fn graph_data() -> GameData {
    let mut d = data();
    let mut put = |q: Quest| {
        d.npc.quests.insert(q.id, q);
    };
    // 201 or 202 rewarded, or 203 under way, before 204.
    put(Quest {
        next_quest: 204,
        ..quest(201)
    });
    put(Quest {
        next_quest: 204,
        ..quest(202)
    });
    put(Quest {
        next_quest: -204,
        special_flags: 1,
        required_items: [(SCALE, 1), (0, 0), (0, 0), (0, 0)],
        ..quest(203)
    });
    put(quest(204));
    // One of 211, 212 and 213 (which repeats).
    for (id, flags) in [(211, 0), (212, 0), (213, 1)] {
        put(Quest {
            exclusive_group: 1,
            special_flags: flags,
            ..quest(id)
        });
    }
    // A breadcrumb to a quest whose condition fails, one at level 3.
    put(Quest {
        breadcrumb_for: 222,
        min_level: 3,
        ..quest(221)
    });
    put(quest(222));
    // A breadcrumb that repeats; one to itself.
    put(Quest {
        breadcrumb_for: 232,
        special_flags: 1,
        ..quest(231)
    });
    put(quest(232));
    put(Quest {
        breadcrumb_for: 241,
        ..quest(241)
    });
    // Cooking 50.
    put(Quest {
        required_skill: 185,
        required_skill_value: 50,
        ..quest(251)
    });
    put(Quest {
        required_skill: 185,
        required_skill_value: 0,
        ..quest(252)
    });
    d.scripts.quest_condition.insert(222, LEVEL_FIVE);
    d.npc.creature_starts.insert(SHOWS + 30, vec![221]);
    d
}

/// Thistle Tea's `QuestDependencies`: any rewarded alternative before a
/// quest, or one under way for a negative one (not failed, nor done and
/// rewarded before); a repeating breadcrumb blocking its quest while on it;
/// a breadcrumb to itself never taken; a breadcrumb whose quest's condition
/// fails not offered, nor shown a few levels off; a skill with its bonuses,
/// only if known.
#[tokio::test]
async fn quests_wait_on_what_they_depend_on() {
    let realm = harness::realm_with(graph_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    // (logged, failed, rewarded, quests asked about)
    let takes = |log: &'static [(u32, bool)], rewarded: &'static [u32], ids: &'static [u32]| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                let d = m.realm.data.clone();
                let p = m.players.get_mut(&g).unwrap();
                p.journal.log = log
                    .iter()
                    .map(|(quest, failed)| crate::game::store::Logged {
                        quest: *quest,
                        failed: *failed,
                        ..Default::default()
                    })
                    .collect();
                p.journal.rewarded = rewarded.iter().copied().collect();
                let p = &m.players[&g];
                ids.iter()
                    .map(|id| quests::can_take(p, &d.npc.quests[id], &d))
                    .collect::<Vec<bool>>()
            })
            .await
        }
    };
    assert_eq!(takes(&[], &[], &[204]).await, vec![false]);
    assert_eq!(takes(&[], &[201], &[204]).await, vec![true]);
    assert_eq!(takes(&[], &[202], &[204]).await, vec![true]);
    assert_eq!(
        takes(&[(203, false)], &[], &[204]).await,
        vec![true],
        "203 on"
    );
    assert_eq!(
        takes(&[(203, true)], &[], &[204]).await,
        vec![false],
        "failed"
    );
    assert_eq!(
        takes(&[], &[203], &[204]).await,
        vec![false],
        "only rewarded"
    );
    assert_eq!(
        takes(&[(203, false)], &[203], &[204]).await,
        vec![true],
        "on it again"
    );
    give(&realm, g, SCALE, 1).await;
    assert_eq!(
        takes(&[(203, false)], &[203], &[204]).await,
        vec![false],
        "done again and rewarded before"
    );
    // Exclusive: one taken, done or done for good shuts out the others; a
    // repeatable one rewarded doesn't.
    assert_eq!(takes(&[(212, false)], &[], &[211]).await, vec![false]);
    assert_eq!(takes(&[(213, false)], &[213], &[211]).await, vec![false]);
    assert_eq!(takes(&[], &[212], &[211]).await, vec![false]);
    assert_eq!(takes(&[], &[213], &[211]).await, vec![true]);
    // A repeating breadcrumb on, rewarded before or not, failed or not.
    for failed in [false, true] {
        let log: &'static [(u32, bool)] = if failed {
            &[(231, true)]
        } else {
            &[(231, false)]
        };
        assert_eq!(takes(log, &[231], &[232]).await, vec![false]);
    }
    assert_eq!(takes(&[], &[231], &[232]).await, vec![true]);
    assert_eq!(takes(&[], &[], &[241]).await, vec![false], "to itself");
    // The breadcrumb to a quest whose condition fails: not offered, nor
    // shown as a few levels off.
    let shown = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().journal = Default::default();
        quests::status(m, g, guid::unit(SHOWS + 30, 1))
    })
    .await;
    assert_eq!(shown, QuestGiverStatus::None);
    // Cooking: unknown, 49, 49 with +2 -1, 50; unknown with +200.
    let skill = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let can = |m: &Map| {
            let p = &m.players[&g];
            [251, 252].map(|id| quests::can_take(p, &d.npc.quests[&id], &d))
        };
        let mut out = vec![can(m)];
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, 185, 1);
        let i = (0..128u16)
            .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == 185)
            .unwrap();
        let value = PLAYER_SKILL_INFO_1_1 + i * 3 + 1;
        let bonus = PLAYER_SKILL_INFO_1_1 + i * 3 + 2;
        p.fields.set_u32(value, (75 << 16) | 49);
        out.push(can(m));
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(bonus, (0xFFFF << 16) | 2);
        out.push(can(m));
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(bonus, 0);
        p.fields.set_u32(value, (75 << 16) | 50);
        out.push(can(m));
        out
    })
    .await;
    assert_eq!(
        skill,
        vec![[false, true], [false, true], [true, true], [true, true]]
    );
}

/// Thistle Tea's `RequiredCondition` at acceptance: a quest's details from
/// its giver whatever its condition, but not taken (DontHaveReq, then the
/// dialog closes) unless forced; once in the log, turned in whatever it;
/// a chain's next offered only if its condition holds and the giver gives
/// it.
#[tokio::test]
async fn a_quests_condition_gates_taking_it_not_turning_it_in() {
    let (realm, mut a, g) = setup().await;
    assert_eq!(details(&mut a, giver(), 85).await, (1, false), "details");
    a.clear();
    accept(&a, 85).await;
    let why = a
        .wait_for("SMSG_QUESTGIVER_QUEST_INVALID", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(i) => Some(i.msg),
            _ => None,
        })
        .await;
    assert_eq!(why, QuestFailedReason::DontHaveReq);
    a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    assert!(!on_map(&realm, move |m| m.players[&g].journal.has(85)).await);
    a.say(".addquest 85").await;
    in_log(&realm, g, 85).await;
    assert!(offered(&mut a, 85).await, "turned in all the same");
    assert!(rewarded(&mut a, 85).await);
    // The chains.
    let mut next = Vec::new();
    for (first, then) in [(86, 87), (88, 89), (95, 96)] {
        accept(&a, first).await;
        in_log(&realm, g, first).await;
        a.clear();
        a.raw.clear();
        assert!(rewarded(&mut a, first).await);
        next.push((then, details_shown(&a, then)));
    }
    assert_eq!(next, vec![(87, false), (89, true), (96, false)]);
}

/// Whether `quest`'s details are among what `a` has had and not looked
/// at, parsed or not.
fn details_shown(a: &Client, quest: u32) -> bool {
    let opcode = quests::SMSG_QUESTGIVER_QUEST_DETAILS_OPCODE;
    a.raw.iter().any(|f| {
        u16::from_le_bytes([f[2], f[3]]) == opcode
            && u32::from_le_bytes(f[12..16].try_into().unwrap()) == quest
    }) || a.backlog.iter().any(|m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(d) if d.quest_id == quest)
    })
}

/// Thistle Tea's quest giver checks (mangos's `CanInteractWithQuestGiver`):
/// a quest is taken only beside a living giver; a creature's and an
/// object's quests of the same entry are their own; an object's turn-ins
/// list one under way or done, not one failed.
#[tokio::test]
async fn a_quest_giver_must_be_at_hand_and_alive() {
    let mut d = data();
    // An object of the giver's entry number, with its own quest.
    d.npc.object_starts.insert(GIVER, vec![91]);
    d.npc.object_ends.insert(GIVER, vec![30]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(giver()).await;
    let far = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().movement.position.x += 10.0;
        quests::accept(m, g, giver(), 1);
        let far = m.players[&g].journal.has(1);
        m.players.get_mut(&g).unwrap().movement.position.x -= 10.0;
        far
    })
    .await;
    assert!(!far, "ten yards off");
    let dead = on_map(&realm, move |m| {
        crate::game::map::combat::kill(m, g, giver());
        quests::accept(m, g, giver(), 1);
        m.players[&g].journal.has(1)
    })
    .await;
    assert!(!dead, "a dead giver");
    let split = on_map(&realm, move |m| {
        let object = guid::game_object(GIVER, 9);
        let ids = |m: &Map, at: Guid| {
            quests::giver_quests(m, g, at)
                .iter()
                .map(|q| q.quest_id)
                .collect::<Vec<_>>()
        };
        let creature = guid::unit(GIVER, 99);
        let p = m.players.get_mut(&g).unwrap();
        p.journal.add(crate::game::store::Logged {
            quest: 30,
            failed: true,
            ..Default::default()
        });
        let failed = ids(m, object);
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log[0].failed = false;
        (ids(m, object), failed, ids(m, creature).contains(&91))
    })
    .await;
    assert_eq!(
        split.0,
        vec![30, 91],
        "the object's own: its turn-in, its quest"
    );
    assert_eq!(split.1, vec![91], "not a failed turn-in");
    assert!(!split.2, "the object's quest isn't the creature's");
}

/// Thistle Tea's `choose_reward/4`: rewards that don't all fit give none,
/// the quest kept; the reward's spell cast only once the items are taken,
/// and not again on a second turn-in.
#[tokio::test]
async fn a_reward_is_all_or_nothing_and_cast_once() {
    let (realm, mut a, g) = setup().await;
    accept(&a, 62).await;
    in_log(&realm, g, 62).await;
    let blessed = move |m: &mut Map| m.players[&g].auras.has_spell(BLESSING);
    assert!(!rewarded(&mut a, 62).await, "no scales");
    assert!(!on_map(&realm, blessed).await, "nor its spell");
    give(&realm, g, SCALE, 2).await;
    // The scales' slot alone for two rewards (the slot the scales leave
    // counts, as Thistle Tea's turn-in plan has it).
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        let free = p
            .carry_slots(&d)
            .iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count() as u32;
        crate::game::map::inventory::add_item(m, g, FILLER, free, false).unwrap();
    })
    .await;
    a.clear();
    assert!(!rewarded(&mut a, 62).await, "no room for both");
    let kept = on_map(&realm, move |m| {
        (
            m.players[&g].journal.has(62),
            count(m, g, SCALE),
            count(m, g, PELT) + count(m, g, BOX),
            m.players[&g].auras.has_spell(BLESSING),
        )
    })
    .await;
    assert_eq!(kept, (true, 2, 0, false));
    on_map(&realm, move |m| {
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.entry == FILLER)
            .unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
    assert!(rewarded(&mut a, 62).await);
    let done = on_map(&realm, move |m| {
        let had = m.players[&g].auras.has_spell(BLESSING);
        crate::game::map::auras::remove_where(m, g, |h| h.spell == BLESSING);
        (
            had,
            count(m, g, SCALE),
            count(m, g, PELT) + count(m, g, BOX),
        )
    })
    .await;
    assert_eq!(done, (true, 0, 2));
    assert!(!rewarded(&mut a, 62).await, "not twice");
    assert!(!on_map(&realm, blessed).await, "nor its spell again");
}
