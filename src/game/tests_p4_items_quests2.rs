//! Phase 4's quests, continued: Thistle Tea's follower tests (crediting,
//! failing, turning on the player, heading for a place), bad breadcrumbs,
//! a breadcrumb to an event's quest, sharing a repeatable or timed quest,
//! and Alterac Valley's captured animals.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::tests_p4_items_quests::{
    MONSTERS, START, UNIT_FLAG_IMMUNE_TO_PLAYER, at, dist, hurry, line, logged, place, player,
    questgiver, realm_of, said, spawn, take, template, the, unit_u32, until, world,
};

// Followers.

/// A follower of `entry` for `quest` beside `START`, and the creature of
/// `goal` (if any) `goal_east` yards east.
fn follower_world(entry: u32, quest: u32, goal: Option<u32>, goal_east: f32) -> GameData {
    let mut d = world();
    questgiver(&mut d, 0, entry, quest, (START.0 - 1.5, START.1, START.2));
    d.creature_templates.get_mut(&entry).unwrap().unit_flags = UNIT_FLAG_IMMUNE_TO_PLAYER;
    if let Some(goal) = goal {
        d.creature_templates.insert(goal, template(goal, "Goal"));
        spawn(&mut d, 0, goal, (START.0 + goal_east, START.1, START.2));
    }
    for id in [3917, 3921, 3922, 1330, 1340, 2704, 3012, 3881, 3929] {
        line(&mut d, id);
    }
    d
}

const SHAY: u32 = 7774;
const SHAY_QUEST: u32 = 2845;
const ROGI: u32 = 7765;

/// Thistle Tea's "follows the player once the accept actions run" and
/// "credits the group and plays the arrival once the goal is beside the
/// follower": Shay Leafrunner (2845) says her line, takes her faction,
/// is no longer immune to players and trails the player; beside Rogi she
/// credits the player and the group's member near, she and Rogi say their
/// lines, and she goes after her time.
#[tokio::test]
async fn a_follower_trails_the_player_and_credits_the_group_at_its_goal() {
    let realm = realm_of(follower_world(SHAY, SHAY_QUEST, Some(ROGI), 60.0));
    let (mut a, ga, map) = player(&realm, "Alice").await;
    let (_b, gb, _) = player(&realm, "Bob").await;
    let online = |g: Guid| realm.registry.online(g).unwrap();
    assert!(crate::game::social::groups::join_directly(
        &realm,
        &online(ga),
        &online(gb)
    ));
    let shay = guid::unit(SHAY, SHAY);
    let rogi = guid::unit(ROGI, ROGI);
    a.sees(shay).await;
    map.query(move |m| {
        m.players
            .get_mut(&gb)
            .unwrap()
            .journal
            .add(crate::game::store::Logged {
                quest: SHAY_QUEST,
                ..Default::default()
            });
    })
    .await;
    take(&map, ga, shay, SHAY_QUEST).await;
    assert_eq!(said(&mut a, 3921).await, shay, "her accept line");
    let set_out = map
        .query(move |m| {
            (
                unit_u32(m, shay, UNIT_FIELD_FACTIONTEMPLATE),
                unit_u32(m, shay, UNIT_FIELD_FLAGS) & UNIT_FLAG_IMMUNE_TO_PLAYER,
                unit_u32(m, shay, UNIT_NPC_FLAGS),
            )
        })
        .await
        .unwrap();
    assert_eq!(set_out, (10, 0, 0));
    // She trails Alice.
    let to = at(START.0, START.1 + 15.0, START.2);
    until(&map, "beside Alice", move |m| {
        place(m, ga, to);
        (dist(m.creature(shay)?.position, to) <= 3.5).then_some(())
    })
    .await;
    // To Rogi, Bob too: both credited.
    let by_rogi = at(START.0 + 50.0, START.1, START.2);
    until(&map, "both credited", move |m| {
        place(m, ga, by_rogi);
        place(m, gb, by_rogi);
        let both = [ga, gb].map(|g| logged(m, g, SHAY_QUEST));
        (both == [Some((true, false)); 2]).then_some(())
    })
    .await;
    assert_eq!(said(&mut a, 3917).await, shay);
    assert_eq!(said(&mut a, 3922).await, rogi);
    // She goes after her 30 s.
    let gone = |m: &mut Map| m.escorts.is_empty() && the(m, SHAY).is_none();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(map.query(gone).await, Some(false), "not yet");
    hurry(&map, Duration::from_secs(30)).await;
    until(&map, "gone", move |m| gone(m).then_some(())).await;
}

const KANATI: u32 = 3568;
const KANATI_QUEST: u32 = 938;

/// Thistle Tea's "fails the quest and respawns the follower if it dies or
/// is left behind": Mist (938) killed, the quest fails and she is back at
/// her place at once; taken again and left a hundred and fifty yards
/// behind, it fails again.
#[tokio::test]
async fn a_follower_dead_or_left_behind_fails_its_quest() {
    let realm = realm_of(follower_world(KANATI, KANATI_QUEST, Some(3519), 80.0));
    let (mut a, g, map) = player(&realm, "Alice").await;
    let first = guid::unit(KANATI, KANATI);
    a.sees(first).await;
    let back = |map: MapHandle| async move {
        until(&map, "back at her place", |m| {
            if !m.escorts.is_empty() {
                return None;
            }
            let c = m.creature(the(m, KANATI)?)?;
            (c.position.x == START.0 - 1.5).then_some(c.guid)
        })
        .await
    };
    for left_behind in [false, true] {
        let mist = map.query(|m| the(m, KANATI)).await.flatten().unwrap();
        map.query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            if let Some(slot) = p.journal.slot_of(KANATI_QUEST) {
                p.journal.clear(slot);
            }
            place(m, g, at(START.0, START.1, START.2));
        })
        .await;
        take(&map, g, mist, KANATI_QUEST).await;
        assert_eq!(map.query(|m| m.escorts.len()).await, Some(1));
        map.query(move |m| {
            if left_behind {
                place(m, g, at(START.0 + 150.0, START.1, START.2));
            } else {
                crate::game::map::combat::deal_damage(m, g, mist, 1_000_000);
            }
        })
        .await;
        a.wait_for("SMSG_QUESTUPDATE_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_FAILED(f) if f.quest_id == KANATI_QUEST => {
                Some(())
            }
            _ => None,
        })
        .await;
        back(map.clone()).await;
    }
}

const GORM: u32 = 6669;
const GORM_QUEST: u32 = 2078;
const GOAL: u32 = 6667;

/// Thistle Tea's "can turn on the player at its goal instead of crediting
/// them": 2078's follower, beside its goal, credits no one, takes a
/// hostile faction and attacks the player, as the goal says its line.
#[tokio::test]
async fn a_follower_turns_on_the_player_at_its_goal() {
    let realm = realm_of(follower_world(GORM, GORM_QUEST, Some(GOAL), 30.0));
    let (mut a, g, map) = player(&realm, "Alice").await;
    let gorm = guid::unit(GORM, GORM);
    a.sees(gorm).await;
    take(&map, g, gorm, GORM_QUEST).await;
    // Its key turned (its gossip option, not the quest's acceptance).
    map.query(move |m| crate::game::map::escorts::gossip_select(m, g, gorm, 1))
        .await
        .unwrap();
    let by_goal = at(START.0 + 25.0, START.1, START.2);
    let turned = until(&map, "turned", move |m| {
        place(m, g, by_goal);
        let c = m.creature(gorm)?;
        (c.combat.attacking && c.combat.victim == g).then(|| {
            (
                unit_u32(m, gorm, UNIT_FIELD_FACTIONTEMPLATE),
                logged(m, g, GORM_QUEST),
            )
        })
    })
    .await;
    assert_eq!(turned, (MONSTERS, Some((false, false))), "no credit");
    assert_eq!(said(&mut a, 2704).await, guid::unit(GOAL, GOAL));
}

const KERNOBEE: u32 = 7850;
const KERNOBEE_QUEST: u32 = 2904;

/// Thistle Tea's "can head for a place instead of a creature": Kernobee
/// (2904) credits the player within ten yards of the place he heads for.
#[tokio::test]
async fn a_follower_heads_for_a_place() {
    // His goal, as the catalog has it, and his spawn thirty yards off.
    let goal = at(-330.92, -3.03, -152.85);
    let mut d = world();
    questgiver(
        &mut d,
        0,
        KERNOBEE,
        KERNOBEE_QUEST,
        (goal.x + 30.0, goal.y, goal.z),
    );
    let realm = realm_of(d);
    let (mut a, g, map) = player(&realm, "Alice").await;
    let kernobee = guid::unit(KERNOBEE, KERNOBEE);
    // Beside him, at once.
    let by_him = at(goal.x + 29.0, goal.y, goal.z);
    map.query(move |m| m.teleport(g, m.key, by_him, 0.0)).await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    until(&map, "there", move |m| {
        m.players[&g].teleport.is_none().then_some(())
    })
    .await;
    a.sees(kernobee).await;
    take(&map, g, kernobee, KERNOBEE_QUEST).await;
    assert_eq!(map.query(|m| m.escorts.len()).await, Some(1));
    // Halfway: nothing; at the place, credited.
    let halfway = at(goal.x + 16.0, goal.y, goal.z);
    until(&map, "halfway", move |m| {
        place(m, g, halfway);
        (dist(m.creature(kernobee)?.position, halfway) <= 3.5).then_some(())
    })
    .await;
    assert_eq!(
        map.query(move |m| logged(m, g, KERNOBEE_QUEST))
            .await
            .flatten(),
        Some((false, false))
    );
    until(&map, "credited", move |m| {
        place(m, g, at(goal.x + 2.0, goal.y, goal.z));
        (logged(m, g, KERNOBEE_QUEST) == Some((true, false))).then_some(())
    })
    .await;
}

// Quest dependencies and sharing.

/// Thistle Tea's "invalid breadcrumb paths fail closed and ordinary
/// missing links are ignored": breadcrumbs round in a circle, or to a
/// missing quest, are invalid and lead nowhere; a quest whose previous and
/// next quests are missing depends on nothing.
#[test]
fn bad_breadcrumbs_fail_closed_and_missing_links_are_ignored() {
    let quests: ahash::AHashMap<u32, Quest> = [
        Quest {
            id: 1,
            breadcrumb_for: 2,
            ..Default::default()
        },
        Quest {
            id: 2,
            breadcrumb_for: 1,
            ..Default::default()
        },
        Quest {
            id: 3,
            breadcrumb_for: 4,
            ..Default::default()
        },
        Quest {
            id: 5,
            breadcrumb_for: 3,
            ..Default::default()
        },
        Quest {
            id: 6,
            prev_quest: 99,
            next_in_chain: 100,
            ..Default::default()
        },
    ]
    .into_iter()
    .map(|q| (q.id, q))
    .collect();
    let g = crate::game::data::npc::QuestGraph::compile(&quests);
    for id in [1, 2, 3, 5] {
        assert!(g.invalid.contains(&id), "{id} invalid");
        assert!(
            !g.breadcrumb_targets.contains_key(&id),
            "{id} leads nowhere"
        );
    }
    assert!(!g.invalid.contains(&6));
    assert!(!g.prerequisites.contains_key(&6), "no prerequisites");
    assert!(g.breadcrumbs_to.is_empty());
}

const GIVER: u32 = 3100;
const EVENT: u32 = 12;

fn giver_world() -> GameData {
    let mut d = world();
    let mut t = template(GIVER, "Giver");
    t.npc_flags = 0x2;
    d.creature_templates.insert(GIVER, t);
    spawn(&mut d, 0, GIVER, (START.0 + 2.0, START.1, START.2));
    let quest = |id: u32| Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        ..Default::default()
    };
    for q in [
        // A breadcrumb to an event's quest.
        Quest {
            breadcrumb_for: 2,
            ..quest(1)
        },
        quest(2),
        // To share: one that repeats, one after 13, one timed; and 13.
        Quest {
            flags: 0x8,
            special_flags: 1,
            ..quest(11)
        },
        Quest {
            flags: 0x8,
            prev_quest: 13,
            ..quest(12)
        },
        quest(13),
        Quest {
            flags: 0x8,
            limit_time: 60,
            ..quest(14)
        },
        Quest {
            limit_time: 60,
            ..quest(15)
        },
    ] {
        d.npc.quests.insert(q.id, q);
    }
    d.world.event_quests.insert(2, EVENT);
    d.npc.creature_starts.insert(GIVER, vec![1, 2]);
    d
}

/// Thistle Tea's "breadcrumb targets retain their event requirement after
/// compilation": a breadcrumb to an event's quest isn't offered, nor taken,
/// while the event is off; with it on, it is.
#[tokio::test]
async fn a_breadcrumb_to_an_events_quest_waits_on_the_event() {
    let realm = realm_of(giver_world());
    let (mut a, g, map) = player(&realm, "Alice").await;
    let giver = guid::unit(GIVER, GIVER);
    a.sees(giver).await;
    let offered = move |m: &mut Map| -> Vec<u32> {
        crate::game::map::quests::giver_quests(m, g, giver)
            .iter()
            .map(|q| q.quest_id)
            .collect()
    };
    assert_eq!(map.query(offered).await.unwrap(), Vec::<u32>::new(), "off");
    let taken = map
        .query(move |m| {
            crate::game::map::quests::accept(m, g, giver, 1);
            m.players[&g].journal.has(1)
        })
        .await;
    assert_eq!(taken, Some(false), "not taken");
    let on = map
        .query(move |m| {
            m.realm.social.events.lock().insert(EVENT);
            offered(m)
        })
        .await
        .unwrap();
    assert_eq!(on, vec![1, 2], "on");
    let taken = map
        .query(move |m| {
            crate::game::map::quests::accept(m, g, giver, 1);
            m.players[&g].journal.has(1)
        })
        .await;
    assert_eq!(taken, Some(true));
}

/// The results the sharer hears on sharing `quest`.
async fn share(a: &mut Client, giver: Guid, quest_id: u32) -> Vec<QuestPartyMessage> {
    a.clear();
    a.send(CMSG_PUSHQUESTTOPARTY { quest_id }).await;
    // Its packets handled: a status query answered.
    a.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver }).await;
    a.wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(_) => Some(()),
        _ => None,
    })
    .await;
    let mut out = Vec::new();
    for m in std::mem::take(&mut a.backlog) {
        if let ServerOpcodeMessage::MSG_QUEST_PUSH_RESULT(r) = m {
            out.push(r.message);
        }
    }
    out
}

/// Thistle Tea's "allows rewarded repeatable quests but enforces
/// prerequisites and existing timers": a repeatable quest the member was
/// rewarded is offered them; one whose previous quest they haven't done,
/// or a timed one while they have a timer running, they can't take.
#[tokio::test]
async fn a_repeatable_quest_is_shared_but_not_past_its_prerequisite_or_a_timer() {
    use QuestPartyMessage as R;
    let realm = realm_of(giver_world());
    let (mut a, ga, map) = player(&realm, "Alice").await;
    let (_b, gb, _) = player(&realm, "Bob").await;
    let giver = guid::unit(GIVER, GIVER);
    a.sees(giver).await;
    let online = |g: Guid| realm.registry.online(g).unwrap();
    assert!(crate::game::social::groups::join_directly(
        &realm,
        &online(ga),
        &online(gb)
    ));
    map.query(move |m| {
        let expires = crate::game::social::mail::now() + 300;
        let p = m.players.get_mut(&ga).unwrap();
        for (quest, expires) in [(11, 0), (12, 0), (14, expires)] {
            p.journal.add(crate::game::store::Logged {
                quest,
                expires,
                ..Default::default()
            });
        }
        let p = m.players.get_mut(&gb).unwrap();
        p.journal.rewarded.insert(11);
    })
    .await;
    assert_eq!(
        share(&mut a, giver, 11).await,
        vec![R::SharingQuest],
        "offered"
    );
    assert_eq!(
        share(&mut a, giver, 12).await,
        vec![R::SharingQuest, R::CantTakeQuest],
        "13 not done"
    );
    map.query(move |m| {
        m.players.get_mut(&gb).unwrap().misc.quest_shared = None;
    })
    .await;
    assert_eq!(
        share(&mut a, giver, 14).await,
        vec![R::SharingQuest],
        "no timer of his"
    );
    map.query(move |m| {
        let expires = crate::game::social::mail::now() + 300;
        let p = m.players.get_mut(&gb).unwrap();
        p.misc.quest_shared = None;
        p.journal.add(crate::game::store::Logged {
            quest: 15,
            expires,
            ..Default::default()
        });
    })
    .await;
    assert_eq!(
        share(&mut a, giver, 14).await,
        vec![R::SharingQuest, R::CantTakeQuest],
        "his timer running"
    );
}

// Alterac Valley's captured animals.

const FROSTWOLF: u32 = 10981;
const WOLF_AURA: u32 = 21827;

/// A Frostwolf beside `START`, and its aura.
fn stables() -> GameData {
    let mut d = world();
    d.creature_templates
        .insert(FROSTWOLF, template(FROSTWOLF, "Frostwolf"));
    spawn(&mut d, 0, FROSTWOLF, (START.0 + 3.0, START.1, START.2));
    let mut aura = SpellTemplate {
        id: WOLF_AURA,
        name: "Frostwolf Aura DND".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    aura.effects[0] = SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 25,
        ..Default::default()
    };
    d.spells.spells.insert(WOLF_AURA, aura);
    d
}

fn wolf() -> Guid {
    guid::unit(FROSTWOLF, FROSTWOLF)
}

/// Alice with the Frostwolf captured: its aura on her, from it.
async fn captured() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = realm_of(stables());
    let (mut a, g, map) = player(&realm, "Alice").await;
    a.sees(wolf()).await;
    map.query(move |m| {
        let spell = m.realm.data.spells.spells[&WOLF_AURA].clone();
        let _ = crate::game::map::auras::apply_spell(
            m,
            wolf(),
            g,
            &spell,
            &[(0, 0)],
            crate::game::map::auras::Context::default(),
        );
    })
    .await;
    (realm, a, g, map)
}

/// When the capture's next look is due, from now.
fn next_look(m: &Map, g: Guid) -> Option<Duration> {
    let h = crate::game::map::auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == WOLF_AURA)?;
    h.effects[0]
        .next_tick
        .map(|t| t.saturating_duration_since(m.now))
}

/// The capture's look due now.
fn look_now(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(a) = crate::game::map::auras::auras_mut(m, g)
        && let Some(h) = a.holders.iter_mut().find(|h| h.spell == WOLF_AURA)
    {
        h.effects[0].next_tick = Some(now);
    }
}

/// When the wolf goes, from now, if it is to.
fn wolf_goes_in(m: &Map) -> Option<Duration> {
    let (at, _) = crate::game::map::event_ai::scripted(m, wolf())?.script_despawn?;
    Some(at.saturating_duration_since(m.now))
}

/// Thistle Tea's "capture auras schedule two-second checks and keep the
/// latest live creature observation": the aura looks every two seconds,
/// and while the wolf is alive and near, sets it following the player
/// again, the aura kept.
#[tokio::test]
async fn a_captured_animal_is_looked_at_every_two_seconds_and_follows() {
    let (_realm, _a, g, map) = captured().await;
    let first = map.query(move |m| next_look(m, g)).await.flatten();
    assert!(
        first.is_some_and(|t| t > Duration::from_millis(1500) && t <= CHECK),
        "{first:?}"
    );
    let follow = |m: &Map| {
        crate::game::map::event_ai::scripted(m, wolf())
            .and_then(|s| s.follow)
            .map(|(whom, yards, _)| (whom, yards))
    };
    assert_eq!(map.query(move |m| follow(m)).await.flatten(), None);
    map.query(move |m| look_now(m, g)).await;
    let after = until(&map, "following", move |m| {
        let f = follow(m)?;
        Some((f, next_look(m, g), wolf_goes_in(m)))
    })
    .await;
    assert_eq!(after.0, (g, 3.0), "three yards behind her");
    assert!(
        after.1.is_some_and(|t| t > Duration::from_millis(1500)),
        "the next in two seconds: {after:?}"
    );
    assert_eq!(after.2, None, "kept");
}

const CHECK: Duration = Duration::from_secs(2);

/// Thistle Tea's "dead, missing, distant and cross-world captures are
/// removed": the wolf dead, gone, more than fifty yards off or the player
/// dead, the next look ends the aura and sends the wolf off in a second
/// (or, the player dead and her auras gone with her life, two).
#[tokio::test]
async fn a_lost_capture_ends_and_its_animal_goes() {
    for case in ["dead", "gone", "far", "player dead"] {
        let (_realm, _a, g, map) = captured().await;
        map.query(move |m| {
            match case {
                "dead" => crate::game::map::combat::deal_damage(m, g, wolf(), 1_000_000),
                "gone" => {
                    m.remove_creature(wolf());
                }
                "far" => place(m, g, at(START.0 + 3.0, START.1 + 50.5, START.2)),
                _ => crate::game::map::combat::deal_damage(m, wolf(), g, 1_000_000),
            }
            look_now(m, g);
        })
        .await;
        let goes = until(&map, "the aura gone", move |m| {
            let gone =
                !crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(WOLF_AURA));
            gone.then(|| wolf_goes_in(m))
        })
        .await;
        match case {
            "dead" | "gone" => {}
            "far" => assert!(
                goes.is_some_and(|t| t <= Duration::from_millis(1000)),
                "{case}: {goes:?}"
            ),
            _ => assert!(
                goes.is_some_and(|t| t <= Duration::from_millis(2000)),
                "{case}: {goes:?}"
            ),
        }
    }
    // Fifty yards is near enough.
    let (_realm, _a, g, map) = captured().await;
    map.query(move |m| {
        place(m, g, at(START.0 + 3.0, START.1 + 49.5, START.2));
        look_now(m, g);
    })
    .await;
    let kept = until(&map, "looked at", move |m| {
        next_look(m, g)
            .filter(|t| *t > Duration::from_millis(1000))
            .map(|_| wolf_goes_in(m))
    })
    .await;
    assert_eq!(kept, None);
}

/// Thistle Tea's "delivery, cancellation and logout remove the aura and
/// dismiss its creature after two seconds": the Stable Master's script
/// takes the aura and the wolf goes two seconds after; leaving the map
/// does the same.
#[tokio::test]
async fn a_delivered_capture_sends_its_animal_off() {
    let (_realm, _a, g, map) = captured().await;
    let goes = map
        .query(move |m| {
            crate::game::map::auras::remove_spell(m, g, WOLF_AURA, None);
            wolf_goes_in(m)
        })
        .await
        .flatten();
    assert!(
        goes.is_some_and(|t| t > Duration::from_millis(1500) && t <= CHECK),
        "{goes:?}"
    );
    // Leaving the map.
    let (realm, mut a, g, map) = captured().await;
    map.call(move |m| m.teleport(g, MapKey::world(1), at(0.0, 0.0, 0.0), 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    let goes = until(&map, "Alice gone", move |m| {
        (!m.players.contains_key(&g)).then(|| wolf_goes_in(m))
    })
    .await;
    assert!(
        goes.is_some_and(|t| t > Duration::from_millis(1000) && t <= CHECK),
        "{goes:?}"
    );
    let _ = realm;
}
