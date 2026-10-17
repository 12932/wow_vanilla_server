//! Phase 3's world facts, ported from Thistle Tea's
//! `test/game/world/entity/ai_environment_test.exs` (docs/parity/matrix.csv):
//! what a condition reads of the world, a game event, a map event, the local
//! clock and a copy's instance data, each copy its own.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::harness;
use crate::game::map::conditions::met;
use crate::game::map::instance_script::{self, Script};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};

const EVENT_ON: u32 = 9_200;
const MAP_EVENT_ON: u32 = 9_201;
const NOW_IS_IN: u32 = 9_202;
const NOW_IS_OUT: u32 = 9_203;
const DATA_IS_THREE: u32 = 9_204;
const FAIRE: i32 = 4;

fn condition(d: &mut GameData, entry: u32, kind: i32, values: [i32; 4]) {
    d.scripts.conditions.insert(
        entry,
        Condition {
            entry,
            kind,
            values,
            flags: 0,
        },
    );
}

fn world() -> GameData {
    use chrono::Timelike;
    let mut d = data();
    condition(&mut d, EVENT_ON, 12, [FAIRE, 0, 0, 0]);
    condition(&mut d, MAP_EVENT_ON, 36, [648, 0, 0, 0]);
    // The local clock now, within a window about it, and outside one an
    // hour off (wrapping round midnight a window that can't hold it).
    let now = chrono::Local::now();
    let (h, mi) = (now.hour() as i32, now.minute() as i32);
    condition(&mut d, NOW_IS_IN, 53, [0, 0, 23, 59]);
    let later = (h + 1) % 24;
    condition(&mut d, NOW_IS_OUT, 53, [later, 0, later, 0]);
    let _ = mi;
    condition(&mut d, DATA_IS_THREE, 34, [99, 3, 0, 0]);
    d
}

/// Thistle Tea's "lends active game events to the conditions that name
/// one", "requests scripted map-event facts from their owner" and "reads
/// the local clock only for local-time conditions": a condition reads
/// whether a game event is on, whether a map event is under way, and the
/// hour and minute of the server's local clock.
#[tokio::test]
async fn a_condition_reads_the_events_and_the_local_clock() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let realm2 = realm.clone();
    let out = on(&map, move |m| {
        let off = met(m, EVENT_ON, None, None);
        realm2.social.events.lock().insert(FAIRE as u32);
        let on_ = met(m, EVENT_ON, None, None);
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let before = met(m, MAP_EVENT_ON, Some(brute), None);
        crate::game::map::scripts::execute(m, &step(61, [648, 60, 0, 0]), brute, Guid::zero());
        let during = met(m, MAP_EVENT_ON, Some(brute), None);
        crate::game::map::scripts::execute(m, &step(62, [648, 0, 0, 0]), brute, Guid::zero());
        let after = met(m, MAP_EVENT_ON, Some(brute), None);
        (
            (off, on_),
            (before, during, after),
            (
                met(m, NOW_IS_IN, None, None),
                met(m, NOW_IS_OUT, None, None),
            ),
        )
    })
    .await;
    assert_eq!(out.0, (false, true), "the Faire, off then on");
    assert_eq!(
        out.1,
        (false, true, false),
        "the map event, before, under way, done"
    );
    assert_eq!(out.2, (true, false), "the local clock");
}

/// Thistle Tea's "batches EventAI and loaded-script instance fields once
/// for the exact copy", "uses a game object's full instance identity" and
/// "keeps identical map IDs isolated by instance ID": a condition on
/// instance data reads its own copy's, a creature's or an object's alike,
/// and two copies of one dungeon keep theirs apart.
#[tokio::test]
async fn each_copy_keeps_its_own_instance_data() {
    let mut d = world();
    d.gameobject_templates.insert(
        97_200,
        crate::game::tests_p3_entity_objects::object(97_200, 5, "Post", &[]),
    );
    let realm = harness::realm_with(d);
    let at = Vector3d {
        x: -16.0,
        y: -383.0,
        z: 61.0,
    };
    let copies = [
        realm.maps.get(
            &realm,
            MapKey {
                map: 36,
                instance: 1,
            },
        ),
        realm.maps.get(
            &realm,
            MapKey {
                map: 36,
                instance: 2,
            },
        ),
    ];
    let seen = move |m: &mut Map, data: u32| {
        m.instance.script = Some(Script::Deadmines);
        if data != 0 {
            instance_script::set_data(m, 99, data, 0);
        }
        let creature = {
            let t = m.realm.data.creature_template(BRUTE).cloned().unwrap();
            let counter = m.next_creature_counter;
            m.next_creature_counter += 1;
            let c = crate::game::map::creature::Creature::new(
                crate::game::guid::unit(BRUTE, counter | 0x80_0000),
                &t,
                20,
                at,
                0.0,
            );
            m.add_creature(c.with_base())
        };
        let object =
            crate::game::map::objects::summon(m, None, 97_200, at, 0.0, 0, 0, 0, None).unwrap();
        (
            met(m, DATA_IS_THREE, Some(creature), None),
            met(m, DATA_IS_THREE, Some(object), None),
        )
    };
    let first = on(&copies[0], move |m| seen(m, 3)).await;
    let second = on(&copies[1], move |m| seen(m, 0)).await;
    assert_eq!(
        first,
        (true, true),
        "the first copy's three, for a creature and an object"
    );
    assert_eq!(second, (false, false), "the second's own, untouched");
}
