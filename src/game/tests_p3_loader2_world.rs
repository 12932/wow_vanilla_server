//! Phase 3's second loader stage, the world (Thistle Tea's
//! `world/loader/alterac_valley_vmangos_test.exs`, `area_trigger_test.exs`
//! and the world tests beside them): Alterac Valley's riders, commanders,
//! donations and supplies, and the real area triggers, over the real data
//! when it is here.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::Logged;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// Thistle Tea's Alterac Valley seed (`alterac_valley_vmangos_test.exs`):
/// the six named riders have looks, factions and their launch lines; each
/// wing commander gives her supply quest (an item, no log) and walks a
/// route home past her last point; the altars' overrides summon one boss
/// apiece; the armor donations take twenty scraps; every graveyard
/// event's eight states have spawns; and the six supplies, each of its
/// mine's event and of states 0 to 2, are their mines'.
#[test]
#[ignore = "extended"]
fn the_real_alterac_valley_loads_as_thistle_tea_reads_it() {
    let Some(d) = real() else {
        return;
    };
    for (entry, text) in [
        (14_943, 10_341),
        (14_944, 10_343),
        (14_945, 10_346),
        (14_946, 10_351),
        (14_947, 10_349),
        (14_948, 10_353),
    ] {
        let t = d.creature_template(entry).unwrap();
        assert!(t.display_ids[0] > 0 && t.faction > 0, "{entry}");
        assert!(d.npc.broadcast_line(text, false).is_some(), "{text}");
    }
    for (entry, quest, item, last) in [
        (13_179, 6825, 17_326, 74),
        (13_180, 6826, 17_327, 84),
        (13_181, 6827, 17_328, 97),
        (13_438, 6942, 17_502, 66),
        (13_439, 6941, 17_503, 76),
        (13_437, 6943, 17_504, 92),
    ] {
        let q = &d.npc.quests[&quest];
        assert_eq!(
            (q.required_items[0], q.auto_complete),
            ((item, 1), true),
            "{quest}"
        );
        assert!(d.bg_routes[&entry].len() > last, "{entry}");
        assert!(d.npc.creature_starts[&entry].contains(&quest), "{entry}");
    }
    for (spell, boss) in [(21_249, 13_256), (21_648, 13_419)] {
        let e = d.spells.get(spell).unwrap().effects[1];
        assert_eq!(
            (e.effect, e.misc_value, e.target_a, e.radius_index),
            (41, boss, 32, 12),
            "{spell}"
        );
    }
    for quest in [6741, 6781, 7223, 7224] {
        assert_eq!(
            d.npc.quests[&quest].required_items[0],
            (17_422, 20),
            "{quest}"
        );
    }
    let valley: Vec<u32> = d.creature_spawns[&30].iter().map(|s| s.guid).collect();
    let events: ahash::AHashSet<(u8, u8)> = valley
        .iter()
        .filter_map(|g| d.bg_creature_events.get(g))
        .flatten()
        .copied()
        .collect();
    for event in 15..=21u8 {
        for state in 0..=7u8 {
            assert!(events.contains(&(event, state)), "({event}, {state})");
        }
    }
    let supplies: ahash::AHashSet<(u8, u8, u32)> = d.gameobject_spawns[&30]
        .iter()
        .flat_map(|s| {
            d.bg_object_events
                .get(&s.guid)
                .into_iter()
                .flatten()
                .filter(|(e, _)| matches!(e, 50 | 51))
                .map(move |(e, st)| (*e, *st, s.entry))
        })
        .collect();
    assert_eq!(supplies.len(), 6);
    for (event, state, entry) in supplies {
        assert!(state <= 2, "{entry}");
        assert_eq!(
            crate::game::map::battleground::av::supply_mine(entry),
            Some(usize::from(event - 50)),
            "{entry}"
        );
    }
}

/// Where a trigger is.
fn at(d: &GameData, trigger: u32) -> Vector3d {
    let t = &d.area_triggers[&trigger];
    Vector3d {
        x: t.x,
        y: t.y,
        z: t.z,
    }
}

/// Thistle Tea's area triggers (`area_trigger_test.exs`): Ragefire Chasm's
/// entrance wants level 8 and lands at its own spot, and from another map
/// does nothing; Ravenholdt's (Thistle Tea's port of its script) gives the
/// kill credit to a rogue on The Manor, Ravenholdt, and to nobody else;
/// Brother Sarno greets at the cathedral's doors (the speaker found, the
/// targets swapped), then rests thirty seconds; a trigger of no script
/// runs nothing.
#[tokio::test]
#[ignore = "extended"]
async fn real_area_triggers_teleport_credit_and_greet() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    let rfc = d.area_triggers[&2230].teleport.clone().unwrap();
    assert_eq!(
        (
            rfc.required_level,
            rfc.map,
            rfc.x,
            rfc.y,
            rfc.z,
            rfc.orientation
        ),
        (8, 389, 0.797_643, -8.234_29, -15.5288, 4.712_39)
    );
    // Ragefire's runs no script.
    assert!(!d.scripts.areatrigger.contains_key(&2230));
    assert!(!d.scripts.areatrigger_script.contains_key(&2230));
    let sarno = &d.scripts.areatrigger[&1125];
    assert_eq!(
        (
            sarno[0].command,
            sarno[0].dataint[0],
            sarno[0].data_flags & 0x2
        ),
        (0, 3988, 0x2)
    );
    assert_eq!(d.scripts.areatrigger_cooldown.get(&1125), Some(&30));
    let ravenholdt = &d.scripts.areatrigger[&3066];
    assert_eq!(
        (ravenholdt[0].command, ravenholdt[0].datalong[0]),
        (8, 13_936)
    );

    let realm = realm();
    let mut c = Client::connect(&realm, "P3OTRIG1");
    let rogue = c.enter("Trigga", Race::Human, Class::Rogue).await;
    let mut o = Client::connect(&realm, "P3OTRIG2");
    let other = o.enter("Triggb", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (raven, sarno_rest, elsewhere) = {
        let (ravenholdt, rfc, sarno) = (at(d, 3066), at(d, 2230), at(d, 1125));
        on(&map, move |m| {
            let place = |m: &mut Map, g: Guid, to: Vector3d| {
                m.players.get_mut(&g).unwrap().movement.position = to;
            };
            m.players.get_mut(&rogue).unwrap().journal.add(Logged {
                quest: 6681,
                ..Default::default()
            });
            for g in [rogue, other] {
                place(m, g, ravenholdt);
                crate::game::map::world::area_trigger(m, g, 3066);
            }
            let kills = |m: &Map, g: Guid| {
                let j = &m.players[&g].journal;
                j.slot_of(6681).map(|s| j.log[s].kills[0])
            };
            let raven = (kills(m, rogue), kills(m, other));
            // Ragefire's, from the Eastern Kingdoms.
            place(m, rogue, rfc);
            crate::game::map::world::area_trigger(m, rogue, 2230);
            let elsewhere = m.players[&rogue].position() == rfc && m.players.contains_key(&rogue);
            place(m, rogue, sarno);
            let now = m.now;
            crate::game::map::world::area_trigger(m, rogue, 1125);
            let rest = m
                .trigger_cooldowns
                .get(&1125)
                .map(|t| t.duration_since(now));
            (raven, rest, elsewhere)
        })
        .await
    };
    assert_eq!(raven, (Some(1), None), "the rogue on the quest's");
    assert!(elsewhere, "no teleport from another map");
    assert_eq!(sarno_rest, Some(Duration::from_secs(30)));

    // Ragefire's in Orgrimmar: at 7, refused; at 8, in.
    let mut h = Client::connect(&realm, "P3OTRIG3");
    let orc = h.enter("Triggc", Race::Orc, Class::Warrior).await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let rfc = at(d, 2230);
    let data = realm.data.clone();
    let stayed = on(&kalimdor, move |m| {
        let p = m.players.get_mut(&orc).unwrap();
        crate::game::map::stats::set_level(p, &data, 7);
        // Not a game master, whom no level bars.
        p.gm = false;
        p.movement.position = rfc;
        crate::game::map::world::area_trigger(m, orc, 2230);
        m.players.contains_key(&orc)
    })
    .await;
    assert!(stayed, "refused at level 7");
    let data = realm.data.clone();
    on(&kalimdor, move |m| {
        let p = m.players.get_mut(&orc).unwrap();
        crate::game::map::stats::set_level(p, &data, 8);
        crate::game::map::world::area_trigger(m, orc, 2230);
    })
    .await;
    let landed = h
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_NEW_WORLD(w) => {
                Some((w.map, w.position.x, w.position.y))
            }
            _ => None,
        })
        .await;
    assert_eq!(landed.0, wow_world_messages::vanilla::Map::RagefireChasm);
    assert!((landed.1 - 0.797_643).abs() < 0.01 && (landed.2 + 8.234_29).abs() < 0.01);
    let _ = (c.guid, o.guid);
}
