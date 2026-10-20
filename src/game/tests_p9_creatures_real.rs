//! Round 9's creature tests on the real world, ported from Thistle Tea's
//! `test/game/core/ai/script_targets_vmangos_test.exs`,
//! `companion_scripts_test.exs` and `bt/mob_test.exs` (docs/parity/
//! matrix.csv): the New Year revelers' talk, Estelle's thieves' tools, the
//! Felhound Tracker's stay and follow, and a creature with no way home.
//! Each needs the vmangos database (and the last the navmesh), so is
//! extended.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::{
    SF_SWAP_FINAL_TARGETS, SF_SWAP_INITIAL_TARGETS, SF_TARGET_SELF, ScriptTable,
};
use crate::game::map::creature::Motion;
use crate::game::map::pets::{self, Kind};
use crate::game::map::{Map, ai, scripts};
use crate::game::tests_p3_combat::spawn;
use crate::game::tests_p7_scripts::{hero_as, nav_realm};

/// The Stormwind Reveler, his script's start and its talk.
const REVELER: u32 = 15_694;
const REVELERS: u32 = 1_569_406;
/// Estelle Gendry, her gossip's script, and the tools it conjures.
const ESTELLE: u32 = 6_566;
const ESTELLE_GOSSIP: u32 = 161;
const TOOLS_CONJURE: u32 = 9_949;
const THIEVES_TOOLS: u32 = 5_060;
/// The Felhound Tracker, its roar's stay and its delayed follow.
const TRACKER: u32 = 8_668;
const TRACKER_STAY: u32 = 866_803;
const TRACKER_FOLLOW: u32 = 866_802;
/// A Kobold Vermin.
const KOBOLD: u32 = 6;

fn facing(m: &Map, from: Guid, to: Guid) -> bool {
    let (a, b) = (m.position_of(from).unwrap(), m.position_of(to).unwrap());
    let o = m.creature(from).unwrap().orientation;
    ((b.y - a.y).atan2(b.x - a.x) - o).abs() < 1e-3
}

/// Thistle Tea's loaded source swaps "New Year revelers turn both
/// participants and delay the swapped talk emote", "Estelle's gossip
/// makes the player cast the tools conjure on themselves", and the
/// companion's "loads tracker stay and delayed follow commands": a
/// reveler's start script picks another reveler near and hands him the
/// talk (a final swap), the two turn to each other, and the first talks a
/// second later; Estelle's cast step is the player's on themselves, and
/// conjures their thieves' tools; the tracker's roar script makes it stay,
/// its generic script follow.
#[tokio::test]
#[ignore = "extended"]
async fn the_revelers_talk_estelle_conjures_and_the_tracker_stays_and_follows() {
    let Some(realm) = nav_realm() else {
        eprintln!("skipped: no real world");
        return;
    };
    let d = &realm.data;
    let parent: Vec<_> = d.scripts.ai_scripts[&REVELERS]
        .iter()
        .map(|s| {
            (
                s.command,
                s.target_type,
                s.target_param1,
                s.target_param2,
                s.data_flags & SF_SWAP_FINAL_TARGETS != 0,
                s.datalong[0],
            )
        })
        .collect();
    assert_eq!(parent, [(39, 28, REVELER, 30, true, REVELERS)]);
    let sub: Vec<_> = d.scripts.generic[&REVELERS]
        .iter()
        .map(|s| {
            (
                s.command,
                s.delay_ms,
                s.data_flags & SF_SWAP_INITIAL_TARGETS != 0,
            )
        })
        .collect();
    assert_eq!(sub, [(35, 0, true), (35, 0, false), (1, 1000, true)]);
    let cast = d.scripts.gossip[&ESTELLE_GOSSIP]
        .iter()
        .find(|s| s.command == 15 && s.datalong[0] == TOOLS_CONJURE)
        .cloned()
        .expect("the conjure");
    assert_eq!(
        cast.data_flags & (SF_SWAP_INITIAL_TARGETS | SF_TARGET_SELF),
        SF_SWAP_INITIAL_TARGETS | SF_TARGET_SELF
    );
    let command_88 = |table: ScriptTable, id: u32| {
        realm.data.scripts.table(table)[&id]
            .iter()
            .find(|s| s.command == 88)
            .cloned()
            .expect("its command")
    };
    let stay = command_88(ScriptTable::CreatureAi, TRACKER_STAY);
    let follow = command_88(ScriptTable::Generic, TRACKER_FOLLOW);
    assert_eq!((stay.datalong[0], follow.datalong[0]), (0, 1));

    let (mut a, me, map) = hero_as(&realm, "Rogue", Class::Rogue).await;
    let (x, y, estelle) = map
        .query(move |m| {
            let x = spawn(m, me, REVELER, 60, 3.0);
            let y = spawn(m, me, REVELER, 60, 6.0);
            // Y aside, so the turns differ from their spawns'.
            let at = m.position_of(x).unwrap();
            m.creature_mut(y).unwrap().position = Vector3d {
                x: at.x + 3.0,
                y: at.y + 4.0,
                z: at.z,
            };
            (x, y, spawn(m, me, ESTELLE, 30, 2.0))
        })
        .await
        .unwrap();
    a.sees(x).await;
    a.sees(y).await;
    a.clear();
    let (turned, resolved, conjured, tracker) = map
        .query(move |m| {
            scripts::start(m, ScriptTable::CreatureAi, REVELERS, x, Guid::zero());
            let turned = (facing(m, x, y), facing(m, y, x));
            let resolved = scripts::resolve_targets(m, &cast, estelle, me);
            scripts::execute(m, &cast, estelle, me);
            let conjured = m.players[&me]
                .items
                .iter()
                .any(|i| i.entry == THIEVES_TOOLS);
            let at = m.players[&me].position();
            let t = pets::testing::spawn(m, me, TRACKER, Kind::Guardian, 40, at, 0.0);
            scripts::execute(m, &stay, t, Guid::zero());
            let stays = pets::state(m, t).unwrap().command;
            scripts::execute(m, &follow, t, Guid::zero());
            let follows = pets::state(m, t).unwrap().command;
            (turned, resolved, conjured, (stays, follows))
        })
        .await
        .unwrap();
    assert_eq!(turned, (true, true), "the two face each other");
    assert_eq!(resolved, Some((me, me)), "the player's cast on themselves");
    assert!(conjured, "her thieves' tools");
    assert_eq!(
        tracker,
        (pets::COMMAND_STAY, pets::COMMAND_FOLLOW),
        "the tracker stays, then follows"
    );
    let talker = a
        .wait_for("the talk", |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.emote == Emote::OneshotTalk => Some(e.guid),
            _ => None,
        })
        .await;
    assert_eq!(talker, x, "the one whose script it was, a second on");
}

/// Thistle Tea's scripted waypoint routes "a creature with no path home
/// takes the straight line there": a kobold 300 yards up, where the
/// navmesh has no way from, goes home in a straight line when it gives
/// up its fight.
#[tokio::test]
#[ignore = "extended"]
async fn a_creature_with_no_way_home_goes_straight_there() {
    let Some(realm) = nav_realm() else {
        eprintln!("skipped: no real world");
        return;
    };
    let (_a, me, map) = hero_as(&realm, "Warrior", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = spawn(m, me, KOBOLD, 5, 3.0);
            let home = m.creature(k).unwrap().home;
            let sky = Vector3d {
                z: home.z + 300.0,
                ..home
            };
            m.creature_mut(k).unwrap().position = sky;
            let no_way = m.nav.as_mut().unwrap().path(sky, home).is_none();
            ai::attacked(m, k, me);
            ai::enter_evade(m, k);
            let walk = match &m.creature(k).unwrap().motion {
                Motion::Moving(w) => Some(w.points.clone()),
                _ => None,
            };
            (no_way, walk, sky, home)
        })
        .await
        .unwrap();
    let (no_way, walk, sky, home) = out;
    assert!(no_way, "no way from the sky");
    assert_eq!(walk, Some(vec![sky, home]), "straight home");
}
