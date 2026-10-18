//! Thistle Tea's instance scripts (`test/game/core/instance_script/*`) on
//! the real world: each dungeon's copy entered, its fields set, its
//! creatures killed and its objects used as its script hears of them, and
//! what came of it read off the copy (doors, creatures, the data) or heard
//! as a player's client hears it.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::instance_script::{self, DONE, Event, IN_PROGRESS};
use crate::game::map::{Map, MapHandle};

/// Where a spawn of creature `entry` in `map` stands, or else of object
/// `entry`.
pub(super) fn spawn_of(realm: &Realm, map: u32, entry: u32) -> Vector3d {
    let d = &realm.data;
    if let Some(s) = d
        .creature_spawns
        .get(&map)
        .and_then(|v| v.iter().find(|s| s.entries.contains(&entry)))
    {
        return Vector3d {
            x: s.x,
            y: s.y,
            z: s.z,
        };
    }
    let o = d
        .gameobject_spawns
        .get(&map)
        .and_then(|v| v.iter().find(|s| s.entry == entry))
        .unwrap_or_else(|| panic!("a spawn of {entry} in {map}"));
    Vector3d {
        x: o.x,
        y: o.y,
        z: o.z,
    }
}

/// A [`hero`] in a fresh copy of `map`, by the spawn of `near`.
pub(super) async fn inside(
    realm: &Arc<Realm>,
    name: &str,
    map: u32,
    near: u32,
) -> (Client, Guid, MapHandle) {
    let (mut c, g, _) = hero(realm, name).await;
    let at = spawn_of(realm, map, near);
    let to = go(realm, &mut c, map, at).await;
    (c, g, to)
}

/// The state of the first object of `entry` (0 open, 1 shut, 2 broken).
pub(super) fn door(m: &Map, entry: u32) -> Option<u32> {
    m.gameobjects
        .iter()
        .find(|(_, o)| o.entry == entry)
        .map(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE))
}

pub(super) fn data(m: &Map, field: u32) -> u32 {
    instance_script::get_data(m, field)
}

pub(super) fn set(m: &mut Map, field: u32, value: u32) {
    instance_script::set_data(m, field, value, 0);
}

/// The doors of `entries` shut by hand, then the copy's doors put in line
/// with its data, as when they spawn again: each one's state.
pub(super) fn respawned(m: &mut Map, entries: &[u32]) -> Vec<Option<u32>> {
    for e in entries {
        let gs: Vec<Guid> = m
            .gameobjects
            .iter()
            .filter(|(_, o)| o.entry == *e)
            .map(|(_, o)| o.guid)
            .collect();
        for g in gs {
            if let Some(o) = m.gameobject_mut(g) {
                o.fields.set_u32(GAMEOBJECT_STATE, 1);
            }
        }
    }
    instance_script::settle_spawns(m);
    entries.iter().map(|e| door(m, *e)).collect()
}

/// Whether `g` is hidden from players by its script.
pub(super) fn concealed(m: &Map, g: Guid) -> bool {
    crate::game::map::event_ai::scripted(m, g).is_some_and(|s| s.concealed)
}

/// What `c` hears creature `speaker` say, yell or emote within `within`.
pub(super) async fn spoke(c: &mut Client, speaker: Guid, within: Duration) -> Option<String> {
    c.gets(within, move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => match &s.chat_type {
            SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. }
            | SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. }
                if *sender1 == speaker =>
            {
                Some(s.message.clone())
            }
            SMSG_MESSAGECHAT_ChatType::MonsterEmote { monster, .. }
            | SMSG_MESSAGECHAT_ChatType::RaidBossEmote { monster, .. }
                if *monster == speaker =>
            {
                Some(s.message.clone())
            }
            _ => None,
        },
        _ => None,
    })
    .await
}

/// The text of broadcast text `id`, as a male speaker says it (or the
/// female text, if it has only that).
pub(super) fn text(realm: &Realm, id: u32) -> String {
    realm
        .data
        .npc
        .broadcast_texts
        .get(&id)
        .map(|t| {
            if t.male.is_empty() {
                t.female.clone()
            } else {
                t.male.clone()
            }
        })
        .unwrap_or_default()
}

/// [`living`] from a handle.
pub(super) async fn alive(map: &MapHandle, entry: u32) -> Vec<Guid> {
    map.query(move |m| living(m, entry)).await.unwrap()
}

/// Kill every living creature of `entry`, as `by`.
pub(super) fn kill_all(m: &mut Map, by: Guid, entry: u32) {
    for g in living(m, entry) {
        kill(m, by, g);
    }
}

/// Shadowfang Keep: a freed prisoner opens the Courtyard Door, once; the
/// fourth voidwalker slain (and nothing else told of them) the Sorcerer's
/// Gate; Nandos the Lair for good; in a fresh copy all three stand shut,
/// and once opened they stay open as they spawn again.
#[tokio::test]
#[ignore = "extended"]
async fn shadowfang_keeps_doors_open_as_its_script_has_them() {
    const COURTYARD: u32 = 18895;
    const LAIR: u32 = 18971;
    const GATE: u32 = 18972;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, _me, map) = inside(&realm, "Keep", 33, COURTYARD).await;
    map.query(|m| {
        for d in [COURTYARD, LAIR, GATE] {
            assert_eq!(door(m, d), Some(1), "{d} shut in a fresh copy");
        }
        assert_eq!(respawned(m, &[COURTYARD, LAIR, GATE]), vec![Some(1); 3]);
        set(m, 1, DONE);
        assert_eq!(door(m, COURTYARD), Some(0), "the prisoner opens it");
        set(m, 1, DONE);
        assert_eq!(data(m, 1), DONE);
        // Voidwalkers: only a slain one counts.
        set(m, 6, 1);
        assert_eq!(data(m, 6), 0);
        for n in 1..=3 {
            set(m, 6, DONE);
            assert_eq!(data(m, 6), n);
        }
        set(m, 6, 2);
        assert_eq!((data(m, 6), door(m, GATE)), (3, Some(1)), "three: shut");
        set(m, 6, DONE);
        assert_eq!((data(m, 6), door(m, GATE)), (4, Some(0)), "the fourth");
        // Nandos.
        set(m, 4, IN_PROGRESS);
        assert_eq!(door(m, LAIR), Some(1));
        set(m, 4, DONE);
        assert_eq!(door(m, LAIR), Some(0), "Nandos dead");
        set(m, 4, IN_PROGRESS);
        assert_eq!(data(m, 4), DONE, "for good");
        assert_eq!(respawned(m, &[COURTYARD, LAIR, GATE]), vec![Some(0); 3]);
    })
    .await
    .unwrap();
}

/// The Wailing Caverns: Serpentis cries out when the first of the other
/// Fanglords falls, and not again; all four dead, the disciple calls the
/// party (field four special); Anacondra resting sends the druid at her
/// side away; Mutanus's death takes the nightmare's creatures, Kresh, the
/// disciple and Naralex spared.
#[tokio::test]
#[ignore = "extended"]
async fn the_wailing_caverns_fanglords_and_the_nightmare() {
    const ANACONDRA: u32 = 3671;
    const SERPENTIS: u32 = 3673;
    const DISCIPLE: u32 = 3678;
    const DRUID: u32 = 3840;
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, _me, map) = inside(&realm, "Fang", 43, SERPENTIS).await;
    let serpentis = alive(&map, SERPENTIS).await[0];
    c.sees(serpentis).await;
    c.clear();
    map.query(|m| set(m, 0, DONE)).await;
    assert_eq!(
        spoke(&mut c, serpentis, Duration::from_secs(2)).await,
        Some(text(&realm, 2102)),
        "Serpentis's cry"
    );
    map.query(|m| set(m, 1, DONE)).await;
    assert_eq!(
        spoke(&mut c, serpentis, Duration::from_millis(500)).await,
        None
    );
    let (mut c2, _, map2) = inside(&realm, "Disciple", 43, DISCIPLE).await;
    let disciple = alive(&map2, DISCIPLE).await[0];
    c2.sees(disciple).await;
    map2.query(|m| {
        for f in [0, 1, 3] {
            set(m, f, DONE);
        }
        assert_eq!(data(m, 4), 0);
    })
    .await;
    c2.clear();
    map2.query(|m| set(m, 2, DONE)).await;
    assert_eq!(
        spoke(&mut c2, disciple, Duration::from_secs(2)).await,
        Some(text(&realm, 2101)),
        "the disciple's call"
    );
    assert_eq!(map2.query(|m| data(m, 4)).await, Some(4), "special");
    // Anacondra rests: the druid beside her goes.
    let (_c3, _, map3) = inside(&realm, "Rest", 43, ANACONDRA).await;
    let (beside, before) = map3
        .query(|m| {
            let a = m.position_of(first(m, ANACONDRA).unwrap()).unwrap();
            let beside = living(m, DRUID)
                .into_iter()
                .filter(|d| {
                    m.position_of(*d)
                        .is_some_and(|p| crate::game::map::combat::dist(p, a) <= 5.0)
                })
                .count();
            (beside, living(m, DRUID).len())
        })
        .await
        .unwrap();
    assert!(beside >= 1, "a druid at her side");
    map3.query(|m| set(m, 0, 4)).await;
    assert_eq!(
        map3.query(|m| living(m, DRUID).len()).await,
        Some(before - 1),
        "one druid sent away"
    );
    // Mutanus dies.
    map3.query(|m| {
        let spared: Vec<usize> = [3653, DISCIPLE, 3679]
            .iter()
            .map(|e| living(m, *e).len())
            .collect();
        set(m, 5, DONE);
        assert!(living(m, ANACONDRA).is_empty() && living(m, DRUID).is_empty());
        let after: Vec<usize> = [3653, DISCIPLE, 3679]
            .iter()
            .map(|e| living(m, *e).len())
            .collect();
        assert_eq!(after, spared, "Kresh, the disciple and Naralex spared");
    })
    .await
    .unwrap();
}

/// Razorfen Kraul: the first Ward Keeper slain leaves the ward up; the
/// second drops it and sends Agathelos running his patrol, which he keeps
/// when he spawns again; before that, a spawn of his stands.
#[tokio::test]
#[ignore = "extended"]
async fn razorfen_krauls_ward_keepers_free_agathelos() {
    const AGATHELOS: u32 = 4422;
    const WARD: u32 = 21099;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Kraul", 47, AGATHELOS).await;
    map.query(move |m| {
        let a = first(m, AGATHELOS).unwrap();
        let patrol = |m: &Map| {
            m.creature(a).map(|c| c.movement_type)
                == Some(crate::game::map::creature::MOVEMENT_WAYPOINT)
                && crate::game::map::event_ai::scripted(m, a).is_some_and(|s| s.run)
        };
        assert!(!patrol(m), "held");
        instance_script::creature_event(m, a, Event::Spawned);
        assert!(!patrol(m), "a spawn before the ward drops stands");
        set(m, 1, DONE);
        assert_eq!((data(m, 1), door(m, WARD)), (0, Some(1)), "one keeper");
        assert_eq!(respawned(m, &[WARD]), vec![Some(1)]);
        set(m, 1, DONE);
        assert_eq!((data(m, 1), door(m, WARD)), (DONE, Some(0)), "two");
        assert!(patrol(m), "Agathelos runs his patrol");
        assert_eq!(respawned(m, &[WARD]), vec![Some(0)]);
        // He spawns again, as new.
        if let Some(c) = m.creature_mut(a) {
            c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
        }
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, a) {
            s.run = false;
        }
        instance_script::creature_event(m, a, Event::Spawned);
        assert!(patrol(m), "his patrol kept");
        let _ = me;
    })
    .await
    .unwrap();
}

/// Maraudon: Celebras the Redeemed keeps hidden until the Cursed falls,
/// then shows; spawning again after, he isn't hidden.
#[tokio::test]
#[ignore = "extended"]
async fn maraudons_redeemed_spirit_waits_for_the_cursed() {
    const CURSED: u32 = 12225;
    const REDEEMED: u32 = 13716;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Celebras", 349, CURSED).await;
    map.query(move |m| {
        let r = first(m, REDEEMED).expect("the redeemed spirit");
        assert!(concealed(m, r), "hidden");
        kill_all(m, me, CURSED);
        assert_eq!(data(m, 1), DONE);
        assert!(!concealed(m, r), "shown once the Cursed falls");
        instance_script::creature_event(m, r, Event::Spawned);
        assert!(!concealed(m, r), "and spawning again");
    })
    .await
    .unwrap();
}

/// The Deadmines: each boss's death opens its door, once, and it stays
/// open as it spawns again; the shredder's death opens none; the gunpowder
/// brings one Blackguard down the ramp; the cannon fired opens itself,
/// blows the iron-clad door and (once a copy) starts the alarm: three
/// seconds on, the alarm's call and the pirates' charge, fifteen more the
/// assault; a pirate of the two spawning again charges again, another
/// doesn't; another copy's cannon is its own.
#[tokio::test]
#[ignore = "extended"]
async fn the_deadmines_doors_gunpowder_and_cannon() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = inside(&realm, "Mines", 36, 644).await;
    map.query(move |m| {
        assert_eq!(door(m, 16400), Some(1));
        kill_all(m, me, 642);
        assert_eq!(door(m, 16400), Some(1), "the shredder opens nothing");
        for (boss, d) in [(644, 13965), (643, 16400), (1763, 16399)] {
            assert_eq!(door(m, d), Some(1), "{d} shut");
            kill_all(m, me, boss);
            assert_eq!(door(m, d), Some(0), "{boss}'s death opens {d}");
            assert_eq!(respawned(m, &[d]), vec![Some(0)]);
        }
        // The gunpowder.
        let before = living(m, 634).len();
        instance_script::object_used(m, 17155);
        instance_script::object_used(m, 17155);
        assert_eq!(living(m, 634).len(), before + 1, "one Blackguard");
        // The cannon.
        instance_script::object_used(m, 16398);
        assert_eq!(door(m, 16398), Some(0), "the cannon fired");
        assert_eq!(door(m, 16397), Some(2), "the door blown");
        assert_eq!(data(m, 1), 1);
        let alarms = m.instance.timers.len();
        instance_script::object_used(m, 16398);
        set(m, 1, 1);
        assert_eq!((data(m, 1), m.instance.timers.len()), (1, alarms), "once");
    })
    .await
    .unwrap();
    // The alarm goes three seconds on: its call, and the pirates charge.
    let pirates = map
        .query(|m| {
            hurry_copy(m, Duration::from_millis(2900));
            assert_eq!(data(m, 1), 1, "not before three seconds");
            hurry_copy(m, Duration::from_millis(200));
            assert_eq!(data(m, 1), 2, "the alarm");
            [79289u32, 79290].map(|s| crate::game::map::scripts::creature_of_spawn(m, s))
        })
        .await
        .unwrap();
    assert!(pirates.iter().all(|p| p.is_some()), "the two pirates");
    let charging = |m: &Map, g: Guid| {
        m.creature(g)
            .is_some_and(|c| c.movement_type == crate::game::map::creature::MOVEMENT_IDLE)
    };
    map.query(move |m| {
        for p in pirates.iter().flatten() {
            assert!(charging(m, *p), "a pirate off its patrol");
        }
        hurry_copy(m, Duration::from_secs(14));
        assert_eq!(data(m, 1), 2);
        hurry_copy(m, Duration::from_millis(1100));
        assert_eq!(data(m, 1), 3, "the assault");
        // A pirate of the two spawning again charges again.
        let p = pirates[0].unwrap();
        if let Some(c) = m.creature_mut(p) {
            c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
        }
        instance_script::creature_event(m, p, Event::Spawned);
        assert!(charging(m, p), "charges again");
    })
    .await
    .unwrap();
    // Another copy's cannon is its own.
    let (_c2, _, other) = inside(&realm, "Other", 36, 644).await;
    other
        .query(|m| {
            assert_eq!((data(m, 1), door(m, 16397)), (0, Some(1)));
            instance_script::object_used(m, 16398);
            assert_eq!(door(m, 16397), Some(2));
        })
        .await
        .unwrap();
}

/// The Scarlet Monastery's cathedral: Mograine pulled brings the chapel's
/// defenders; his fall opens the door, and Whitemane yells and runs to him;
/// her raising him brings them both on; a wipe before either dies shuts
/// the door and sends her home; a wipe after one died takes the survivor
/// and settles the fight; both dead settles it, the door open; a dead
/// Whitemane neither yells nor runs; Herod's death opens his door for
/// good; a boss spawning again isn't counted dead.
#[tokio::test]
#[ignore = "extended"]
async fn the_scarlet_cathedral_stages_mograine_and_whitemane() {
    const MOGRAINE: u32 = 3976;
    const WHITEMANE: u32 = 3977;
    const DOOR: u32 = 104600;
    const HEROD_DOOR: u32 = 101854;
    let Some(realm) = realm() else {
        return;
    };
    // Herod, in the Armory.
    let (_h, me, map) = inside(&realm, "Herod", 189, 3975).await;
    map.query(move |m| {
        assert_eq!(door(m, HEROD_DOOR), Some(1));
        kill_all(m, me, 3975);
        assert_eq!(door(m, HEROD_DOOR), Some(0), "Herod's door");
        assert_eq!(respawned(m, &[HEROD_DOOR]), vec![Some(0)]);
    })
    .await
    .unwrap();
    // The cathedral, staged.
    let (mut c, _, map) = inside(&realm, "Cathedral", 189, WHITEMANE).await;
    let whitemane = alive(&map, WHITEMANE).await[0];
    c.sees(whitemane).await;
    let home = map.query(move |m| m.position_of(whitemane)).await.flatten();
    map.query(|m| {
        set(m, 1, 1);
        assert_eq!(door(m, DOOR), Some(1));
    })
    .await;
    c.clear();
    map.query(|m| set(m, 1, 2)).await;
    assert_eq!(
        spoke(&mut c, whitemane, Duration::from_secs(2)).await,
        Some(text(&realm, 2973)),
        "Whitemane's yell"
    );
    assert_eq!(map.query(|m| door(m, DOOR)).await.flatten(), Some(0));
    assert!(
        until(&map, 3000, move |m| m
            .position_of(whitemane)
            .zip(home)
            .is_some_and(
                |(p, h)| crate::game::map::combat::dist(p, h) > 1.0
            ))
        .await,
        "she runs to him"
    );
    // A wipe before either dies: the door shuts, she goes home.
    map.query(move |m| {
        set(m, 1, 0);
        assert_eq!(door(m, DOOR), Some(1), "shut");
        assert_eq!(respawned(m, &[DOOR]), vec![Some(1)]);
        // Raised, both turn on the cathedral (and Mograine, pulled anew
        // here, reports his fight begun, as his database script has it).
        set(m, 1, 3);
        let pair = [MOGRAINE, WHITEMANE].map(|e| first(m, e).unwrap());
        assert!(pair.iter().all(|g| fighting(m, *g)), "both fight");
        // One dies, then the wipe.
        instance_script::store(m, 1, 3);
        assert_eq!(door(m, DOOR), Some(0));
        reported_death(m, WHITEMANE);
        set(m, 1, 0);
        assert_eq!(data(m, 1), 4, "settled");
        assert!(living(m, MOGRAINE).is_empty(), "the survivor taken");
        set(m, 1, 1);
        assert_eq!(data(m, 1), 4, "settled stays");
    })
    .await
    .unwrap();
    // Both dead settles it, the door open.
    let (_c2, _, map2) = inside(&realm, "Both", 189, WHITEMANE).await;
    map2.query(|m| {
        instance_script::store(m, 1, 3);
        reported_death(m, WHITEMANE);
        reported_death(m, WHITEMANE);
        assert_eq!(data(m, 1), 3, "one of them, twice, isn't both");
        reported_death(m, MOGRAINE);
        assert_eq!((data(m, 1), door(m, DOOR)), (4, Some(0)));
    })
    .await
    .unwrap();
    // A dead Whitemane neither yells nor runs; a respawned boss is alive.
    let (mut c3, _, map3) = inside(&realm, "Dead", 189, WHITEMANE).await;
    let w = alive(&map3, WHITEMANE).await[0];
    c3.sees(w).await;
    map3.query(move |m| reported_death(m, WHITEMANE)).await;
    c3.clear();
    map3.query(|m| set(m, 1, 2)).await;
    assert_eq!(spoke(&mut c3, w, Duration::from_millis(600)).await, None);
    assert_eq!(map3.query(|m| door(m, DOOR)).await.flatten(), Some(0));
    let (_c4, _, map4) = inside(&realm, "Again", 189, MOGRAINE).await;
    map4.query(|m| {
        reported_death(m, WHITEMANE);
        let g = first(m, WHITEMANE).unwrap();
        instance_script::creature_event(m, g, Event::Spawned);
        set(m, 1, 0);
        assert_eq!(data(m, 1), 0, "no one counted dead");
    })
    .await
    .unwrap();
}

/// The cathedral pair's death as the copy's script hears of it: neither
/// falls to a blow here (Whitemane waits out of reach behind her door,
/// Mograine feigns his first fall).
fn reported_death(m: &mut Map, entry: u32) {
    let g = first(m, entry).unwrap();
    instance_script::creature_event(m, g, Event::Death);
}

/// The real world's dungeons as their scripts want them: each named for
/// its script, each sealed passage one shut door, and the creatures that
/// open them reporting their deaths: Nandos opens Arugal's Lair, the two
/// Ward Keepers (and only both) Agathelos's ward, Mograine's pull starts
/// the cathedral's fight.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_dungeons_report_to_their_scripts() {
    let Some(realm) = realm() else {
        return;
    };
    for (map, script) in [
        (33, "instance_shadowfang_keep"),
        (47, "instance_razorfen_kraul"),
        (109, "instance_sunken_temple"),
        (189, "instance_scarlet_monastery"),
    ] {
        assert_eq!(realm.data.scripts.map_templates[&map].script_name, script);
    }
    for (map, door) in [
        (33, 18895),
        (33, 18971),
        (33, 18972),
        (47, 21099),
        (109, 149431),
        (189, 104600),
    ] {
        let states: Vec<u8> = realm.data.gameobject_spawns[&map]
            .iter()
            .filter(|s| s.entry == door)
            .map(|s| s.state)
            .collect();
        assert_eq!(states, vec![1], "{door}: one, shut");
    }
    assert_eq!(
        realm.data.creature_spawns[&47]
            .iter()
            .filter(|s| s.entries.contains(&4625))
            .count(),
        2,
        "two Ward Keepers"
    );
    // Nandos.
    let (_c, me, map) = inside(&realm, "Nandos", 33, 3927).await;
    map.query(move |m| kill_all(m, me, 3927)).await;
    assert!(
        until(&map, 3000, |m| data(m, 4) == DONE
            && door(m, 18971) == Some(0))
        .await,
        "Nandos's death opens the Lair"
    );
    // The Ward Keepers.
    let (_c, me, map) = inside(&realm, "Wards", 47, 4625).await;
    let keepers = alive(&map, 4625).await;
    let (k0, k1) = (keepers[0], keepers[1]);
    assert_eq!(keepers.len(), 2);
    map.query(move |m| kill(m, me, k0)).await;
    sleep_ms(500).await;
    assert_eq!(
        map.query(|m| door(m, 21099)).await.flatten(),
        Some(1),
        "one keeper: the ward holds"
    );
    map.query(move |m| kill(m, me, k1)).await;
    assert!(
        until(&map, 3000, |m| data(m, 1) == DONE
            && door(m, 21099) == Some(0))
        .await,
        "both: the ward drops"
    );
    // Mograine pulled.
    let (_c, me, map) = inside(&realm, "Pull", 189, 3976).await;
    map.query(move |m| {
        let g = first(m, 3976).unwrap();
        crate::game::map::ai::attacked(m, g, me);
    })
    .await;
    assert!(
        until(&map, 3000, |m| data(m, 1) == IN_PROGRESS).await,
        "his pull starts the fight"
    );
}
