//! Round 7, against Thistle Tea's `test/game/core/death_test.exs`
//! `leave_battleground/2` (docs/parity/matrix.csv): what a living player
//! takes home from a battleground.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::{UNIT_FIELD_HEALTH, UNIT_FIELD_MAXHEALTH};
use crate::game::harness::{self, Client};
use crate::game::map::spells::redemption;
use crate::game::tests_pvp::{both_into, marker};

/// Thistle Tea's `leave_battleground/2` "cancels the spirit form and its
/// delayed suicide while retaining the talent" and "does not refill a
/// living participant's health or resources": a priest who leaves a
/// battleground a spirit of redemption goes home a living priest, the
/// spirit ended and no death to come of it, the talent kept; a warrior who
/// leaves wounded goes home as wounded (mangos's `RemovePlayerAtLeave`
/// resurrects only the dead).
#[tokio::test]
async fn a_spirit_leaving_a_battleground_goes_home_alive_and_the_wounded_as_they_are() {
    let mut d = crate::game::tests_bg5::gulch();
    for id in std::iter::once(redemption::FORM).chain(redemption::STATE) {
        marker(&mut d, id);
    }
    marker(&mut d, crate::game::map::battleground::SPELL_DESERTER);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Priest).await;
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::WarsongGulch).await;
    // Alice, with the talent, takes a killing blow: a spirit. Grom is
    // wounded.
    let out = map
        .query(move |m| {
            m.players
                .get_mut(&alice)
                .unwrap()
                .spells
                .push(redemption::TALENT);
            let health = m.players[&alice].fields.get_u32(UNIT_FIELD_HEALTH);
            crate::game::map::combat::deal_damage(m, grom, alice, health);
            let f = &mut m.players.get_mut(&grom).unwrap().fields;
            let half = f.get_u32(UNIT_FIELD_MAXHEALTH) / 2;
            f.set_u32(UNIT_FIELD_HEALTH, half);
            (redemption::spirit(m, alice), half)
        })
        .await
        .unwrap();
    let (spirit, half) = out;
    assert!(spirit, "a spirit of redemption");
    for (c, map_id) in [(&mut a, Map::WarsongGulch), (&mut b, Map::WarsongGulch)] {
        c.send(CMSG_LEAVE_BATTLEFIELD { map: map_id }).await;
        c.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    }
    a.sees(alice).await;
    b.sees(grom).await;
    let home = realm.registry.online(alice).unwrap().map();
    let priest = home
        .query(move |m| {
            let p = &m.players[&alice];
            (
                p.death == crate::game::map::death::Death::Alive,
                redemption::spirit(m, alice),
                redemption::STATE.map(|s| p.auras.has_spell(s)),
                p.spells.contains(&redemption::TALENT),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        priest,
        (true, false, [false, false], true),
        "home alive, the spirit and its end gone, the talent kept"
    );
    let home = realm.registry.online(grom).unwrap().map();
    let wounded = home
        .query(move |m| m.players[&grom].fields.get_u32(UNIT_FIELD_HEALTH))
        .await
        .unwrap();
    assert_eq!(wounded, half, "home as wounded as he left");
}
