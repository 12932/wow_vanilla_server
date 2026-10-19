//! Round 6, against Thistle Tea's `area_trigger_script_test.exs`,
//! `game_object_script_test.exs`, `event_script_test.exs` and
//! `event_script/imp_delivery_test.exs` (docs/parity/matrix.csv): its
//! ported trigger, object and event scripts, as this server imports them
//! (`data/tt_scripts.rs`), run on the real world for those out in it, and
//! read as data for those in a dungeon (as Thistle Tea's own tests read
//! them).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::scripts::{Condition, ScriptStep, ScriptTable};
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::{Map, MapHandle, MapKey, scripts};
use crate::game::store::Logged;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map")
}

/// Where trigger `id` is.
fn trigger(d: &GameData, id: u32) -> Vector3d {
    let t = &d.area_triggers[&id];
    Vector3d {
        x: t.x,
        y: t.y,
        z: t.z,
    }
}

/// The living creatures of `entry` within `yards` of `at`.
fn living(m: &Map, entry: u32, at: Vector3d, yards: f32) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive() && dist(c.position, at) <= yards)
        .map(|(_, c)| c.guid)
        .collect()
}

/// `g` at `at`, as if walked there.
fn place(m: &mut Map, g: Guid, at: Vector3d) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = at;
}

/// `g` steps into trigger `id`, at its middle, its cooldown let go.
fn step_in(m: &mut Map, g: Guid, id: u32) {
    let at = trigger(&m.realm.data, id);
    place(m, g, at);
    m.trigger_cooldowns.clear();
    crate::game::map::world::area_trigger(m, g, id);
}

/// `quest` in `g`'s log, its objectives not met; or out of it.
fn take(m: &mut Map, g: Guid, quest: u32) {
    let j = &mut m.players.get_mut(&g).unwrap().journal;
    if j.slot_of(quest).is_none() {
        j.add(Logged {
            quest,
            ..Default::default()
        });
    }
}

fn drop_quest(m: &mut Map, g: Guid, quest: u32) {
    m.players
        .get_mut(&g)
        .unwrap()
        .journal
        .log
        .retain(|l| l.quest != quest);
}

fn explored(m: &Map, g: Guid, quest: u32) -> Option<bool> {
    let j = &m.players[&g].journal;
    j.slot_of(quest).map(|s| j.log[s].explored)
}

/// The scripts under way here `by` sooner, their delayed steps due.
fn skip(m: &mut Map, by: Duration) {
    for r in m.scripts.runs.iter_mut() {
        if let Some(t) = r.started.checked_sub(by) {
            r.started = t;
        }
    }
    scripts::update(m);
}

async fn hero(realm: &Arc<Realm>, account: &str, race: Race, class: Class) -> (Client, Guid) {
    let mut c = Client::connect(realm, account);
    let name = format!("{}{}", &account[..1], account[1..].to_lowercase());
    let g = c.enter(&name, race, class).await;
    (c, g)
}

async fn whispered(c: &mut Client) -> bool {
    c.gets(Duration::from_millis(500), |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(x)
            if matches!(
                x.chat_type,
                SMSG_MESSAGECHAT_ChatType::MonsterWhisper { .. }
            ) =>
        {
            Some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

/// Thistle Tea's Eastern Kingdoms triggers: Ravenholdt's courtyard credits
/// a rogue looking for the manor, no one else; the Twilight Grove raises a
/// Twilight Corrupter (only one) to whisper to a player on The Nightmare's
/// Corruption, the one about whispering the next time, and stays quiet
/// for others; Huldar and Miran's camp credits the delivery only (its
/// ambush can't spring, its first step having explored the quest the
/// second asks for incomplete), and does nothing for one without it.
#[tokio::test]
#[ignore = "extended"]
async fn eastern_triggers_answer_those_on_their_quests_and_no_one_else() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut r, rogue) = hero(&realm, "SIXHTRIGA", Race::Human, Class::Rogue).await;
    let (mut o, other) = hero(&realm, "SIXHTRIGB", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    // Ravenholdt.
    let raven = on(&map, move |m| {
        take(m, rogue, 6681);
        for g in [rogue, other] {
            step_in(m, g, 3066);
        }
        let kills = |m: &Map, g: Guid| {
            let j = &m.players[&g].journal;
            j.slot_of(6681).map(|s| j.log[s].kills[0])
        };
        (kills(m, rogue), kills(m, other))
    })
    .await;
    assert_eq!(raven, (Some(1), None), "the rogue on the manor's quest");
    // The Twilight Grove: none about, one raised; then the one about
    // whispers, and no other comes.
    let grove = on(&map, move |m| {
        let at = trigger(&m.realm.data, 4017);
        let before = living(m, 15_625, at, 400.0).len();
        step_in(m, other, 4017);
        let for_others = living(m, 15_625, at, 400.0).len();
        take(m, rogue, 8735);
        step_in(m, rogue, 4017);
        skip(m, Duration::from_secs(1));
        let raised = living(m, 15_625, at, 400.0).len();
        (before, for_others, raised)
    })
    .await;
    assert_eq!(grove, (0, 0, 1), "one corrupter, for the rogue alone");
    assert!(whispered(&mut r).await, "it whispers to her");
    let again = on(&map, move |m| {
        step_in(m, rogue, 4017);
        skip(m, Duration::from_secs(1));
        let at = trigger(&m.realm.data, 4017);
        living(m, 15_625, at, 400.0).len()
    })
    .await;
    assert_eq!(again, 1, "no second corrupter");
    assert!(whispered(&mut r).await, "the one about whispers again");
    assert!(!whispered(&mut o).await, "not to the other");
    // Huldar and Miran's camp.
    let camp = on(&map, move |m| {
        let at = trigger(&m.realm.data, 171);
        let saean = living(m, 1380, at, 100.0);
        let faction = |m: &Map| {
            saean.first().map(|g| {
                m.unit_fields(*g)
                    .unwrap()
                    .get_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE)
            })
        };
        let was = faction(m);
        step_in(m, other, 171);
        let without = explored(m, other, 273);
        take(m, rogue, 273);
        step_in(m, rogue, 171);
        skip(m, Duration::from_secs(30));
        (
            without,
            explored(m, rogue, 273),
            faction(m) == was,
            living(m, 1981, at, 100.0).len(),
        )
    })
    .await;
    assert_eq!(camp.0, None, "no delivery, nothing");
    assert_eq!(camp.1, Some(true), "the delivery credited");
    assert!(camp.2, "Saean stays as he was");
    assert_eq!(camp.3, 0, "no ambush");
    drop((r, o));
}

/// Thistle Tea's Kalimdor triggers: Lar'korwi's mate answers the scent for
/// a player on Lar'korwi, once while it is about, and another of the
/// scents does nothing for one without; the ancients wake for a hunter
/// who has done The Ancient Leaf, and stay asleep for one partway through
/// it, one past the next, and a warrior; the Greymist camp calls Murkdeep
/// out for a player on WANTED: Murkdeep! and stays quiet for others; at
/// the Affray, Twiggy starts the fight for a player on The Affray.
#[tokio::test]
#[ignore = "extended"]
async fn kalimdor_triggers_answer_those_on_their_quests_and_no_one_else() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, hunter) = hero(&realm, "SIXHTRIGC", Race::NightElf, Class::Hunter).await;
    let (_w, warrior) = hero(&realm, "SIXHTRIGD", Race::NightElf, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let mate = on(&map, move |m| {
        let at = trigger(&m.realm.data, 1731);
        step_in(m, warrior, 1766);
        let without = living(m, 9683, trigger(&m.realm.data, 1766), 100.0).len();
        // (One of them roams near.)
        let before = living(m, 9683, at, 100.0).len();
        take(m, hunter, 4291);
        step_in(m, hunter, 1731);
        let one = living(m, 9683, at, 100.0).len() - before;
        step_in(m, hunter, 1731);
        (without, one, living(m, 9683, at, 100.0).len() - before)
    })
    .await;
    assert_eq!(
        mate,
        (0, 1, 1),
        "the mate comes once, for the scent's quest"
    );
    let ancients = [14_524, 14_525, 14_526];
    let woken = on(&map, move |m| {
        let at = trigger(&m.realm.data, 3587);
        let count = |m: &Map| {
            ancients
                .iter()
                .map(|e| living(m, *e, at, 150.0).len())
                .collect::<Vec<_>>()
        };
        let mut out = Vec::new();
        // Partway: The Ancient Leaf taken, not done.
        take(m, hunter, 7632);
        step_in(m, hunter, 3587);
        out.push(count(m));
        drop_quest(m, hunter, 7632);
        // Past the next one too.
        m.players
            .get_mut(&hunter)
            .unwrap()
            .journal
            .rewarded
            .extend([7632, 7636]);
        step_in(m, hunter, 3587);
        out.push(count(m));
        m.players
            .get_mut(&hunter)
            .unwrap()
            .journal
            .rewarded
            .remove(&7636);
        // A warrior who has done it.
        m.players
            .get_mut(&warrior)
            .unwrap()
            .journal
            .rewarded
            .insert(7632);
        step_in(m, warrior, 3587);
        out.push(count(m));
        // The hunter who has.
        step_in(m, hunter, 3587);
        out.push(count(m));
        out
    })
    .await;
    assert_eq!(
        woken,
        vec![vec![0, 0, 0], vec![0, 0, 0], vec![0, 0, 0], vec![1, 1, 1]],
        "the ancients wake for the hunter who did The Ancient Leaf alone"
    );
    let murkdeep = on(&map, move |m| {
        let at = trigger(&m.realm.data, 1966);
        step_in(m, warrior, 1966);
        let quiet = living(m, 10_323, at, 150.0).len();
        take(m, hunter, 4740);
        step_in(m, hunter, 1966);
        (quiet, living(m, 10_323, at, 150.0).len())
    })
    .await;
    assert_eq!(murkdeep, (0, 1), "Murkdeep, for the one who wants him");
    let affray = on(&map, move |m| {
        let at = trigger(&m.realm.data, 522);
        let twiggy = living(m, 6248, at, 60.0).len();
        step_in(m, warrior, 522);
        skip(m, Duration::from_secs(3));
        let quiet = living(m, 6240, at, 150.0).len();
        take(m, hunter, 1719);
        step_in(m, hunter, 522);
        for _ in 0..3 {
            skip(m, Duration::from_secs(1));
        }
        (twiggy, quiet, living(m, 6240, at, 150.0).len())
    })
    .await;
    assert_eq!(affray.0, 1, "Twiggy at the ring");
    assert_eq!(affray.1, 0, "no fight for one not on The Affray");
    assert!(affray.2 > 0, "the first challenger, for one on it");
}

/// The steps of the first script `id` of `table` starts, by its
/// `start_script` (39) or `start_script_for_all` (68).
fn sub<'a>(d: &'a GameData, step: &ScriptStep) -> &'a [ScriptStep] {
    let id = match step.command {
        39 | 68 => step.datalong[0],
        _ => 0,
    };
    d.scripts.generic.get(&id).map(Vec::as_slice).unwrap_or(&[])
}

fn condition(d: &GameData, id: u32) -> &Condition {
    &d.scripts.conditions[&id]
}

/// Thistle Tea's dungeon triggers, as data (Thistle Tea's own tests read
/// them so): Sentry Point's tower entrance answers as the door does;
/// Zum'rah turns on intruders once a copy while he is near; Antu'sul
/// hatches four broodlings and comes down to the basin; the Ring of Law
/// asks its copy to start the fight; the Dragonspine Door opens for the
/// Seal of Ascension; The Beast leaves its furnace while idle.
#[test]
#[ignore = "extended"]
fn the_dungeon_triggers_are_thistle_teas() {
    if !have_data() {
        return;
    }
    let d = crate::game::data::load::real_data().expect("the real world");
    let at = &d.scripts.areatrigger;
    // Sentry Point: the tower's entrance the same as the door.
    // (A summon's steps are a script of their own, by an id of their own.)
    let shape = |s: &[ScriptStep]| {
        s.iter()
            .map(|x| {
                let visit = d
                    .scripts
                    .generic
                    .get(&(x.dataint[1] as u32))
                    .filter(|_| x.command == 10)
                    .map(|v| {
                        v.iter()
                            .map(|y| (y.command, y.datalong, y.delay_ms))
                            .collect::<Vec<_>>()
                    });
                (x.command, x.datalong, x.dataint[0], x.delay_ms, visit)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&at[&302]), shape(&at[&1667]));
    assert!(shape(&at[&302])[0].4.as_ref().is_some_and(|v| v.len() == 4));
    // Zum'rah: once a copy (instance data 4 at 0), with him near.
    let [wake] = &at[&962][..] else {
        panic!("one step")
    };
    assert_eq!(wake.command, 39);
    assert_ne!(wake.condition, 0);
    let [mark, rouse] = sub(d, wake) else {
        panic!("two steps")
    };
    assert_eq!(
        (mark.command, mark.datalong[0], mark.datalong[1]),
        (37, 4, 1)
    );
    assert_eq!((rouse.command, rouse.target_param1), (39, 7271));
    let zumrah: Vec<u32> = sub(d, rouse).iter().map(|s| s.command).collect();
    assert_eq!(zumrah, vec![4, 22, 0], "unflagged, hostile, he speaks");
    assert_eq!(sub(d, rouse)[1].datalong[0], 37);
    assert_eq!(sub(d, rouse)[2].dataint[0], 3622);
    // Antu'sul: four broodlings, and down to the basin.
    let [antu] = &at[&1447][..] else {
        panic!("one step")
    };
    let [mark, wake] = sub(d, antu) else {
        panic!("two steps")
    };
    assert_eq!(
        (mark.command, mark.datalong[0], mark.datalong[1]),
        (37, 5, 1)
    );
    assert_eq!((wake.target_param1, wake.target_param2), (8127, 100));
    let lunch = sub(d, wake);
    assert_eq!((lunch[0].command, lunch[0].dataint[0]), (0, 4166));
    let broods: Vec<&ScriptStep> = lunch.iter().filter(|s| s.command == 10).collect();
    assert_eq!(broods.len(), 4);
    assert!(
        broods
            .iter()
            .all(|b| (b.datalong[0], b.datalong[1]) == (8138, 25_000))
    );
    let descend = lunch.last().unwrap();
    assert_eq!(descend.command, 3);
    assert!((descend.x - 1_805.133_7).abs() < 0.01 && (descend.y - 740.349_3).abs() < 0.01);
    // The Ring of Law.
    let [ring] = &at[&1526][..] else {
        panic!("one step")
    };
    assert_eq!(
        (
            ring.command,
            ring.datalong[0],
            ring.datalong[1],
            ring.condition
        ),
        (37, 0, 1, 0)
    );
    // The Dragonspine Door, for the Seal of Ascension.
    let door = &at[&2046];
    assert!(
        door.iter().any(|s| s.command == 37
            && s.datalong[0] == 5
            && s.datalong[1] == 3
            && s.condition != 0)
    );
    // The Beast, while out of a fight.
    let [beast] = &at[&2066][..] else {
        panic!("one step")
    };
    assert_eq!((beast.command, beast.target_param1), (26, 10_430));
    let c = condition(d, beast.condition);
    assert_eq!(
        c.flags
            & (crate::game::data::scripts::CF_REVERSE_RESULT
                | crate::game::data::scripts::CF_SWAP_TARGETS),
        crate::game::data::scripts::CF_REVERSE_RESULT | crate::game::data::scripts::CF_SWAP_TARGETS,
        "not in combat, judged on The Beast"
    );
}

/// Thistle Tea's outdoor object scripts: the Hand of Iruxos crystal calls
/// one Demon Spirit on whoever touched it; Lard's Picnic Basket springs
/// three kidnappers on the player while none are about; the Inconspicuous
/// Landmark brings five treasure hunters down on the player; the Panther
/// Cage sets the Enraged Panther on a player on the Hypercapacitor Gizmo,
/// and on no one else. And the poisoned well's water (event 5246) calls
/// three toxicologists.
#[tokio::test]
#[ignore = "extended"]
async fn outdoor_objects_and_the_well_call_their_creatures_on_the_player() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, human) = hero(&realm, "SIXHOBJA", Race::Human, Class::Warrior).await;
    let (_e, elf) = hero(&realm, "SIXHOBJB", Race::NightElf, Class::Warrior).await;
    let east = realm.maps.get(&realm, MapKey::world(0));
    let west = realm.maps.get(&realm, MapKey::world(1));
    /// The first spawn of an object of `entry` here, brought in if it
    /// isn't, `g` beside it.
    fn object(m: &mut Map, g: Guid, entry: u32) -> Guid {
        let data = m.realm.data.clone();
        let s = data
            .gameobject_spawns
            .get(&m.key.map)
            .into_iter()
            .flatten()
            .filter(|s| s.entry == entry)
            .min_by_key(|s| s.guid)
            .expect("spawned")
            .clone();
        let at = Vector3d {
            x: s.x,
            y: s.y,
            z: s.z,
        };
        place(
            m,
            g,
            Vector3d {
                x: at.x + 2.0,
                ..at
            },
        );
        if scripts::object_of_spawn(m, s.guid).is_none() {
            crate::game::map::pools::spawn_object(m, s.guid);
        }
        scripts::object_of_spawn(m, s.guid).expect("the object")
    }
    /// Whether the creatures of `entry` near `g` are set on `g`.
    fn on_me(m: &Map, g: Guid, entry: u32) -> (usize, bool) {
        let at = m.players[&g].position();
        let near = living(m, entry, at, 80.0);
        let set = near.iter().all(|c| {
            m.creature(*c)
                .is_some_and(|c| c.combat.victim == g || c.threat.iter().any(|(t, _)| *t == g))
        });
        (near.len(), set)
    }
    let kalimdor = on(&west, move |m| {
        let crystal = object(m, elf, 176_581);
        take(m, elf, 5381);
        scripts::object_scripts(m, elf, crystal);
        let spirit = on_me(m, elf, 11_876);
        let cage = object(m, elf, 176_195);
        scripts::object_scripts(m, elf, cage);
        let not_on_it = on_me(m, elf, 10_992);
        take(m, elf, 5151);
        scripts::object_scripts(m, elf, cage);
        let panther = on_me(m, elf, 10_992);
        let landmark = object(m, elf, 142_189);
        take(m, elf, 2882);
        scripts::object_scripts(m, elf, landmark);
        let hunters: usize = [7_899, 7_901, 7_902]
            .iter()
            .map(|e| on_me(m, elf, *e).0)
            .sum();
        let cask = object(m, elf, 178_145);
        scripts::object_scripts(m, elf, cask);
        let goggeroc = living(m, 11_920, m.players[&elf].position(), 10.0).len();
        (spirit, not_on_it, panther, hunters, goggeroc)
    })
    .await;
    assert_eq!(kalimdor.0, (1, true), "one Demon Spirit, on her");
    assert_eq!(kalimdor.2, (1, true), "the panther set on her");
    assert!(
        !kalimdor.1.1 || kalimdor.1.0 == 0,
        "not set on one off the quest"
    );
    let eastern = on(&east, move |m| {
        let basket = object(m, human, 179_910);
        scripts::object_scripts(m, human, basket);
        let kidnappers = on_me(m, human, 14_748);
        scripts::object_scripts(m, human, basket);
        let again = on_me(m, human, 14_748).0;
        (kidnappers, again)
    })
    .await;
    assert_eq!(eastern.0, (3, true), "three kidnappers, on him");
    assert_eq!(eastern.1, 3, "none more while they are about");
    assert_eq!(kalimdor.3, 5, "five treasure hunters");
    assert_eq!(kalimdor.4, 1, "Goggeroc beside the cask");
    let well = on(&west, move |m| {
        let at = Vector3d {
            x: 335.0,
            y: -2280.0,
            z: 242.0,
        };
        place(m, elf, at);
        scripts::start(m, ScriptTable::Event, 5246, elf, elf);
        living(m, 12_319, at, 40.0).len()
    })
    .await;
    assert_eq!(well, 3, "three toxicologists");
}

/// Thistle Tea's dungeon object and event scripts, as data: the Resonite
/// Cask raises Goggeroc beside it; reading the Incantation of Celebras
/// lets Celebras go on (only for one on his quest); the imp's release
/// kneels the warlock and sets J'eevee loose, who says her lines and
/// credits them; an object no script is ported for keeps its database's.
#[test]
#[ignore = "extended"]
fn dungeon_objects_and_events_are_thistle_teas() {
    if !have_data() {
        return;
    }
    let d = crate::game::data::load::real_data().expect("the real world");
    let hello = |entry: u32| {
        let id = d.scripts.object_hello_entry[&entry];
        &d.scripts.generic[&id]
    };
    // The Resonite Cask: Goggeroc for five minutes.
    let [cask] = &hello(178_145)[..] else {
        panic!("one step")
    };
    assert_eq!(
        (cask.command, cask.datalong[0], cask.datalong[1]),
        (10, 11_920, 300_000)
    );
    // The Incantation: Celebras let go, for one on his quest.
    let [read] = &hello(178_965)[..] else {
        panic!("one step")
    };
    assert_ne!(read.condition, 0);
    let release = sub(d, read);
    assert_eq!(
        release.iter().map(|s| s.command).collect::<Vec<_>>(),
        vec![scripts::C_RELEASE_WAYPOINTS, 41],
        "Celebras released, the tome gone"
    );
    // The imp's release: the warlock kneels, J'eevee comes for three
    // minutes.
    let imp = &d.scripts.event[&8438];
    assert_eq!(imp[0].command, 1, "an emote");
    assert_eq!(imp[0].datalong[0], 16, "kneeling");
    let jeevee = imp.iter().find(|s| s.command == 10).expect("J'eevee");
    assert_eq!((jeevee.datalong[0], jeevee.datalong[1]), (14_500, 180_000));
    assert_eq!((jeevee.dataint[2], jeevee.dataint[3]), (-1, 1));
    assert_ne!(
        jeevee.data_flags & crate::game::data::scripts::SF_TARGET_SELF,
        0,
        "on the warlock"
    );
    // Her wander: twelve swings, then the credit, the teleport and, two
    // seconds on, gone; her four lines in order; walking but for the dash
    // between the benches.
    let wander = &d.scripts.generic[&(jeevee.dataint[1] as u32)];
    assert!(wander.is_sorted_by_key(|s| s.delay_ms));
    let of = |c: u32| wander.iter().filter(move |s| s.command == c);
    let [credit] = of(8).collect::<Vec<_>>()[..] else {
        panic!("one credit")
    };
    assert_eq!(credit.datalong[0], 14_500);
    let [teleport] = of(15).collect::<Vec<_>>()[..] else {
        panic!("one cast")
    };
    assert_eq!(teleport.datalong[0], 7_791);
    let gone = wander.last().unwrap();
    assert_eq!(gone.command, 18);
    assert_eq!(of(1).count(), 12);
    assert!(of(1).all(|s| s.delay_ms < credit.delay_ms));
    assert!(credit.delay_ms <= teleport.delay_ms && gone.delay_ms - teleport.delay_ms == 2_000);
    assert!((30_000..=40_000).contains(&gone.delay_ms));
    let lines: Vec<i32> = of(0).map(|s| s.dataint[0]).collect();
    assert_eq!(lines, vec![9_769, 9_770, 9_771, 9_742]);
    let pace: Vec<&str> = of(3)
        .map(|s| {
            if s.datalong[2] & 0x4 != 0 {
                "run"
            } else {
                "walk"
            }
        })
        .collect();
    assert_eq!(
        pace,
        [
            "walk", "walk", "walk", "walk", "walk", "walk", "walk", "walk", "run", "run", "run",
            "walk", "walk"
        ]
    );
    // The poisoned well's water: three toxicologists, two minutes each.
    let well = &d.scripts.event[&5246];
    assert_eq!(well.len(), 3);
    assert!(
        well.iter()
            .all(|s| (s.command, s.datalong[0], s.datalong[1]) == (10, 12_319, 120_000))
    );
    // Unported objects keep their database scripts.
    let kept = d
        .scripts
        .gameobject
        .keys()
        .filter(|spawn| {
            d.gameobject_spawns
                .values()
                .flatten()
                .find(|s| s.guid == **spawn)
                .is_some_and(|s| !d.scripts.object_hello_entry.contains_key(&s.entry))
        })
        .count();
    assert!(kept > 0, "the database's object scripts stay");
}
