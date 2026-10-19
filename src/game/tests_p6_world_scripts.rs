//! Round 6's world tests, ported from Thistle Tea's
//! `test/game/world/system/scripted_event_test.exs` (docs/parity/matrix.csv):
//! map events' commands and targets, scripts for all of a kind near or
//! for a zone's players, server variables set by a script.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::{CF_REVERSE_RESULT, Condition, SF_ABORT_ON_FAILURE, ScriptStep};
use crate::game::fields::UNIT_NPC_EMOTESTATE;
use crate::game::map::map_events::{active, target_of, targets};
use crate::game::map::scripts::execute;
use crate::game::map::{Map, conditions};
use crate::game::tests_p3_combat::{BRUTE, HELPER, data, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};

/// The script every test here starts: its source takes emote state 10.
const MARK: u32 = 96_300;
/// Conditions: map event 7 under way; server variable 96_310 at 6; its
/// reverse.
const EVENT_ON: u32 = 96_301;
const AT_SIX: u32 = 96_302;
const NOT_SIX: u32 = 96_303;

fn world() -> GameData {
    let mut d = data();
    d.scripts.generic.insert(
        MARK,
        vec![step(2, [u32::from(UNIT_NPC_EMOTESTATE), 10, 0, 0])],
    );
    for (entry, kind, values, flags) in [
        (EVENT_ON, 36, [7, 0, 0, 0], 0),
        (AT_SIX, 11, [96_310, 6, 0, 0], 0),
        (NOT_SIX, 11, [96_310, 6, 0, 0], CF_REVERSE_RESULT),
    ] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind,
                values,
                flags,
            },
        );
    }
    d
}

/// A step whose failure ends its script, so that `execute` tells it.
fn strict(command: u32, datalong: [u32; 4]) -> ScriptStep {
    ScriptStep {
        data_flags: SF_ABORT_ON_FAILURE,
        ..step(command, datalong)
    }
}

fn marked(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.get_u32(UNIT_NPC_EMOTESTATE) == 10)
}

/// Thistle Tea's map events "report duplicate starts and missing event
/// mutations", "target results expose map event source, target, and
/// matching extra targets" and "conditional target removal requires a
/// resolved condition": an event's commands (62 to 66, 69) fail before it
/// starts (a script asking to abort on failure ends there), a second start
/// (61) of one under way is refused; its
/// source, target and an extra target by entry are what target types 22,
/// 23 and 24 name; removing targets by a condition (64, modes 1 and 2)
/// wants one; once it has ended, it may start again.
#[tokio::test]
async fn map_event_commands_need_their_event_and_name_its_targets() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        let host = spawn(m, ga, BRUTE, 20, 5.0);
        let extra = spawn(m, ga, HELPER, 20, 6.0);
        let missing: Vec<bool> = [62, 63, 64, 65, 66, 69]
            .into_iter()
            .map(|c| execute(m, &strict(c, [7, 1, 0, 0]), host, ga))
            .collect();
        let started = execute(m, &strict(61, [7, 600, 0, 0]), host, ga);
        let again = execute(m, &strict(61, [7, 600, 0, 0]), host, ga);
        let added = execute(m, &strict(63, [7, 0, 0, 0]), extra, ga);
        let named = (
            target_of(m, 22, 7, 0),
            target_of(m, 23, 7, 0),
            target_of(m, 24, 7, HELPER),
            target_of(m, 24, 7, BRUTE),
        );
        let without: Vec<bool> = [1, 2]
            .into_iter()
            .map(|mode| execute(m, &strict(64, [7, 0, mode, 0]), host, ga))
            .collect();
        let kept = targets(m, 7);
        let with = execute(m, &strict(64, [7, EVENT_ON, 2, 0]), host, ga);
        let left = targets(m, 7);
        let ended = execute(m, &strict(62, [7, 1, 0, 0]), host, ga);
        let after = (
            active(m, 7),
            execute(m, &strict(63, [7, 0, 0, 0]), extra, ga),
        );
        let restarted = execute(m, &strict(61, [7, 600, 0, 0]), host, ga);
        (
            missing, started, again, added, named, without, kept, with, left, ended, after,
            restarted, host, extra,
        )
    })
    .await;
    let (missing, started, again, added, named, without, kept, with, left, ended, after) =
        (r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10);
    let (restarted, host, extra) = (r.11, r.12, r.13);
    assert_eq!(missing, [false; 6], "nothing to change before it starts");
    assert!(started && !again, "one at a time");
    assert!(added);
    assert_eq!(named, (Some(host), Some(ga), Some(extra), None));
    assert_eq!(without, [false, false], "no condition, no removal");
    assert_eq!(kept, [extra]);
    assert!(with);
    assert_eq!(left, Vec::<Guid>::new(), "all that meet it removed");
    assert!(ended);
    assert_eq!(after, (false, false));
    assert!(restarted);
}

/// Thistle Tea's "start script for all filters nearby objects by type and
/// entry" and "start script on zone": command 68 starts a script for each
/// creature of its entry within its radius (a creature of another entry,
/// or one further, left out), command 92 for each player in its zone (one
/// elsewhere left out) and, asked (datalong3 1), their pets.
#[tokio::test]
async fn scripts_start_for_all_of_a_kind_near_and_for_a_zones_players() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        let host = spawn(m, ga, HELPER, 20, 1.0);
        // A brute near, a helper near, a brute far.
        spawn(m, ga, BRUTE, 20, 10.0);
        spawn(m, ga, HELPER, 20, 12.0);
        spawn(m, ga, BRUTE, 20, 40.0);
        execute(m, &step(68, [MARK, 2, BRUTE, 20]), host, Guid::zero());
        // Alice in zone 12 with a pet, Bob in 40.
        let pet = spawn(m, ga, HELPER, 20, 3.0);
        m.players.get_mut(&ga).unwrap().zone = 12;
        m.players.get_mut(&ga).unwrap().owned.pet = Some(pet);
        m.players.get_mut(&gb).unwrap().zone = 40;
        execute(m, &step(92, [MARK, 12, 1, 0]), host, Guid::zero());
    })
    .await;
    // The scripts run in the map's update.
    crate::game::harness::an_update(&map).await;
    let now = on(&map, move |m| {
        let near = m
            .creatures
            .iter()
            .map(|(_, c)| c)
            .filter(|c| c.entry == BRUTE)
            .map(|c| (c.position.x as i32, marked(m, c.guid)))
            .collect::<Vec<_>>();
        let helpers = m
            .creatures
            .iter()
            .filter(|(_, c)| c.entry == HELPER && marked(m, c.guid))
            .count();
        (near, helpers, marked(m, ga), marked(m, gb))
    })
    .await;
    let (brutes, helpers, alice, bob) = now;
    let mut brutes = brutes;
    brutes.sort();
    assert_eq!(
        brutes.iter().map(|(_, on)| *on).collect::<Vec<_>>(),
        [true, false],
        "the near one alone: {brutes:?}"
    );
    assert_eq!(helpers, 1, "of the helpers, the pet alone");
    assert!(alice && !bob, "the zone's player alone");
}

/// Thistle Tea's "global writes need no map event": a script sets a server
/// variable (54) with no event under way, and a condition on it (type 11)
/// reads it, in any copy; reversed, it reads the opposite.
#[tokio::test]
async fn a_server_variable_a_script_sets_is_read_by_conditions() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        let before = (
            conditions::met(m, AT_SIX, None, None),
            conditions::met(m, NOT_SIX, None, None),
        );
        let set = execute(m, &step(54, [96_310, 6, 0, 0]), ga, ga);
        let after = (
            conditions::met(m, AT_SIX, None, None),
            conditions::met(m, NOT_SIX, None, None),
        );
        (before, set, after)
    })
    .await;
    assert_eq!(r, ((false, true), true, (true, false)));
}
