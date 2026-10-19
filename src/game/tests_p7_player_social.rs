//! Round 7's player tests of standing, ported from Thistle Tea's
//! `test/game/world/entity/player/reputation_test.exs` and `honor_test.exs`
//! (docs/parity/matrix.csv): a standing sent at login and the reaction it
//! makes, forced or not.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::misc::{RANK_HATED, RANK_NEUTRAL};
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::reputation::{modify, reaction_rank};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player::{FORCE, GUILD, MERCHANT, MERCHANT_A, with_factions};

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Thistle Tea's `Reputation.send_initial` "sends all 64 client slots
/// using standing offsets" and `cached_projection` "reuses the projection
/// until standings or forced reactions change": a standing gained is the
/// next login's, in its slot of `SMSG_INITIALIZE_FACTIONS`; and how the
/// faction's merchant regards the player follows the standing as it
/// changes, and an aura forcing a reaction over it, and the standing again
/// when the aura goes.
#[tokio::test]
async fn a_standing_is_sent_at_login_and_its_reaction_follows_it_and_a_forced_one() {
    let mut d = with_factions(crate::game::tests_p3_player::data());
    d.faction_templates
        .insert(1, crate::game::tests_p3_combat::faction(1, 0x1, 0x2));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let merchant = guid::unit(MERCHANT, MERCHANT_A);
    a.sees(merchant).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| modify(m, g, GUILD, 250, false)).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(g).await;
    let init = a
        .wait_frame("SMSG_INITIALIZE_FACTIONS", 0x122, |_| true)
        .await;
    assert_eq!(u32_at(&init, 0), 64, "every slot");
    // The faction's slot, 5: shown, at 250.
    let at = 4 + 5 * 5;
    assert_eq!(init[at] & 0x1, 0x1, "shown");
    assert_eq!(u32_at(&init, at + 1), 250, "its standing");
    a.sees(merchant).await;
    let reactions = on(&map, move |m| {
        let liked = reaction_rank(m, merchant, g);
        modify(m, g, GUILD, -42_250, false);
        let hated = reaction_rank(m, merchant, g);
        // Forced Neutral.
        let s = m.realm.data.spells.get(FORCE).unwrap().clone();
        crate::game::map::auras::apply_spell(m, g, g, &s, &[(0, 3)], Default::default()).unwrap();
        let forced = reaction_rank(m, merchant, g);
        crate::game::map::auras::remove_spell(m, g, FORCE, None);
        let again = reaction_rank(m, merchant, g);
        (liked, hated, forced, again)
    })
    .await;
    assert_eq!(
        reactions,
        (
            Some(RANK_NEUTRAL),
            Some(RANK_HATED),
            Some(RANK_NEUTRAL),
            Some(RANK_HATED)
        ),
        "250 is Neutral; then Hated; forced Neutral; Hated again"
    );
}

/// A civilian, worth no experience.
const PEASANT: u32 = 64_400;

/// The next `SMSG_PVP_CREDIT`'s honor, signed.
async fn credit(c: &mut Client) -> i32 {
    let b = c
        .wait_frame(
            "SMSG_PVP_CREDIT",
            crate::game::map::pvp::SMSG_PVP_CREDIT,
            |_| true,
        )
        .await;
    i32::from_le_bytes(b[0..4].try_into().unwrap())
}

/// Thistle Tea's combat credit "restores totals on reconnect", and its
/// honor messages "encode positive credit and signed dishonorable
/// penalties": an honorable kill is credited with its honor, a
/// civilian's death with a negative amount (`SMSG_PVP_CREDIT`'s signed
/// honor); and today's and the lifetime's kills, both kinds, are as they
/// were after a logout and a login.
#[tokio::test]
async fn kills_are_credited_signed_and_their_totals_last_a_relog() {
    use crate::game::fields::{
        PLAYER_FIELD_LIFETIME_DISHONORABLE_KILLS, PLAYER_FIELD_LIFETIME_HONORABLE_KILLS,
        PLAYER_FIELD_SESSION_KILLS,
    };
    let mut d = crate::game::tests_p3_combat::data();
    let mut peasant = crate::game::tests_p3_combat::template(PEASANT, 7);
    peasant.civilian = true;
    peasant.type_flags |= 0x2;
    d.creature_templates.insert(PEASANT, peasant);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    // Grom is Forsaken: his start is on Alice's continent.
    let mut b = Client::connect(&realm, "B");
    let grom = b.enter("Grom", Race::Undead, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    on(&map, move |m| {
        crate::game::map::combat::kill(m, alice, grom)
    })
    .await;
    assert!(credit(&mut a).await > 0, "an honorable kill");
    on(&map, move |m| {
        let p = crate::game::tests_p3_combat::spawn(m, alice, PEASANT, 1, 3.0);
        crate::game::map::combat::kill(m, alice, p);
    })
    .await;
    let penalty = credit(&mut a).await;
    assert!(penalty < 0, "dishonorable, signed: {penalty}");
    let totals = |map: crate::game::map::MapHandle| async move {
        on(&map, move |m| {
            let f = &m.players[&alice].fields;
            let session = f.get_u32(PLAYER_FIELD_SESSION_KILLS);
            (
                session & 0xFFFF,
                session >> 16,
                f.get_u32(PLAYER_FIELD_LIFETIME_HONORABLE_KILLS),
                f.get_u32(PLAYER_FIELD_LIFETIME_DISHONORABLE_KILLS),
            )
        })
        .await
    };
    assert_eq!(totals(map.clone()).await, (1, 1, 1, 1));
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(alice).await;
    a.sees(alice).await;
    assert_eq!(totals(map).await, (1, 1, 1, 1), "after the relog");
}
