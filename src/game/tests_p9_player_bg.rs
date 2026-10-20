//! Round 9's player: battleground supplies, flags and insignia (Thistle
//! Tea's `battleground_supplies_test`, `game_objects_test` and
//! `insignia_test`).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p4_items_av as av;
use super::tests_pvp::{battleground, both_into, marker, put};
use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_quests::{Fleet, FleetPhase, March};
use crate::game::map::battleground::{self as bg, Flag, wsg};
use crate::game::map::inventory::BANK_START;
use crate::game::map::{Map as World, MapHandle};
use crate::game::store::BAG_PLAYER;

const JUNK: u32 = 63_999;

/// Alice of the Alliance and Grom of the Horde in a running valley, its
/// beacons and orders one a player, as 1.12's are.
async fn valley() -> (Arc<Realm>, Client, Guid, Client, Guid, MapHandle) {
    let mut d = av::data();
    for &(_, _, _, _, _, beacon, _) in &av::COMMANDERS {
        d.items.get_mut(&beacon).unwrap().max_count = 1;
    }
    d.items.get_mut(&av::ORDERS).unwrap().max_count = 1;
    d.items.insert(
        JUNK,
        ItemTemplate {
            entry: JUNK,
            name: "Junk".into(),
            stackable: 1,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    put(&map, &mut a, 3.0, 0.0).await;
    put(&map, &mut b, 3.0, 0.0).await;
    (realm, a, alice, b, grom, map)
}

/// `g`'s bags filled to the last slot.
fn fill(m: &mut World, g: Guid) {
    while crate::game::map::inventory::add_item(m, g, JUNK, 1, false).is_ok() {}
}

fn empty(m: &mut World, g: Guid) {
    let d = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.items.retain(|i| i.entry != JUNK);
    p.refresh_inventory(&d);
}

/// One of `entry` in `g`'s bank.
fn bank(m: &mut World, g: Guid, entry: u32) {
    let d = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = m.players.get_mut(&g).unwrap();
    p.items.push(crate::game::map::player::Item::new(
        &d,
        crate::game::guid::item(counter),
        g,
        entry,
        BAG_PLAYER,
        BANK_START,
        1,
    ));
    p.refresh_inventory(&d);
}

fn unbank(m: &mut World, g: Guid, entry: u32) {
    let d = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.items.retain(|i| i.entry != entry);
    p.refresh_inventory(&d);
}

/// Thistle Tea's "full bags and an existing bank beacon do not spend
/// supplies" and "commits one beacon and receipt before a stale selection
/// can take more": a beacon asked for with full bags, or with one in the
/// bank, isn't given, and the fleet keeps its supplies; with room and none,
/// it is, once, the supplies spent.
#[tokio::test]
async fn a_beacon_not_carried_keeps_its_fleets_supplies() {
    let (_realm, _a, _alice, mut b, grom, map) = valley().await;
    let (entry, _, _, goal, _, beacon, _) = av::COMMANDERS[0];
    let commander = av::of(&map, entry).await;
    let i = 0;
    let state = move |m: &World| (av::x(m).air[i].count, m.players[&grom].count_of(beacon));
    map.query(move |m| {
        av::xm(m).air[i] = Fleet {
            phase: FleetPhase::Ready,
            count: goal,
            launched: false,
        };
        fill(m, grom);
    })
    .await;
    assert_eq!(av::menu(&mut b, commander).await.1.len(), 2);
    av::pick(&mut b, commander, 0).await;
    assert_eq!(
        map.query(move |m| state(m)).await,
        Some((goal, 0)),
        "full bags"
    );
    map.query(move |m| {
        empty(m, grom);
        bank(m, grom, beacon);
    })
    .await;
    av::menu(&mut b, commander).await;
    av::pick(&mut b, commander, 0).await;
    assert_eq!(
        map.query(move |m| state(m)).await,
        Some((goal, 1)),
        "one banked"
    );
    map.query(move |m| unbank(m, grom, beacon)).await;
    av::menu(&mut b, commander).await;
    av::pick(&mut b, commander, 0).await;
    assert_eq!(
        map.query(move |m| state(m)).await,
        Some((0, 1)),
        "given, spent"
    );
    av::pick(&mut b, commander, 0).await;
    assert_eq!(map.query(move |m| state(m)).await, Some((0, 1)), "once");
}

/// Thistle Tea's "full bags and an existing bank order leave the shared
/// stockpile ready": the quartermaster's orders asked for with full bags,
/// or with them in the bank, aren't given, and the side's stockpile and
/// its march stay as they were; with room, they are, the stockpile spent.
#[tokio::test]
async fn orders_not_carried_leave_the_stockpile_ready() {
    let (_realm, mut a, alice, _b, _grom, map) = valley().await;
    av::set_standing(&map, alice, 730, 9000).await;
    let quartermaster = av::of(&map, av::QUARTERMASTERS[0]).await;
    let state = move |m: &World| {
        let gr = av::x(m).ground[0];
        (
            gr.irondeep,
            gr.coldtooth,
            gr.phase,
            m.players[&alice].count_of(av::ORDERS),
        )
    };
    map.query(move |m| {
        av::xm(m).ground[0].irondeep = 280;
        av::xm(m).ground[0].coldtooth = 70;
        fill(m, alice);
    })
    .await;
    assert_eq!(av::menu(&mut a, quartermaster).await.1.len(), 3);
    av::pick(&mut a, quartermaster, 1).await;
    assert_eq!(
        map.query(move |m| state(m)).await,
        Some((280, 70, March::Ready, 0)),
        "full bags"
    );
    map.query(move |m| {
        empty(m, alice);
        bank(m, alice, av::ORDERS);
    })
    .await;
    av::menu(&mut a, quartermaster).await;
    av::pick(&mut a, quartermaster, 1).await;
    assert_eq!(
        map.query(move |m| state(m)).await,
        Some((280, 70, March::Ready, 1)),
        "banked"
    );
    map.query(move |m| unbank(m, alice, av::ORDERS)).await;
    av::menu(&mut a, quartermaster).await;
    av::pick(&mut a, quartermaster, 1).await;
    assert_eq!(
        map.query(move |m| state(m)).await,
        Some((0, 0, March::Assembled, 1)),
        "given"
    );
}

/// Thistle Tea's "base and dropped flags require a nearby living player
/// in the same world": the Horde's flag at its stand isn't taken from
/// twenty yards, nor named from another world (an object not in this
/// copy); beside it, it is. (Alive, unmounted and free:
/// `tests_p5_data_carrier::only_one_free_and_not_wholly_protected_may_take_a_flag_up`.)
#[tokio::test]
async fn a_flag_is_taken_only_beside_it_in_its_own_copy() {
    let mut d = GameData::empty();
    battleground(&mut d, 489, 2, "PVPZone03", [769, 770]);
    marker(&mut d, wsg::SPELL_FLAG_CARRIED[1]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::WarsongGulch).await;
    let flag = |m: &World| m.battleground.as_ref().and_then(|b| b.flags[1]);
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !matches!(
        map.query(move |m| flag(m)).await.flatten(),
        Some(Flag::AtBase(_))
    ) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the flag at its stand"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let got = map
        .query(move |m| {
            let Some(Flag::AtBase(go)) = flag(m) else {
                unreachable!()
            };
            let (x, y, z, _) = wsg::FLAG_AT[1];
            let place = |m: &mut World, dx: f32| {
                m.players.get_mut(&alice).unwrap().movement.position = Vector3d { x: x + dx, y, z };
            };
            place(m, 20.0);
            wsg::use_object(m, alice, go);
            let far = flag(m);
            place(m, 0.0);
            let elsewhere = crate::game::guid::game_object(179_831, 999_999);
            wsg::use_object(m, alice, elsewhere);
            let foreign = flag(m);
            wsg::use_object(m, alice, go);
            (far, foreign, flag(m), go)
        })
        .await
        .unwrap();
    let base = Some(Flag::AtBase(got.3));
    assert_eq!((got.0, got.1), (base, base), "from afar, or not here: no");
    assert_eq!(got.2, Some(Flag::Carried(alice)), "beside it");
}

/// Thistle Tea's "rejects changed death identities, teammates, distant
/// looters, and another copy": a fallen enemy's insignia isn't taken
/// from twenty yards, nor by himself; beside his body, it is, and his
/// corpse is bones; that death's insignia taken, nothing more is (another
/// copy's corpse is another map's, never this one's to name).
#[tokio::test]
async fn an_insignia_is_taken_beside_an_enemys_body_once() {
    let realm = harness::realm_with(super::tests_pvp2::valley());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    let got = map
        .query(move |m| {
            let at = m.players[&alice].position();
            m.players.get_mut(&grom).unwrap().movement.position = Vector3d {
                x: at.x + 20.0,
                ..at
            };
            bg::died(m, grom, alice);
            crate::game::map::death::kill(m, grom);
            let corpse = |m: &World| {
                m.corpses
                    .values()
                    .find(|c| c.owner == grom)
                    .map(|c| c.fields.get_u32(crate::game::fields::CORPSE_FIELD_FLAGS))
            };
            bg::insignia::remove(m, alice, Some(grom));
            let far = (m.players[&alice].looting, corpse(m));
            bg::insignia::remove(m, grom, Some(grom));
            let own = corpse(m);
            m.players.get_mut(&alice).unwrap().movement.position = Vector3d {
                x: at.x + 19.0,
                ..at
            };
            bg::insignia::remove(m, alice, Some(grom));
            let taken = (m.players[&alice].looting.is_some(), corpse(m));
            crate::game::map::loot::close(m, alice, m.players[&alice].looting.unwrap());
            bg::insignia::remove(m, alice, Some(grom));
            (far, own, taken, m.players[&alice].looting)
        })
        .await
        .unwrap();
    assert_eq!(got.0.0, None, "from twenty yards: no window");
    let insignia = bg::insignia::CORPSE_INSIGNIA;
    assert!(
        got.0.1.is_none_or(|f| f & insignia != 0),
        "his body untouched"
    );
    assert!(got.1.is_none_or(|f| f & insignia != 0), "not by himself");
    assert!(got.2.0, "beside him: taken");
    assert!(
        got.2
            .1
            .is_some_and(|f| f & crate::game::map::bones::CORPSE_FLAG_BONES != 0),
        "bones"
    );
    assert_eq!(got.3, None, "once");
}
