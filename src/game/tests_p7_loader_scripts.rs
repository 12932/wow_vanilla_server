//! Phase 7's loader round, the scripts (Thistle Tea's `world/loader/` tests
//! of its creature scripts' routes and texts, escorts, group, guardian and
//! waypoint scripts, conditions and gossip): what wvs reads of them, and
//! what it does with them.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::data::scripts::{InlineText, ScriptStep, StepExt};
use crate::game::map::escorts::{self, Act};
use crate::game::map::{Map, MapHandle, event_ai, scripts};

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn step(command: u32, datalong: [u32; 4], dataint: [i32; 4]) -> ScriptStep {
    ScriptStep {
        command,
        datalong,
        dataint,
        ..Default::default()
    }
}

/// The route a script set `g` walking: its points' numbers, how many have
/// a script, and whether it repeats.
fn route_of(m: &Map, g: Guid) -> Option<(Vec<u32>, usize, bool)> {
    event_ai::scripted(m, g)
        .and_then(|s| s.route.as_ref())
        .map(|r| {
            (
                r.points.iter().map(|p| p.point).collect(),
                r.points.iter().filter(|p| p.script_id != 0).count(),
                r.repeat,
            )
        })
}

/// Thistle Tea's creature-script routes (`creature_script_route_test.exs`)
/// and the waypoint origins (`waypoint_vmangos_test.exs`), on the real
/// world and the imported scripts: a route with a path of its own (10096's)
/// numbers it from 0, its point steps (0, 1, 4, 5) a movement script each;
/// one without (13179's) walks its `script_waypoint` rows, 0 to 75, with
/// its point 74's steps. `start_waypoints` from source 5 picks the route
/// by its variant (`dataint3`): Reginald Windsor's (12580) second; there is
/// no variant 0 of his. Source 3 walks a special path (7784's: 54 points,
/// three with scripts, repeating with `datalong4`); source 1 a spawn's
/// (11006's, 9 points), source 2 an entry's (1446's, 27).
#[tokio::test]
#[ignore = "extended"]
async fn real_routes_number_their_points_and_waypoints_start_from_their_origin() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let own = &d.scripts.routes[&(10_096, 0)];
    assert_eq!(
        own.iter().map(|p| p.point).collect::<Vec<_>>(),
        (0..6).collect::<Vec<_>>()
    );
    assert_eq!(own[0].wait_ms, 1_800_000);
    let scripted: Vec<u32> = own
        .iter()
        .filter(|p| p.script_id != 0)
        .map(|p| p.point)
        .collect();
    assert_eq!(scripted, vec![0, 1, 4, 5]);
    assert!(own.iter().all(|p| {
        p.script_id == 0
            || d.scripts
                .movement
                .get(&p.script_id)
                .is_some_and(|s| !s.is_empty())
    }));
    let rows = &d.scripts.routes[&(13_179, 0)];
    let table = &d.script_waypoints[&13_179];
    assert_eq!(
        rows.iter().map(|p| (p.point, p.x)).collect::<Vec<_>>(),
        table.iter().map(|p| (p.point, p.x)).collect::<Vec<_>>()
    );
    assert_eq!(rows.len(), 76);
    let at: Vec<u32> = rows
        .iter()
        .filter(|p| p.script_id != 0)
        .map(|p| p.point)
        .collect();
    assert_eq!(at, vec![74]);
    assert!(d.scripts.routes.contains_key(&(12_580, 2)));
    assert!(!d.scripts.routes.contains_key(&(12_580, 0)));
    assert_eq!(d.waypoints_special[&7784].len(), 54);
    assert_eq!(d.waypoints[&11_006].len(), 9);
    assert_eq!(d.waypoints_by_entry[&1446].len(), 27);

    let (_c, g, map) = player(&realm, "P7SROUTE", "Routa", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let windsor = spawn(m, g, 12_580, 60, 5.0);
        let second = scripts::execute(m, &step(60, [5, 0, 0, 0], [0, 0, 2, 0]), windsor, g);
        let second = (second, route_of(m, windsor));
        let other = spawn(m, g, 12_580, 60, 5.0);
        scripts::execute(m, &step(60, [5, 0, 0, 0], [0, 0, 0, 0]), other, g);
        let none = route_of(m, other).is_some();
        let walker = dummy(m, g, 60, 5.0);
        let special = scripts::execute(m, &step(60, [3, 0, 0, 1], [0, 7784, 0, 0]), walker, g);
        let special = (special, route_of(m, walker));
        let by_spawn = scripts::execute(m, &step(60, [1, 0, 0, 0], [11_006, 0, 0, 0]), walker, g);
        let by_spawn = (by_spawn, route_of(m, walker).map(|r| r.0.len()));
        let by_entry = scripts::execute(m, &step(60, [2, 0, 0, 0], [0, 1446, 0, 0]), walker, g);
        let by_entry = (by_entry, route_of(m, walker).map(|r| r.0.len()));
        (second, none, special, by_spawn, by_entry)
    })
    .await;
    let (second, none, special, by_spawn, by_entry) = out;
    let want: Vec<u32> = d.scripts.routes[&(12_580, 2)]
        .iter()
        .map(|p| p.point)
        .collect();
    assert!(second.0);
    assert_eq!(second.1.map(|r| r.0), Some(want), "the second variant");
    assert!(!none, "no variant 0");
    let (ok, special) = special;
    assert!(ok);
    let (points, with_scripts, repeat) = special.unwrap();
    assert_eq!((points.len(), with_scripts, repeat), (54, 3, true));
    assert_eq!(points[0], 1, "from its first point");
    assert_eq!(by_spawn, (true, Some(9)));
    assert_eq!(by_entry, (true, Some(27)));
}

/// The kind of a creature's line `c` hears, and its words.
async fn heard(c: &mut crate::game::harness::Client) -> (String, String) {
    c.wait_for("a creature's line", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => {
            let kind = format!("{:?}", s.chat_type);
            let kind = kind
                .split([' ', '{', '('])
                .next()
                .unwrap_or_default()
                .to_owned();
            kind.starts_with("Monster")
                .then(|| (kind, s.message.clone()))
        }
        _ => None,
    })
    .await
}

const COME_SEE_THE_NIGHTMARE: i32 = 11_271;

/// Thistle Tea's talk steps (`script_text_vmangos_test.exs`,
/// `script_test.exs`): a talk of a broadcast text says it as the text
/// has it (11271, a yell), or as the step's chat type says (4, a
/// whisper), its `$n` the player's name; a talk of no broadcast text
/// says its own words (an imported script's), as their chat type has it.
#[tokio::test]
#[ignore = "extended"]
async fn talk_steps_say_their_texts_as_their_chat_types_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, g, map) = player(&realm, "P7STALK", "Talka", Race::Human, Class::Warrior).await;
    let speaker = on(&map, move |m| dummy(m, g, 60, 3.0)).await;
    let line = |s: ScriptStep| {
        let map = map.clone();
        async move { on(&map, move |m| scripts::execute(m, &s, speaker, g)).await }
    };
    c.clear();
    assert!(line(step(0, [0; 4], [COME_SEE_THE_NIGHTMARE, 0, 0, 0])).await);
    let stored = heard(&mut c).await;
    assert_eq!(
        stored,
        (
            "MonsterYell".to_owned(),
            "Come, Talka. See what the Nightmare brings...".to_owned()
        )
    );
    c.clear();
    assert!(line(step(0, [4, 0, 0, 0], [COME_SEE_THE_NIGHTMARE, 0, 0, 0])).await);
    let whispered = heard(&mut c).await;
    assert_eq!(whispered.0, "MonsterWhisper");
    assert_eq!(whispered.1, stored.1);
    c.clear();
    let angry = ScriptStep {
        ext: Some(Box::new(StepExt {
            texts: vec![InlineText {
                text: "Something is very, very angry.".into(),
                chat_type: scripts::CHAT_TEXT_EMOTE,
                language: 0,
                emote: 0,
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(line(angry).await);
    let literal = heard(&mut c).await;
    assert_eq!(
        literal,
        (
            "MonsterEmote".to_owned(),
            "Something is very, very angry.".to_owned()
        )
    );
}

/// Every text an escort's acts say, through what they hold for later.
fn said(acts: &'static [Act], out: &mut Vec<u32>) {
    for a in acts {
        match a {
            Act::Say(t) | Act::SayBy(_, t) => out.push(*t),
            Act::After(_, more) | Act::Hold(more) => said(more, out),
            Act::Await(_, a, b) => {
                said(a, out);
                said(b, out);
            }
            _ => {}
        }
    }
}

/// Thistle Tea's escorts (`quest_escort_vmangos_test.exs`): every line an
/// escort or follower says, and its quest's start script's talks, is a
/// broadcast text of the real world. Miran (309) at point 19 cries for
/// help ("Help! I've only one hand...") and two Dark Iron raiders (2149)
/// come, whose own script shouts "Get him!" as they come on (mangos's
/// `npc_miran`, its `SAY_DARK_IRON_RAIDER`; Thistle Tea's first raider
/// says "Feel the power of the Dark Iron Dwarves!", 1936, instead); Ruul
/// Snowhoof's quest (6482) starts by opening his cage
/// (`quest_start_scripts`, open door 48166).
#[test]
#[ignore = "extended"]
fn real_escorts_say_broadcast_texts_and_keep_their_scripts() {
    let Some(d) = real() else {
        return;
    };
    let mut missing = Vec::new();
    for e in escorts::catalog::ESCORTS {
        let mut texts = Vec::new();
        said(e.accept, &mut texts);
        for (_, acts) in e.points {
            said(acts, &mut texts);
        }
        if let Some(steps) = d
            .scripts
            .quest_start_script
            .get(&e.quest)
            .and_then(|s| d.scripts.quest_start.get(s))
        {
            for s in steps.iter().filter(|s| s.command == 0) {
                texts.extend(s.dataint.iter().filter(|t| **t > 0).map(|t| *t as u32));
            }
        }
        missing.extend(
            texts
                .into_iter()
                .filter(|t| !d.npc.broadcast_texts.contains_key(t))
                .map(|t| (e.quest, t)),
        );
    }
    for f in escorts::catalog::FOLLOWERS {
        let mut texts = Vec::new();
        said(f.accept, &mut texts);
        said(f.arrive, &mut texts);
        missing.extend(
            texts
                .into_iter()
                .filter(|t| !d.npc.broadcast_texts.contains_key(t))
                .map(|t| (f.quest, t)),
        );
    }
    assert_eq!(missing, Vec::new(), "every escort's line is a text");

    let miran = escorts::escort(309).unwrap();
    let (_, at_19) = miran.points.iter().find(|(p, _)| *p == 19).unwrap();
    let Act::Say(cry) = at_19[0] else {
        panic!("{:?}", at_19[0]);
    };
    assert!(
        d.npc.broadcast_texts[&cry]
            .male
            .starts_with("Help! I've only one hand")
    );
    let raiders = at_19
        .iter()
        .filter(|a| matches!(a, Act::Summon { entry: 2149, .. }))
        .count();
    assert_eq!(raiders, 2);
    let yells = d.scripts.ai_events[&2149]
        .iter()
        .flat_map(|e| e.actions)
        .filter_map(|a| d.scripts.ai_scripts.get(&a))
        .flatten()
        .filter(|s| s.command == 0)
        .flat_map(|s| s.dataint)
        .any(|t| {
            d.npc
                .broadcast_texts
                .get(&(t.max(0) as u32))
                .is_some_and(|b| b.male == "Get him!")
        });
    assert!(yells, "the raiders shout as they come on");
    let cage = &d.scripts.quest_start[&d.scripts.quest_start_script[&6482]];
    assert_eq!((cage[0].command, cage[0].datalong[0]), (11, 48_166));
}

/// Thistle Tea's group and guardian scripts (`group_script_vmangos_test.exs`,
/// `guardian_script_vmangos_test.exs`): the Stratholme patrol's movement
/// script (52124) runs generic script 5, for sure, on each of its group,
/// which makes where each stands its home; Ilkrud's evade (366401) is
/// the step that takes his guardians away (`pets::remove_guardians`).
#[tokio::test]
#[ignore = "extended"]
async fn the_patrols_group_script_sets_homes_and_ilkruds_evade_is_read() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let patrol = d.scripts.movement[&52_124]
        .iter()
        .find(|s| s.command == 90)
        .cloned()
        .unwrap();
    assert_eq!((patrol.datalong[0], patrol.dataint[0]), (5, 100));
    let home = &d.scripts.generic[&5];
    assert_eq!(
        home.iter()
            .map(|s| (s.command, s.datalong[0]))
            .collect::<Vec<_>>(),
        vec![(34, 1)]
    );
    let evade = &d.scripts.ai_scripts[&366_401];
    assert_eq!(
        evade
            .iter()
            .map(|s| (s.command, s.datalong[0]))
            .collect::<Vec<_>>(),
        vec![(56, 0)]
    );

    let (_c, g, map) = player(&realm, "P7SGROUP", "Groupa", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let walker = dummy(m, g, 60, 8.0);
        let at = m.creature(walker).unwrap().position;
        if let Some(c) = m.creature_mut(walker) {
            c.home = Vector3d {
                x: at.x + 30.0,
                ..at
            };
            c.position = at;
        }
        let ran = scripts::execute(m, &patrol, walker, walker);
        scripts::update(m);
        let now = m.creature(walker).unwrap().home;
        (
            ran,
            (now.x - at.x).abs() < 0.01 && (now.y - at.y).abs() < 0.01,
        )
    })
    .await;
    assert_eq!(out, (true, true), "home where it stands");
}

/// Thistle Tea's war effort and fishing conditions
/// (`server_variable_vmangos_test.exs`): with no server variable set,
/// the war effort's first phase (1630, 30050 at 0) and the fishing
/// contest's open (7716, 30056 at 0) hold; with 30050 at 2 and 30056 at
/// 1, the first phase is over (1630 doesn't), the third (264) is on, and
/// the contest won (7715).
#[tokio::test]
#[ignore = "extended"]
async fn server_variables_drive_the_war_effort_and_fishing_conditions() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P7SVARS", "Varsa", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let met = |m: &Map, id: u32| crate::game::map::conditions::met(m, id, None, Some(g));
        scripts::set_server_variable(30_050, 0);
        scripts::set_server_variable(30_056, 0);
        let initial = (met(m, 1630), met(m, 7716), met(m, 264), met(m, 7715));
        scripts::set_server_variable(30_050, 2);
        scripts::set_server_variable(30_056, 1);
        let advanced = (met(m, 1630), met(m, 264), met(m, 7715), met(m, 7716));
        (initial, advanced)
    })
    .await;
    assert_eq!(out.0, (true, true, false, false), "at the start");
    assert_eq!(out.1, (false, true, true, false), "later");
}

/// Thistle Tea's scripted gossip (`gossip_test.exs`): an option with a
/// reply text leads to a menu of that text alone. Plucky Johnson's (6626,
/// an imported script) "Please tell me the Phrase.." leads to a menu of
/// text 738 and no options.
#[test]
#[ignore = "extended"]
fn an_imported_gossip_reply_is_a_menu_of_its_text() {
    let Some(d) = real() else {
        return;
    };
    let menu = &d.npc.gossip_menus[&d.creature_templates[&6626].gossip_menu_id];
    assert_eq!(menu.texts[0].0, 720);
    let phrase = menu
        .options
        .iter()
        .find(|o| o.text.contains("Phrase"))
        .unwrap();
    let reply = &d.npc.gossip_menus[&(phrase.action_menu as u32)];
    assert_eq!(reply.texts, vec![(738, 0)]);
    assert!(reply.options.is_empty());
}
