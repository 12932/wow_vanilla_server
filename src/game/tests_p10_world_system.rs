//! Round 10's tests of the world's systems, ported from Thistle Tea's
//! `test/game/world/system/` and `loot/`: a call Omen never answers let go
//! (`minions_of_omen_test.exs`), the Scourge sent to a zone at once
//! (`scourge_invasion_test.exs`), and a mine's supplies kept from everyone
//! with no match (`actor_factory_test.exs`).

use wow_world_messages::vanilla::*;

use crate::game::data::{GameData, GameObjectSpawn, GameObjectTemplate};
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::invasion::{self, AttackRefused};
use crate::game::map::MapKey;
use crate::game::omen::{LAUNCHER, MINIONS_OF_OMEN};
use crate::game::tests_p3_combat::START;

fn object(entry: u32, name: &str, kind: u8) -> GameObjectTemplate {
    GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: name.into(),
        size: 1.0,
        ..Default::default()
    }
}

fn placed(guid: u32, entry: u32) -> GameObjectSpawn {
    GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + 2.0,
        y: START.1 + 2.0,
        z: START.2,
        state: 1,
        ..Default::default()
    }
}

/// Thistle Tea's `MinionsOfOmen` "a call Omen never answers is let go so
/// the fireworks can call him again": with no Omen to rise (his template
/// missing), the twentieth firework from his launcher calls him and brings
/// none; the watch isn't left waiting on him, so the count starts again
/// and the fortieth calls him again, his minions out from the third on
/// (wvs's call raises Omen at once, so a call unanswered is one that
/// raised no one).
#[tokio::test]
async fn a_call_omen_never_answers_leaves_the_fireworks_free_to_call_again() {
    let mut d = GameData::empty();
    d.gameobject_templates
        .insert(LAUNCHER, object(LAUNCHER, "Cluster Launcher", 22));
    d.gameobject_spawns.insert(0, vec![placed(70, LAUNCHER)]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let launcher = guid::game_object(LAUNCHER, 70);
    // The count after each launch.
    let counts = map
        .query(move |m| {
            (0..40)
                .map(|_| {
                    crate::game::map::world::use_object(m, g, launcher);
                    let w = m.realm.social.omen.lock();
                    (w.fireworks, w.omen.is_some(), w.minions)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let calls: Vec<usize> = (0..40).filter(|i| counts[*i].0 == 0).collect();
    assert_eq!(calls, vec![19, 39], "the twentieth and the fortieth call");
    assert!(counts.iter().all(|c| !c.1), "no Omen waited on");
    assert!(!counts[1].2 && counts[2].2, "the minions from the third");
    let driven = realm
        .social
        .driven_events
        .lock()
        .get(&MINIONS_OF_OMEN)
        .copied();
    assert_eq!(driven, Some(true), "the minions' event on");
}

const WINTERSPRING: u32 = 618;
const ATTACKING_WINTERSPRING: u32 = 90;

/// Thistle Tea's `ScourgeInvasion.attack/2` "sends the Scourge to a zone
/// at once": with the invasion on and Winterspring's three necropolises
/// fallen and its victory counted (it rests), `.invasion attack 618` sends
/// the Scourge back there at once, its event on again with three
/// necropolises standing; asked again, it is under attack already. Off,
/// the invasion attacks nowhere; a zone it never attacks is refused.
#[tokio::test]
async fn the_scourge_are_sent_to_a_zone_at_once() {
    let realm = harness::realm_with(GameData::empty());
    assert_eq!(
        invasion::attack(&realm, WINTERSPRING),
        Err(AttackRefused::Disabled),
        "off"
    );
    invasion::enable(&realm);
    for _ in 0..3 {
        invasion::necropolis_fell(&realm, WINTERSPRING);
    }
    realm.social.invasion.lock().next_pass = 0;
    invasion::sweep(&realm);
    let rested = {
        let s = realm.social.invasion.lock();
        (s.victories, s.active.contains(&WINTERSPRING))
    };
    assert_eq!(rested, (1, false), "won, Winterspring rests");
    let driven = |realm: &crate::game::Realm| {
        realm
            .social
            .driven_events
            .lock()
            .get(&ATTACKING_WINTERSPRING)
            .copied()
    };
    assert_eq!(driven(&realm), Some(false));
    assert_eq!(invasion::attack(&realm, WINTERSPRING), Ok(()));
    let attacked = {
        let s = realm.social.invasion.lock();
        (
            s.active.contains(&WINTERSPRING),
            s.remaining.get(&WINTERSPRING).copied(),
        )
    };
    assert_eq!(attacked, (true, Some(3)), "attacked again at once");
    assert_eq!(driven(&realm), Some(true), "its event on");
    assert_eq!(
        invasion::attack(&realm, WINTERSPRING),
        Err(AttackRefused::AlreadyAttacked)
    );
    assert_eq!(
        invasion::attack(&realm, 12),
        Err(AttackRefused::UnknownZone),
        "Elwynn"
    );
}

/// Irondeep's and Coldtooth's supplies, Alterac Valley's mines'.
const SUPPLIES: [u32; 6] = [178_784, 178_785, 178_786, 178_787, 178_788, 178_789];

/// An ordinary chest beside them.
const CHEST: u32 = 997_400;

/// Thistle Tea's `ActorFactory` "deny supply #{entry} access without an
/// active match reservation", for each of the six: a mine's supplies with
/// no match on their map are barred to either side and to one of none,
/// and using them is refused there, where they were open to all
/// (`av::supplies_barred`, which bars them in a match to the side not
/// holding their mine); an ordinary chest is left to open as chests do.
#[tokio::test]
async fn a_mines_supplies_are_barred_with_no_match() {
    let mut d = GameData::empty();
    for (i, entry) in SUPPLIES.into_iter().chain([CHEST]).enumerate() {
        d.gameobject_templates
            .insert(entry, object(entry, "Supplies", 3));
        d.gameobject_spawns
            .entry(0)
            .or_default()
            .push(placed(80 + i as u32, entry));
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (barred, chest) = map
        .query(move |m| {
            use crate::game::map::battleground::{self as bg, av};
            let barred = SUPPLIES
                .into_iter()
                .enumerate()
                .map(|(i, entry)| {
                    let o = guid::game_object(entry, 80 + i as u32);
                    assert!(m.gameobject(o).is_some(), "supplies {entry}");
                    let sides = [Some(0), Some(1), None].map(|s| av::supplies_barred(m, s, o));
                    (sides, bg::use_object(m, g, o))
                })
                .collect::<Vec<_>>();
            let chest = guid::game_object(CHEST, 86);
            (
                barred,
                (
                    av::supplies_barred(m, None, chest),
                    bg::use_object(m, g, chest),
                ),
            )
        })
        .await
        .unwrap();
    assert_eq!(barred, vec![([true; 3], true); 6], "barred to all, refused");
    assert_eq!(chest, (false, false), "a chest is a chest");
}
