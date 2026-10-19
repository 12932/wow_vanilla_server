//! Round 6's player tests on the real world, ported from Thistle Tea's
//! `test/game/world/entity/player/` (docs/parity/matrix.csv): a vendor of
//! the seed data, and Fishing's pole.

use std::sync::LazyLock;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{MapKey, spells};
use crate::game::tests_p3_entity::on;

static DB: LazyLock<std::path::PathBuf> = LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

/// Thistle Tea's `Vendor.valid_vendor?` "accepts Corina Steele's vanilla
/// vendor and repair flags from seed data": Goldshire's weaponsmith (54) is
/// a vendor and a repairer (0x4004), and shows her wares.
#[tokio::test]
#[ignore = "extended"]
async fn corina_steele_sells_and_repairs() {
    const CORINA_STEELE: u32 = 54;
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let flags = realm
        .data
        .creature_template(CORINA_STEELE)
        .map(|t| t.npc_flags);
    assert_eq!(flags, Some(0x4004));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let corina = on(&map, move |m| {
        crate::game::tests_p3_combat::spawn(m, g, CORINA_STEELE, 10, 2.0)
    })
    .await;
    a.sees(corina).await;
    a.send(CMSG_LIST_INVENTORY { guid: corina }).await;
    let wares = a
        .wait_for("SMSG_LIST_INVENTORY", |m| match m {
            ServerOpcodeMessage::SMSG_LIST_INVENTORY(l) => Some(l.items.len()),
            _ => None,
        })
        .await;
    assert!(wares > 0, "her wares");
}

/// Whether `a` is told it may hold fishing poles (weapon subclass 20).
async fn told_poles(a: &mut Client) {
    a.wait_frame("SMSG_SET_PROFICIENCY of weapons", 0x127, |f| {
        f[0] == 2 && u32::from_le_bytes(f[1..5].try_into().unwrap()) & (1 << 20) != 0
    })
    .await;
}

/// Thistle Tea's `Spells.send_proficiencies` "advertises fishing-pole
/// proficiency when Fishing is known": Fishing learned teaches Fishing
/// Poles (7738), whose proficiency the client is told; and told again at
/// a login.
#[tokio::test]
#[ignore = "extended"]
async fn fishing_learned_tells_the_client_it_may_hold_a_pole() {
    const FISHING: u32 = 7620;
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Angler", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.clear();
    let poles = on(&map, move |m| {
        spells::learn(m, g, FISHING);
        m.players[&g].spells.contains(&7738)
    })
    .await;
    assert!(poles, "Fishing Poles learned with Fishing");
    told_poles(&mut a).await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(g).await;
    told_poles(&mut a).await;
}
