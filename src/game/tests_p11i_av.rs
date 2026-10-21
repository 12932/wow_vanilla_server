//! Round 11's test-only rows on Alterac Valley's supplies (Thistle Tea's
//! `battleground_supplies_test.exs`): requests for a beacon or for the
//! assault orders sent together, which the valley's map handles one at a
//! time (an actor), so the first takes the stockpile and the rest find it
//! spent.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p4_items_av::{COMMANDERS, ORDERS, QUARTERMASTERS, data, menu, of, set_standing, x, xm};
use super::tests_pvp::{both_into, put};
use super::tests_pvp2::choose;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::av_quests::{Fleet, FleetPhase, March};
use crate::game::map::{Map as World, MapHandle};

const MARSHAL: u32 = 13446;

async fn setup() -> (Client, Guid, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    put(&map, &mut a, 3.0, 0.0).await;
    put(&map, &mut b, 3.0, 0.0).await;
    (a, alice, b, grom, map)
}

fn count_of(m: &World, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

/// `n` choices of `option` at `npc`, sent together, and the menu's
/// closings they bring.
async fn together(c: &mut Client, npc: Guid, option: u32, n: usize) {
    c.clear();
    for _ in 0..n {
        choose(c, npc, option).await;
    }
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(c).await;
}

/// Thistle Tea's "parallel requests serialize against the shared
/// stockpile": eight requests for Guse's beacon sent together, his fleet
/// funded for one, give one beacon and leave the stockpile spent
/// (`av_quests::gossip_select`'s `TakeBeacon`, asked again at each).
#[tokio::test]
async fn beacon_requests_sent_together_give_one_beacon() {
    let (_a, _alice, mut b, grom, map) = setup().await;
    let (i, &(entry, _, _, goal, _, beacon, _)) = COMMANDERS
        .iter()
        .enumerate()
        .find(|(_, c)| c.0 == 13179)
        .unwrap();
    let guse = of(&map, entry).await;
    map.query(move |m| {
        xm(m).air[i] = Fleet {
            phase: FleetPhase::Ready,
            count: goal,
            launched: false,
        };
    })
    .await;
    assert_eq!(menu(&mut b, guse).await.1.len(), 2, "the beacon and the attack");
    together(&mut b, guse, 0, 8).await;
    let (count, beacons) = map
        .query(move |m| (x(m).air[i].count, count_of(m, grom, beacon)))
        .await
        .unwrap();
    assert_eq!((count, beacons), (0, 1), "one beacon, the stockpile spent");
}

/// Thistle Tea's "commits one order and receipt and deploys only once"
/// and "racing requesters cannot duplicate an infantry deployment": eight
/// requests for the assault orders sent together, the Irondeep supplies
/// funding one, give one order and assemble one field marshal; the
/// supplies spent, the ground assault assembled.
#[tokio::test]
async fn orders_requests_sent_together_give_one_order_and_one_marshal() {
    let (mut a, alice, _b, _grom, map) = setup().await;
    set_standing(&map, alice, 730, 9000).await;
    let quartermaster = of(&map, QUARTERMASTERS[0]).await;
    map.query(|m| {
        xm(m).ground[0].irondeep = 280;
    })
    .await;
    assert_eq!(menu(&mut a, quartermaster).await.1.len(), 3, "status, orders, goods");
    together(&mut a, quartermaster, 1, 8).await;
    let marshals = |m: &World| {
        m.creatures
            .iter()
            .filter(|(_, c)| c.entry == MARSHAL)
            .count()
    };
    let out = map
        .query(move |m| {
            let gr = x(m).ground[0];
            (count_of(m, alice, ORDERS), marshals(m), gr.phase, gr.irondeep)
        })
        .await
        .unwrap();
    assert_eq!(out, (1, 1, March::Assembled, 0), "one order, one marshal");
}

/// Thistle Tea's "admits enemy bodies and rejects stale, hidden, distant,
/// and foreign-copy snapshots": Remove Insignia takes an enemy's body
/// (`battleground::insignia::remove`), and does nothing, silently (as
/// mangos's `EffectSkinPlayerCorpse`), at one's own body, from more than
/// ten yards, cast by the dead, at a body no longer the enemy's to take
/// (its skinnable flag gone), or at bones already taken; nor is he made to
/// release by any of those. Another copy's body is another map's.
#[tokio::test]
async fn remove_insignia_takes_an_enemys_body_near_and_refuses_the_rest() {
    use crate::game::fields::*;
    use crate::game::map::battleground::{self as bg, insignia};
    use crate::game::map::death::{self, Death};
    const REMOVE_INSIGNIA: u32 = 22_027;
    let mut d = super::tests_pvp2::valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: REMOVE_INSIGNIA,
        name: "Remove Insignia".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = insignia::EFFECT_SKIN_PLAYER_CORPSE;
    d.spells.spells.insert(REMOVE_INSIGNIA, s);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::AlteracValley).await;
    put(&map, &mut a, 0.0, 0.0).await;
    put(&map, &mut b, 12.0, 0.0).await;
    map.query(move |m| {
        bg::died(m, grom, alice);
        death::kill(m, grom);
    })
    .await;
    let remove = |target: Guid| {
        let map = map.clone();
        async move {
            map.query(move |m| {
                crate::game::map::spells::cast_triggered(m, alice, REMOVE_INSIGNIA, Some(target));
                matches!(m.players[&grom].death, Death::Dead { .. })
            })
            .await
            .unwrap()
        }
    };
    a.clear();
    // From twelve yards: nothing.
    assert!(remove(grom).await, "not made to release from afar");
    // At her own body, or cast while dead: nothing.
    put(&map, &mut a, 11.0, 0.0).await;
    a.clear();
    assert!(remove(alice).await, "her own");
    map.query(move |m| death::kill(m, alice)).await;
    assert!(remove(grom).await, "cast by the dead");
    map.query(move |m| {
        death::resurrect(m, alice, 1.0);
    })
    .await;
    // His flag gone (he has risen and fallen out of it): nothing.
    map.query(move |m| {
        m.players
            .get_mut(&grom)
            .unwrap()
            .fields
            .set_flag(UNIT_FIELD_FLAGS, insignia::UNIT_FLAG_SKINNABLE, false);
    })
    .await;
    assert!(remove(grom).await, "no body to take");
    assert_eq!(looted(&mut a).await, None, "none of those opened his body");
    // His body, near: taken, he made to release, its bones to loot.
    map.query(move |m| {
        m.players
            .get_mut(&grom)
            .unwrap()
            .fields
            .set_flag(UNIT_FIELD_FLAGS, insignia::UNIT_FLAG_SKINNABLE, true);
    })
    .await;
    assert!(!remove(grom).await, "taken: he is made to release");
    assert_eq!(looted(&mut a).await, Some(()), "his bones' loot");
    a.send(CMSG_LOOT_RELEASE {
        guid: Guid::zero(),
    })
    .await;
    // The bones already taken: nothing again.
    let bones = map
        .query(|m| m.corpses.values().map(|c| c.guid).next())
        .await
        .unwrap()
        .expect("the bones");
    a.clear();
    remove(bones).await;
    remove(grom).await;
    assert_eq!(looted(&mut a).await, None, "bones are taken once");
}

/// Whether a loot window opens for `c` soon.
async fn looted(c: &mut Client) -> Option<()> {
    c.gets(std::time::Duration::from_millis(300), |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
    })
    .await
}
