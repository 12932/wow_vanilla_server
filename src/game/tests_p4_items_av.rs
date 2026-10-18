//! Alterac Valley's turn-in supplies, as Thistle Tea's air, armor, beacon,
//! cavalry, ground and offering tests have them, through the protocol: who
//! may give and take them, the six fleets and their beacons, the
//! commanders' deaths and respawns, and the texts the quartermaster and the
//! summoners show.
//!
//! mangos-classic's `BattleGroundAV::HandleQuestComplete` counts the same
//! quests per team (the scraps by 20, the offerings by 1 or 5) and leaves
//! the rest as "need to implement some events"; Thistle Tea's scripts are
//! the reference for what follows.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{banner, both_into, put, said};
use super::tests_pvp2::{arrive, choose, creature, line, npc, spawn, turn_in, valley};
use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_quests::{self, Extras, Fleet, FleetPhase, March};
use crate::game::map::battleground::av_scripts;
use crate::game::map::battleground::{Phase, Score};
use crate::game::map::{Map as World, MapHandle, MapKey};

const AV: u32 = 30;

/// The wing commanders: entry, side, supply quest, goal, reputation per
/// turn-in, beacon, the point of their road that is home.
const COMMANDERS: [(u32, usize, u32, u32, i32, u32, usize); 6] = [
    (13179, 1, 6825, 90, 1, 17324, 74),
    (13180, 1, 6826, 60, 2, 17325, 84),
    (13181, 1, 6827, 30, 5, 17323, 97),
    (13438, 0, 6942, 90, 1, 17506, 66),
    (13439, 0, 6941, 60, 2, 17507, 76),
    (13437, 0, 6943, 30, 5, 17505, 92),
];

const ICHMAN: u32 = 13437;
const MULVERICK: u32 = 13181;
const SMITHS: [u32; 2] = [13257, 13176];
const SUMMONERS: [u32; 2] = [13442, 13236];
const RAM_COMMANDER: u32 = 13577;
const QUARTERMASTERS: [u32; 2] = [12096, 12097];
const MARSHAL: u32 = 13446;
const ORDERS: u32 = 17353;

fn text(d: &mut GameData, id: u32, line: &str) {
    d.npc.broadcast_texts.insert(
        id,
        crate::game::data::npc::BroadcastText {
            male: line.into(),
            ..Default::default()
        },
    );
}

fn item(d: &mut GameData, entry: u32, name: &str) {
    d.items.insert(
        entry,
        ItemTemplate {
            entry,
            class: 12,
            name: name.into(),
            stackable: 1,
            ..Default::default()
        },
    );
}

/// The valley with both sides' smiths, summoners, wing commanders (and
/// their roads home), the Alliance's ram commander and quartermaster, the
/// Horde's quartermaster, the beacons and the orders, and each menu
/// option's line.
fn data() -> GameData {
    let mut d = valley();
    npc(&mut d, SMITHS[1], "Smith Regzar", 35);
    npc(&mut d, SUMMONERS[1], "Primalist Thurloga", 35);
    for (entry, name) in [
        (13438, "Wing Commander Slidore"),
        (13439, "Wing Commander Vipore"),
        (13179, "Wing Commander Guse"),
        (13180, "Wing Commander Jeztor"),
        (13181, "Wing Commander Mulverick"),
        (RAM_COMMANDER, "Stormpike Ram Rider Commander"),
        (13576, "Stormpike Ram Rider"),
        (QUARTERMASTERS[0], "Stormpike Quartermaster"),
        (QUARTERMASTERS[1], "Frostwolf Quartermaster"),
        (MARSHAL, "Field Marshal Teravaine"),
        (13161, "Aerie Gryphon"),
    ] {
        npc(&mut d, entry, name, 35);
    }
    spawn(&mut d, 170010, SMITHS[1], 1.5);
    spawn(&mut d, 170011, SUMMONERS[1], 2.5);
    for (i, entry) in [13438, 13439, 13179, 13180, 13181].into_iter().enumerate() {
        spawn(&mut d, 170012 + i as u32, entry, 3.5 + i as f32 * 0.5);
    }
    spawn(&mut d, 170020, RAM_COMMANDER, 4.0);
    spawn(&mut d, 170021, QUARTERMASTERS[0], 6.5);
    spawn(&mut d, 170022, QUARTERMASTERS[1], 7.0);
    for &(entry, _, _, _, _, beacon, home) in &COMMANDERS {
        d.bg_routes.insert(entry, line(home + 1, 0.05));
        item(&mut d, beacon, &format!("Beacon {beacon}"));
    }
    d.bg_routes.insert(RAM_COMMANDER, vec![(30.0, 0.0, 0.0)]);
    d.bg_routes.insert(MARSHAL, vec![(40.0, 0.0, 0.0)]);
    item(&mut d, ORDERS, "Stormpike Assault Orders");
    for (id, line) in [
        (9130, "armor status"),
        (8718, "upgrade to seasoned"),
        (8719, "upgrade to veteran"),
        (8723, "upgrade to champion"),
        (8903, "launch the cavalry"),
        (9128, "ground status"),
        (9050, "assault orders"),
    ] {
        text(&mut d, id, line);
    }
    for id in [8757, 8759, 8761, 8641, 8643, 8645] {
        text(&mut d, id, &format!("offering {id}"));
    }
    for id in [8667, 8669, 8671, 8796, 8799, 8793] {
        text(&mut d, id, &format!("beacon {id}"));
    }
    for id in [10341, 10343, 10346, 10351, 10353, 10349] {
        text(&mut d, id, &format!("launch {id}"));
    }
    banner(&mut d, 178724, 10);
    banner(&mut d, 179999, 10);
    d
}

fn x(m: &World) -> &Extras {
    &m.battleground.as_ref().unwrap().av.as_ref().unwrap().extras
}

fn xm(m: &mut World) -> &mut Extras {
    &mut m.battleground.as_mut().unwrap().av.as_mut().unwrap().extras
}

/// Each fleet's phase, supplies and whether it launched.
fn fleets(m: &World) -> Vec<(FleetPhase, u32, bool)> {
    x(m).air
        .iter()
        .map(|f| (f.phase, f.count, f.launched))
        .collect()
}

/// Every other stockpile of both sides: scraps and tier, offerings,
/// hides and mounts, mine supplies and the ground assault's phase.
fn stockpiles(m: &World) -> Vec<u32> {
    let x = x(m);
    (0..2)
        .flat_map(|s| {
            [
                x.armor[s].scraps,
                u32::from(x.armor[s].tier),
                x.offerings[s].count,
                u32::from(x.offerings[s].launched),
                x.cavalry[s].hides,
                x.cavalry[s].mounts,
                x.cavalry[s].phase as u32,
                x.ground[s].irondeep,
                x.ground[s].coldtooth,
                x.ground[s].phase as u32,
            ]
        })
        .collect()
}

fn standing(m: &World, g: Guid, faction: u32) -> i32 {
    crate::game::map::reputation::standing(&m.players[&g], &m.realm.data, faction)
}

async fn set_standing(map: &MapHandle, g: Guid, faction: u32, to: i32) {
    map.query(move |m| {
        let delta = to - standing(m, g, faction);
        crate::game::map::reputation::modify(m, g, faction, delta, false);
    })
    .await;
}

async fn set_phase(map: &MapHandle, phase: Phase) {
    map.query(move |m| m.battleground.as_mut().unwrap().phase = phase)
        .await;
}

fn countdown() -> Phase {
    Phase::Starting {
        at: Instant::now() + Duration::from_secs(3600),
    }
}

fn ended() -> Phase {
    Phase::Ended {
        winner: 1,
        at: Instant::now(),
    }
}

/// `g` is taken out of the match (as one invited who never came in, or
/// gone): their score, to put back.
async fn outside(map: &MapHandle, g: Guid) -> Score {
    map.query(move |m| m.battleground.as_mut().unwrap().scores.remove(&g))
        .await
        .flatten()
        .expect("in the match")
}

async fn inside(map: &MapHandle, g: Guid, score: Score) {
    map.query(move |m| {
        m.battleground.as_mut().unwrap().scores.insert(g, score);
    })
    .await;
}

/// Polls `what` up to two seconds.
async fn until(map: &MapHandle, what: impl Fn(&World) -> bool + Send + Clone + 'static) -> bool {
    for _ in 0..200 {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

async fn of(map: &MapHandle, entry: u32) -> Guid {
    map.query(move |m| creature(m, entry))
        .await
        .flatten()
        .expect("spawned")
}

/// Alice of the Alliance and Grom of the Horde in a running valley, by
/// its NPCs.
async fn setup() -> (Arc<Realm>, Client, Guid, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    put(&map, &mut a, 3.0, 0.0).await;
    put(&map, &mut b, 3.0, 0.0).await;
    (realm, a, alice, b, grom, map)
}

/// The menu `npc` opens with: its title and its options' lines. The
/// NPC's own (template) menu is `(68, [])`: no menu of the valley's.
async fn menu(c: &mut Client, npc: Guid) -> (u32, Vec<String>) {
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("the menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => Some((
            g.title_text_id,
            g.gossips.iter().map(|o| o.message.clone()).collect(),
        )),
        _ => None,
    })
    .await
}

const NONE: (u32, Vec<String>) = (68, Vec::new());

/// An option chosen that closes the menu.
async fn pick(c: &mut Client, npc: Guid, option: u32) {
    c.clear();
    choose(c, npc, option).await;
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
}

/// An option chosen that shows a page: its title.
async fn page(c: &mut Client, npc: Guid, option: u32) -> u32 {
    c.clear();
    choose(c, npc, option).await;
    c.wait_for("the page", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => {
            g.gossips.is_empty().then_some(g.title_text_id)
        }
        _ => None,
    })
    .await
}

/// A prisoner talked to: freed, they set out (the menu closes).
async fn free(c: &mut Client, npc: Guid) {
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
}

fn yell_of(m: &ServerOpcodeMessage, from: Guid) -> Option<String> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
            SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. } if *sender1 == from => {
                Some(c.message.clone())
            }
            _ => None,
        },
        _ => None,
    }
}

fn count_of(m: &World, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

fn index_of(entry: u32) -> usize {
    COMMANDERS.iter().position(|c| c.0 == entry).unwrap()
}

#[tokio::test]
async fn a_wing_commander_is_freed_only_by_their_own_side_in_a_running_match() {
    let (_realm, mut a, alice, mut b, _grom, map) = setup().await;
    let ichman = of(&map, ICHMAN).await;
    let mulverick = of(&map, MULVERICK).await;
    // Grom can't free the Alliance's Ichman, nor Alice the Horde's
    // Mulverick: each opens their own menu.
    assert_eq!(menu(&mut b, ichman).await, NONE);
    assert_eq!(menu(&mut a, mulverick).await, NONE);
    // One not in the match frees no one.
    let score = outside(&map, alice).await;
    assert_eq!(menu(&mut a, ichman).await, NONE);
    inside(&map, alice, score).await;
    // Nor before the battle, or after it.
    for phase in [countdown(), ended()] {
        set_phase(&map, phase).await;
        assert_eq!(menu(&mut a, ichman).await, NONE);
    }
    set_phase(&map, Phase::Playing).await;
    let all = map.query(|m| fleets(m)).await.unwrap();
    assert!(all.iter().all(|f| *f == (FleetPhase::Prisoner, 0, false)));
    // Alice frees him: he runs home.
    free(&mut a, ichman).await;
    let phase = map
        .query(|m| x(m).air[index_of(ICHMAN)].phase)
        .await
        .unwrap();
    assert_eq!(phase, FleetPhase::Returning);
}

#[tokio::test]
async fn each_of_the_six_fleets_counts_its_own_supplies_and_calls_for_its_beacon_once() {
    let (_realm, mut a, alice, mut b, grom, map) = setup().await;
    for (i, &(entry, side, quest, goal, reputation, _, _)) in COMMANDERS.iter().enumerate() {
        let (g, faction, who) = if side == 0 {
            (alice, 730, "Stormpike")
        } else {
            (grom, 729, "the Horde")
        };
        let commander = of(&map, entry).await;
        let (before, stock, rep) = map
            .query(move |m| {
                xm(m).air[i] = Fleet {
                    phase: FleetPhase::Ready,
                    count: goal - 1,
                    launched: false,
                };
                (fleets(m), stockpiles(m), standing(m, g, faction))
            })
            .await
            .unwrap();
        a.clear();
        b.clear();
        turn_in(&map, g, quest, 1).await;
        let (after, stock_after, rep_after) = map
            .query(move |m| (fleets(m), stockpiles(m), standing(m, g, faction)))
            .await
            .unwrap();
        assert_eq!(after[i].1, goal, "{entry}: the goal reached");
        assert_eq!(rep_after - rep, reputation, "{entry}: its reputation");
        for (j, (was, now)) in before.iter().zip(&after).enumerate() {
            if j != i {
                assert_eq!(was, now, "{entry}: another fleet untouched");
            }
        }
        assert_eq!(stock, stock_after, "{entry}: no other stockpile");
        let c = if side == 0 { &mut a } else { &mut b };
        let call = c.wait_for("the call", |m| yell_of(m, commander)).await;
        assert_eq!(
            call,
            format!("Soldiers of {who}, come to my aid! The beacon must be planted.")
        );
        // One more: counted, but no second call.
        c.clear();
        turn_in(&map, g, quest, 1).await;
        let (count, rep_again) = map
            .query(move |m| (x(m).air[i].count, standing(m, g, faction)))
            .await
            .unwrap();
        assert_eq!(count, goal + 1);
        assert_eq!(rep_again - rep_after, reputation);
        let again = c
            .gets(Duration::from_millis(60), |m| yell_of(m, commander))
            .await;
        assert_eq!(again, None, "{entry}: the goal is called once");
    }
}

#[tokio::test]
async fn a_fallen_wing_commander_is_a_prisoner_again_keeping_supplies_and_launches_no_second_attack()
 {
    let (_realm, mut a, alice, _b, grom, map) = setup().await;
    let i = index_of(ICHMAN);
    let ichman = of(&map, ICHMAN).await;
    free(&mut a, ichman).await;
    arrive(&map).await;
    turn_in(&map, alice, 6943, 1).await;
    let home = map.query(move |m| x(m).air[i]).await.unwrap();
    assert_eq!((home.phase, home.count), (FleetPhase::Ready, 1));
    // He falls: a prisoner again, his supplies kept; none count while he
    // is.
    map.query(move |m| crate::game::map::ai::die(m, ichman, grom))
        .await;
    turn_in(&map, alice, 6943, 1).await;
    let fallen = map.query(move |m| x(m).air[i]).await.unwrap();
    assert_eq!((fallen.phase, fallen.count), (FleetPhase::Prisoner, 1));
    // Back, he needs freeing again.
    map.query(move |m| crate::game::map::ai::respawn_now(m, ichman))
        .await;
    let ichman = of(&map, ICHMAN).await;
    put(&map, &mut a, 3.0, 0.0).await;
    free(&mut a, ichman).await;
    let phase = map.query(move |m| x(m).air[i].phase).await.unwrap();
    assert_eq!(phase, FleetPhase::Returning);
    arrive(&map).await;
    // His thirty, and the attack.
    turn_in(&map, alice, 6943, 29).await;
    let (_, options) = menu(&mut a, ichman).await;
    assert_eq!(options, ["beacon 8793", "launch 10349"]);
    pick(&mut a, ichman, 1).await;
    assert!(map.query(move |m| x(m).air[i].launched).await.unwrap());
    // Fallen, back and freed again: home with his thirty, but his rider
    // has flown; he sends no other.
    map.query(move |m| crate::game::map::ai::die(m, ichman, grom))
        .await;
    map.query(move |m| crate::game::map::ai::respawn_now(m, ichman))
        .await;
    let ichman = of(&map, ICHMAN).await;
    put(&map, &mut a, 3.0, 0.0).await;
    free(&mut a, ichman).await;
    arrive(&map).await;
    let back = map.query(move |m| x(m).air[i]).await.unwrap();
    assert_eq!(
        (back.phase, back.count, back.launched),
        (FleetPhase::Ready, 30, true)
    );
    assert_eq!(menu(&mut a, ichman).await, NONE);
}

#[tokio::test]
async fn each_fleet_gives_a_beacon_for_its_supplies_and_another_once_resupplied() {
    let (_realm, mut a, alice, mut b, grom, map) = setup().await;
    for (i, &(entry, side, _, goal, _, beacon, _)) in COMMANDERS.iter().enumerate() {
        let g = if side == 0 { alice } else { grom };
        let c = if side == 0 { &mut a } else { &mut b };
        let commander = of(&map, entry).await;
        let before = map
            .query(move |m| {
                xm(m).air[i] = Fleet {
                    phase: FleetPhase::Ready,
                    count: goal + 10,
                    launched: false,
                };
                fleets(m)
            })
            .await
            .unwrap();
        let (title, options) = menu(c, commander).await;
        assert_eq!(title, 68);
        assert_eq!(options.len(), 2, "{entry}: the beacon and the attack");
        assert!(options[0].starts_with("beacon "));
        pick(c, commander, 0).await;
        let (count, beacons) = map
            .query(move |m| (x(m).air[i].count, count_of(m, g, beacon)))
            .await
            .unwrap();
        assert_eq!((count, beacons), (0, 1), "{entry}: the supplies spent");
        // Spent, no other; asked again, nothing changes.
        assert_eq!(menu(c, commander).await, NONE);
        pick(c, commander, 0).await;
        let (count, beacons) = map
            .query(move |m| (x(m).air[i].count, count_of(m, g, beacon)))
            .await
            .unwrap();
        assert_eq!((count, beacons), (0, 1));
        // Resupplied: another.
        map.query(move |m| xm(m).air[i].count = goal).await;
        assert_eq!(menu(c, commander).await.1.len(), 2);
        pick(c, commander, 0).await;
        let (after, beacons) = map
            .query(move |m| (fleets(m), count_of(m, g, beacon)))
            .await
            .unwrap();
        assert_eq!(after[i], (FleetPhase::Ready, 0, false));
        assert_eq!(beacons, 2);
        for (j, (was, now)) in before.iter().zip(&after).enumerate() {
            if j != i {
                assert_eq!(was, now, "{entry}: another fleet untouched");
            }
        }
    }
}

#[tokio::test]
async fn a_beacon_asked_for_unfunded_by_the_enemy_or_out_of_standing_keeps_the_stockpile() {
    let (_realm, mut a, _alice, mut b, grom, map) = setup().await;
    let i = index_of(13179);
    let guse = of(&map, 13179).await;
    let funded = Fleet {
        phase: FleetPhase::Ready,
        count: 90,
        launched: false,
    };
    map.query(move |m| xm(m).air[i] = funded).await;
    let kept = move |m: &World| {
        let f = x(m).air[i];
        (f.phase, f.count, f.launched) == (FleetPhase::Ready, 90, false)
    };
    // The Alliance's Alice: not hers.
    assert_eq!(menu(&mut a, guse).await, NONE);
    // Grom, out of the match.
    let score = outside(&map, grom).await;
    assert_eq!(menu(&mut b, guse).await, NONE);
    inside(&map, grom, score).await;
    // Grom, below neutral.
    set_standing(&map, grom, 729, -1).await;
    assert_eq!(menu(&mut b, guse).await, NONE);
    set_standing(&map, grom, 729, 0).await;
    // Before the battle.
    set_phase(&map, countdown()).await;
    assert_eq!(menu(&mut b, guse).await, NONE);
    set_phase(&map, Phase::Playing).await;
    assert!(map.query(move |m| kept(m)).await.unwrap());
    // On his way home, a supply short, or his rider flown: none.
    for f in [
        Fleet {
            phase: FleetPhase::Returning,
            ..funded
        },
        Fleet {
            count: 89,
            ..funded
        },
        Fleet {
            launched: true,
            ..funded
        },
    ] {
        map.query(move |m| xm(m).air[i] = f).await;
        assert_eq!(menu(&mut b, guse).await, NONE);
        let now = map.query(move |m| x(m).air[i]).await.unwrap();
        assert_eq!(
            (now.phase, now.count, now.launched),
            (f.phase, f.count, f.launched)
        );
    }
    map.query(move |m| xm(m).air[i] = funded).await;
    // The menu open, Grom falls below neutral: the beacon, and the attack,
    // are asked of him again, and refused.
    assert_eq!(menu(&mut b, guse).await.1.len(), 2);
    set_standing(&map, grom, 729, -1).await;
    pick(&mut b, guse, 0).await;
    pick(&mut b, guse, 1).await;
    let beacons = map.query(move |m| count_of(m, grom, 17324)).await.unwrap();
    assert_eq!(beacons, 0);
    assert!(
        map.query(move |m| kept(m)).await.unwrap(),
        "the stockpile kept"
    );
}

#[tokio::test]
async fn turn_ins_count_only_for_their_own_side_in_the_running_match() {
    let (_realm, _a, alice, _b, grom, map) = setup().await;
    let alliance = [7223, 6781, 7386, 6881, 7026, 7027, 5892, 6982, 6846, 6942];
    let horde = [7224, 6741, 7385, 6801, 7002, 7001, 5893, 6985, 6901, 6825];
    let start = map.query(|m| stockpiles(m)).await.unwrap();
    let reps = move |m: &World| {
        [
            standing(m, alice, 730),
            standing(m, alice, 729),
            standing(m, grom, 730),
            standing(m, grom, 729),
        ]
    };
    let rep = map.query(move |m| reps(m)).await.unwrap();
    let check = |what: &'static str| {
        let map = map.clone();
        let start = start.clone();
        async move {
            let (now, rep_now) = map.query(move |m| (stockpiles(m), reps(m))).await.unwrap();
            assert_eq!(now, start, "{what}: nothing counted");
            assert_eq!(rep_now, rep, "{what}: no reputation");
        }
    };
    // Each side's quests turned in by the other side, and quests of no one.
    for q in horde.into_iter().chain([123, 0]) {
        turn_in(&map, alice, q, 1).await;
    }
    for q in alliance {
        turn_in(&map, grom, q, 1).await;
    }
    check("the other side").await;
    // One not in the match: one gone, or never there.
    let score = outside(&map, alice).await;
    for q in alliance {
        turn_in(&map, alice, q, 1).await;
        turn_in(&map, crate::game::guid::player(999_999), q, 1).await;
    }
    inside(&map, alice, score).await;
    check("one not in the match").await;
    // Before the battle, and after it.
    for phase in [countdown(), ended()] {
        set_phase(&map, phase).await;
        for q in alliance {
            turn_in(&map, alice, q, 1).await;
        }
        for q in horde {
            turn_in(&map, grom, q, 1).await;
        }
    }
    set_phase(&map, Phase::Playing).await;
    check("no battle on").await;
    // In the battle, on her side: counted.
    turn_in(&map, alice, 7223, 1).await;
    let scraps = map.query(|m| x(m).armor[0].scraps).await.unwrap();
    assert_eq!(scraps, 20);
}

#[tokio::test]
async fn the_smith_upgrades_one_tier_at_a_time_for_his_own_side_honored_and_funded() {
    let (_realm, mut a, alice, mut b, grom, map) = setup().await;
    let smith = of(&map, SMITHS[0]).await;
    let regzar = of(&map, SMITHS[1]).await;
    // Unfunded and honored: the status alone.
    set_standing(&map, alice, 730, 9000).await;
    assert_eq!(
        menu(&mut a, smith).await,
        (6073, vec!["armor status".to_owned()])
    );
    // Funded, below honored: the status alone.
    map.query(|m| xm(m).armor[0].scraps = 1520).await;
    set_standing(&map, alice, 730, 8999).await;
    assert_eq!(menu(&mut a, smith).await.1.len(), 1);
    // Honored: the upgrade; below honored when it is chosen, none.
    set_standing(&map, alice, 730, 9000).await;
    assert_eq!(menu(&mut a, smith).await.1.len(), 2);
    set_standing(&map, alice, 730, 8999).await;
    pick(&mut a, smith, 1).await;
    let tier = map.query(|m| x(m).armor[0].tier).await.unwrap();
    assert_eq!(tier, 0);
    set_standing(&map, alice, 730, 9000).await;
    // Each tier in turn, each upgrade once.
    for (tier, (option, rank)) in [
        ("upgrade to seasoned", "Seasoned"),
        ("upgrade to veteran", "Veteran"),
        ("upgrade to champion", "Champion"),
    ]
    .into_iter()
    .enumerate()
    {
        let (title, options) = menu(&mut a, smith).await;
        assert_eq!(title, 6219 + tier as u32);
        assert_eq!(options, ["armor status", option]);
        pick(&mut a, smith, 1).await;
        said(
            &mut a,
            "the upgrade",
            &format!("{rank} units are entering the battle!"),
        )
        .await;
        // The same option again, from the stale menu: not a tier further.
        pick(&mut a, smith, 1).await;
        let (armor_tier, scraps) = map
            .query(|m| (x(m).armor[0].tier, x(m).armor[0].scraps))
            .await
            .unwrap();
        assert_eq!((armor_tier, scraps), (tier as u8 + 1, 1520));
    }
    assert_eq!(menu(&mut a, smith).await, (6222, Vec::new()));
    // The Horde's smith, for the Horde.
    map.query(|m| xm(m).armor[1].scraps = 500).await;
    set_standing(&map, grom, 729, 9000).await;
    assert_eq!(menu(&mut b, regzar).await.1.len(), 2);
    pick(&mut b, regzar, 1).await;
    said(
        &mut b,
        "the upgrade",
        "Seasoned units are entering the battle!",
    )
    .await;
    // Not the other side's smith, nor once the battle is over.
    map.query(|m| xm(m).armor[1].scraps = 1000).await;
    assert_eq!(menu(&mut a, regzar).await, NONE);
    map.query(|m| xm(m).armor[0] = Default::default()).await;
    map.query(|m| xm(m).armor[0].scraps = 500).await;
    assert_eq!(menu(&mut b, smith).await, NONE);
    set_phase(&map, ended()).await;
    assert_eq!(menu(&mut a, smith).await, NONE);
    assert_eq!(menu(&mut b, regzar).await, NONE);
    let tiers = map
        .query(|m| (x(m).armor[0].tier, x(m).armor[1].tier))
        .await
        .unwrap();
    assert_eq!(tiers, (0, 1));
}

#[tokio::test]
async fn the_cavalry_keeps_its_donations_through_its_commanders_death_and_charges_again() {
    let (_realm, mut a, alice, _b, grom, map) = setup().await;
    set_standing(&map, alice, 730, 9000).await;
    let commander = of(&map, RAM_COMMANDER).await;
    turn_in(&map, alice, 7026, 25).await;
    turn_in(&map, alice, 7027, 25).await;
    assert_eq!(
        menu(&mut a, commander).await,
        (68, vec!["launch the cavalry".to_owned()])
    );
    pick(&mut a, commander, 0).await;
    let charged = map
        .query(|m| {
            let c = x(m).cavalry[0];
            (c.phase, c.hides, c.mounts)
        })
        .await
        .unwrap();
    assert_eq!(charged, (March::Marching, 0, 0));
    // Supplied again while he rides: no other charge.
    turn_in(&map, alice, 7026, 25).await;
    turn_in(&map, alice, 7027, 25).await;
    let near = map
        .query(move |m| m.creature(commander).map(|c| c.position.x))
        .await
        .flatten()
        .unwrap();
    put(&map, &mut a, near, 0.0).await;
    assert_eq!(menu(&mut a, commander).await, NONE);
    // He falls: the donations kept.
    map.query(move |m| crate::game::map::ai::die(m, commander, grom))
        .await;
    let dead = map
        .query(|m| {
            let c = x(m).cavalry[0];
            (c.phase, c.hides, c.mounts)
        })
        .await
        .unwrap();
    assert_eq!(dead, (March::Dead, 25, 25));
    // Back: they fund another charge.
    map.query(move |m| {
        crate::game::map::ai::respawn_now(m, commander);
        av_scripts::sweep_soon(m);
    })
    .await;
    assert!(until(&map, |m| x(m).cavalry[0].phase == March::Ready).await);
    let commander = of(&map, RAM_COMMANDER).await;
    let at = map
        .query(move |m| m.creature(commander).map(|c| c.position.x))
        .await
        .flatten()
        .unwrap();
    put(&map, &mut a, at, 0.0).await;
    assert_eq!(menu(&mut a, commander).await.1, ["launch the cavalry"]);
    pick(&mut a, commander, 0).await;
    let again = map
        .query(|m| {
            let c = x(m).cavalry[0];
            (c.phase, c.hides, c.mounts)
        })
        .await
        .unwrap();
    assert_eq!(again, (March::Marching, 0, 0));
}

#[tokio::test]
async fn the_quartermaster_says_where_supplies_stand_at_strict_totals_and_refuses_stale_orders() {
    let (_realm, mut a, alice, mut b, _grom, map) = setup().await;
    set_standing(&map, alice, 730, 9000).await;
    let quartermaster = of(&map, QUARTERMASTERS[0]).await;
    let horde_quartermaster = of(&map, QUARTERMASTERS[1]).await;
    for (iron, cold, text) in [
        (140, 0, 6733),
        (141, 0, 6732),
        (150, 0, 6732),
        (100, 50, 6732),
        (230, 0, 6732),
        (231, 0, 6731),
        (240, 0, 6731),
    ] {
        map.query(move |m| {
            xm(m).ground[0].irondeep = iron;
            xm(m).ground[0].coldtooth = cold;
        })
        .await;
        let (title, options) = menu(&mut a, quartermaster).await;
        assert_eq!(title, 6255);
        assert_eq!(options[0], "ground status");
        assert_eq!(
            page(&mut a, quartermaster, 0).await,
            text,
            "{iron} and {cold}"
        );
    }
    // Funded: who can't take the orders, and the supplies kept.
    map.query(|m| {
        xm(m).ground[0].irondeep = 280;
        xm(m).ground[0].coldtooth = 70;
    })
    .await;
    let supplies = move |m: &World| {
        let gr = x(m).ground[0];
        (
            gr.irondeep,
            gr.coldtooth,
            gr.phase,
            count_of(m, alice, ORDERS),
        )
    };
    let funded = (280, 70, March::Ready, 0);
    let goods = "Let me browse your goods.".to_owned();
    let status = "ground status".to_owned();
    set_standing(&map, alice, 730, 8999).await;
    assert_eq!(
        menu(&mut a, quartermaster).await,
        (6255, vec![status.clone(), goods.clone()])
    );
    set_standing(&map, alice, 730, 9000).await;
    assert_eq!(menu(&mut b, quartermaster).await, NONE, "the Horde");
    assert_eq!(menu(&mut a, horde_quartermaster).await, NONE, "the Horde's");
    let score = outside(&map, alice).await;
    assert_eq!(menu(&mut a, quartermaster).await, NONE, "not in the match");
    inside(&map, alice, score).await;
    map.query(|m| xm(m).ground[0].phase = March::Marching).await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 2, "marching");
    map.query(|m| {
        xm(m).ground[0].phase = March::Ready;
        xm(m).ground[0].quartermaster_dead = true;
    })
    .await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 2, "his death");
    map.query(|m| xm(m).ground[0].quartermaster_dead = false)
        .await;
    assert_eq!(map.query(move |m| supplies(m)).await.unwrap(), funded);
    // The orders offered, then asked for once Alice is below honored: none.
    assert_eq!(
        menu(&mut a, quartermaster).await.1,
        [status, "assault orders".to_owned(), goods]
    );
    set_standing(&map, alice, 730, 8999).await;
    pick(&mut a, quartermaster, 1).await;
    assert_eq!(map.query(move |m| supplies(m)).await.unwrap(), funded);
    assert_eq!(map.query(|m| creature(m, MARSHAL)).await.flatten(), None);
}

#[tokio::test]
async fn orders_outlast_the_quartermaster_new_ones_replace_the_lost_and_a_fallen_marshal_waits_for_a_new_quartermaster()
 {
    let (_realm, mut a, alice, _b, grom, map) = setup().await;
    set_standing(&map, alice, 730, 9000).await;
    let quartermaster = of(&map, QUARTERMASTERS[0]).await;
    let ground = |m: &World| {
        let gr = x(m).ground[0];
        (gr.phase, gr.irondeep, gr.coldtooth, gr.commander)
    };
    let marshals = |m: &World| {
        m.creatures
            .iter()
            .filter(|(_, c)| c.entry == MARSHAL)
            .count()
    };
    // Seven Coldtooth supplies: the orders, and the marshal assembles.
    turn_in(&map, alice, 6982, 7).await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 3);
    pick(&mut a, quartermaster, 1).await;
    let first = map.query(|m| creature(m, MARSHAL)).await.flatten().unwrap();
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Assembled, 0, 0, Some(first))
    );
    // Another marshal's death is nothing to this one's.
    map.query(|m| {
        av_quests::creature_died(m, crate::game::guid::unit(MARSHAL, 0x00AB_CDEF));
    })
    .await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap().0,
        March::Assembled
    );
    // The orders lost, more supplies: new orders, no second assembly.
    map.query(move |m| {
        let i = m.players[&alice]
            .items
            .iter()
            .position(|it| it.entry == ORDERS)
            .unwrap();
        crate::game::map::inventory::take_item(m, alice, i, 1);
    })
    .await;
    turn_in(&map, alice, 6982, 7).await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 3);
    pick(&mut a, quartermaster, 1).await;
    let (now, count, orders) = map
        .query(move |m| (ground(m), marshals(m), count_of(m, alice, ORDERS)))
        .await
        .unwrap();
    assert_eq!(now, (March::Assembled, 0, 0, Some(first)));
    assert_eq!((count, orders), (1, 1));
    // Turned in: he marches, once.
    turn_in(&map, alice, 6846, 1).await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap().0,
        March::Marching
    );
    turn_in(&map, alice, 6846, 1).await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap().0,
        March::Marching
    );
    // He falls: the orders send no one; supplied again, the quartermaster
    // still gives none.
    map.query(move |m| crate::game::map::ai::die(m, first, grom))
        .await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Dead, 0, 0, None)
    );
    turn_in(&map, alice, 6846, 1).await;
    turn_in(&map, alice, 6982, 7).await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Dead, 0, 70, None)
    );
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 2);
    // The quartermaster falls and comes back: the supplies kept, orders
    // again, and a new marshal; the old one's death changes nothing.
    map.query(move |m| crate::game::map::ai::die(m, quartermaster, grom))
        .await;
    map.query(move |m| {
        crate::game::map::ai::respawn_now(m, quartermaster);
        av_scripts::sweep_soon(m);
    })
    .await;
    assert!(until(&map, |m| !x(m).ground[0].quartermaster_dead).await);
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Ready, 0, 70, None)
    );
    let quartermaster = of(&map, QUARTERMASTERS[0]).await;
    put(&map, &mut a, 3.0, 0.0).await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 3);
    pick(&mut a, quartermaster, 1).await;
    let second = map
        .query(move |m| x(m).ground[0].commander)
        .await
        .flatten()
        .unwrap();
    assert_ne!(second, first);
    map.query(move |m| av_quests::creature_died(m, first)).await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Assembled, 0, 0, Some(second))
    );
    // Now the quartermaster falls with the marshal assembled: the orders
    // still send him, once, and the supplies given since stay.
    map.query(move |m| crate::game::map::ai::die(m, quartermaster, grom))
        .await;
    turn_in(&map, alice, 5892, 2).await;
    turn_in(&map, alice, 6846, 1).await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Marching, 20, 0, Some(second))
    );
    turn_in(&map, alice, 6846, 1).await;
    assert_eq!(
        map.query(move |m| ground(m)).await.unwrap(),
        (March::Marching, 20, 0, Some(second))
    );
}

#[tokio::test]
async fn the_summoners_tell_their_own_side_how_the_offerings_stand_and_no_one_else() {
    let (_realm, mut a, alice, mut b, _grom, map) = setup().await;
    let summoners = [of(&map, SUMMONERS[0]).await, of(&map, SUMMONERS[1]).await];
    for (s, greeting, option, status) in [(0, 6174, 8757, 6175), (1, 6093, 8641, 6098)] {
        let c = if s == 0 { &mut a } else { &mut b };
        for (count, progress) in [(0, 0), (99, 0), (100, 1), (159, 1), (160, 2), (199, 2)] {
            map.query(move |m| xm(m).offerings[s].count = count).await;
            assert_eq!(
                menu(c, summoners[s]).await,
                (
                    greeting,
                    vec![format!("offering {}", option + progress * 2)]
                ),
                "{count}"
            );
            assert_eq!(page(c, summoners[s], 0).await, status + progress, "{count}");
        }
    }
    // Not the other side's summoner, nor one out of the match, nor before
    // the battle.
    map.query(|m| xm(m).offerings[1].count = 150).await;
    assert_eq!(menu(&mut a, summoners[1]).await, NONE);
    assert_eq!(menu(&mut b, summoners[0]).await, NONE);
    let score = outside(&map, alice).await;
    assert_eq!(menu(&mut a, summoners[0]).await, NONE);
    inside(&map, alice, score).await;
    set_phase(&map, countdown()).await;
    assert_eq!(menu(&mut a, summoners[0]).await, NONE);
    set_phase(&map, Phase::Playing).await;
    // Their lord called: the status, and nothing to choose.
    map.query(|m| {
        xm(m).offerings[0] = av_quests::Offering {
            count: 200,
            launched: true,
        };
    })
    .await;
    assert_eq!(menu(&mut a, summoners[0]).await, (6177, Vec::new()));
}

#[tokio::test]
async fn only_the_valleys_beacons_call_down_their_flyers() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    fn place(m: &mut World, entry: u32, x: f32) -> Guid {
        let t = m.realm.data.gameobject_templates[&entry].clone();
        let mut go = crate::game::map::gameobject::GameObject::from_spawn(
            &t,
            &GameObjectSpawn {
                guid: 0x80_0100 + entry,
                entry,
                map: AV,
                x,
                state: 1,
                ..Default::default()
            },
        );
        go.spawn_guid = None;
        m.add_gameobject(go)
    }
    // In the valley: a beacon and another object, planted and left their
    // minute.
    let (beacon, other, beacons) = map
        .query(|m| {
            let beacon = place(m, 178724, 1.0);
            let other = place(m, 179999, 2.0);
            m.now = Instant::now();
            xm(m).hurry(Instant::now());
            av_quests::update(m);
            let beacons = (
                av_quests::is_beacon(m, beacon),
                av_quests::is_beacon(m, other),
            );
            for b in xm(m).beacons.iter_mut() {
                b.ready_at = Instant::now();
            }
            m.now = Instant::now();
            xm(m).hurry(Instant::now());
            av_quests::update(m);
            (beacon, other, beacons)
        })
        .await
        .unwrap();
    assert_eq!(beacons, (true, false));
    let (left, gryphons) = map
        .query(move |m| {
            (
                (
                    m.gameobject(beacon).is_some(),
                    m.gameobject(other).is_some(),
                ),
                m.creatures.iter().filter(|(_, c)| c.entry == 13161).count(),
            )
        })
        .await
        .unwrap();
    assert_eq!(left, (false, true), "the beacon called; the other stays");
    assert_eq!(gryphons, 1);
    // Alterac's map outside a match of it: a beacon is an object like any.
    let open = realm.maps.get(&realm, MapKey::world(AV));
    let (is_beacon, there) = open
        .query(|m| {
            let beacon = place(m, 178724, 1.0);
            av_quests::update(m);
            (
                av_quests::is_beacon(m, beacon),
                m.gameobject(beacon).is_some(),
            )
        })
        .await
        .unwrap();
    assert_eq!((is_beacon, there), (false, true));
}
