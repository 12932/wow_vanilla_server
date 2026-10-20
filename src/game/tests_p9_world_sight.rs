//! Round 9's world tests of who notices whom, ported from Thistle Tea's
//! `test/game/world/proximity_test.exs`: a stealthed trap hidden from its
//! owner's opponent for a duel.

use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::duel::{self, Phase};
use crate::game::tests_p3_combat::{self as base, START};
use crate::game::tests_p3_entity::on;

/// A stealthed trap.
const TRAP: u32 = 997_403;

fn data() -> GameData {
    let mut d = base::data();
    let mut trap = [0u32; 24];
    trap[9] = 1;
    d.gameobject_templates.insert(
        TRAP,
        crate::game::data::GameObjectTemplate {
            entry: TRAP,
            kind: 6,
            display_id: 1000,
            name: "Trap".into(),
            size: 1.0,
            data: trap,
            ..Default::default()
        },
    );
    d
}

/// `yards` east of the start.
fn east(yards: f32) -> Vector3d {
    Vector3d {
        x: START.0 + yards,
        y: START.1,
        z: START.2,
    }
}

/// Thistle Tea's `sync/2` "a reaction change tells the traps the unit
/// owns": Alice's stealthed trap, which Bob of her group sees, is hidden
/// from him as their duel's fight begins, and shown again as it ends
/// (mangos's `GameObject::isVisibleForInState`, a player's stealthed trap
/// unseen by one in a duel with its owner; wvs works out the duelists'
/// visibility again at both).
#[tokio::test]
async fn a_duel_hides_the_owners_stealthed_trap_from_the_other_side() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 3.0, START.1, START.2).await;
    let (la, lb) = (
        realm.registry.online(ga).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &la, &lb));
    let map = realm.maps.get(&realm, MapKey::world(0));
    let trap = on(&map, move |m| {
        crate::game::map::objects::summon(m, Some(ga), TRAP, east(1.0), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    b.sees(trap).await;
    // A few updates: what the group's joining has Bob look at again is
    // looked at before the duel.
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    on(&map, move |m| {
        duel::challenge(m, ga, gb);
        duel::accept(m, gb);
        let now = m.now;
        for g in [ga, gb] {
            if let Some(d) = m.players.get_mut(&g).and_then(|p| p.duel.as_mut()) {
                d.phase = Phase::Countdown { until: now };
            }
        }
    })
    .await;
    b.forgets(trap).await;
    assert!(on(&map, move |m| duel::dueling(m, ga, gb)).await);
    on(&map, move |m| duel::cancel(m, gb)).await;
    b.sees(trap).await;
}
