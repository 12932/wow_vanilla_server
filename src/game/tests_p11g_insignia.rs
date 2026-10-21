//! Round 11, against Thistle Tea's `insignia_test.exs`: insignia taken
//! from a victim already released (told they weren't released by it, with
//! no corpse to take back, and the bones outliving their next corpse), and
//! from the corpse of one gone from the world (mangos-classic's
//! `RemovedInsignia` and `EffectSkinPlayerCorpse`), the bones lying their
//! hour and no longer.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::both_into;
use super::tests_pvp2::valley;
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::battleground::{self as bg};
use crate::game::map::death;

const REMOVE_INSIGNIA: u32 = 22027;
const SMSG_LOOT_RESPONSE: u16 = 0x160;

fn data() -> GameData {
    let mut d = valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: REMOVE_INSIGNIA,
        name: "Remove Insignia".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = bg::insignia::EFFECT_SKIN_PLAYER_CORPSE;
    d.spells.spells.insert(REMOVE_INSIGNIA, s);
    d
}

/// Alice of the Alliance and Grom of the Horde, in Alterac Valley.
async fn pair(
    realm: &std::sync::Arc<crate::game::Realm>,
    accounts: [&str; 2],
) -> (Client, Guid, Client, Guid, MapHandle) {
    let mut a = Client::connect(realm, accounts[0]);
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, accounts[1]);
    let gb = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(realm, &mut a, &mut b, Map::AlteracValley).await;
    (a, ga, b, gb, map)
}

async fn put(map: &MapHandle, c: &mut Client, at: Vector3d) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, at, 0.0)).await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

/// Grom falls, at 60, and releases: his corpse, and where it lies.
async fn fallen_and_released(map: &MapHandle, alice: Guid, grom: Guid) -> (Guid, Vector3d) {
    map.query(move |m| {
        m.players
            .get_mut(&grom)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 60);
        m.realm.registry.set_level(grom, 60);
        bg::died(m, grom, alice);
        death::kill(m, grom);
        death::release(m, grom);
        m.corpses
            .values()
            .find(|c| c.owner == grom)
            .map(|c| (c.guid, c.position))
    })
    .await
    .flatten()
    .expect("his corpse")
}

/// The bones of `owner` here.
fn bones_of(m: &crate::game::map::Map, owner: Guid) -> Vec<Guid> {
    m.corpses
        .values()
        .filter(|c| {
            c.owner == owner
                && c.fields.get_u32(CORPSE_FIELD_FLAGS) & crate::game::map::bones::CORPSE_FLAG_BONES
                    != 0
        })
        .map(|c| c.guid)
        .collect()
}

/// Grom, released before Alice takes his insignia, is told it didn't
/// release him (`SMSG_PLAYER_SKINNED`'s 0, mangos's `repop`), has no
/// corpse to take back, and his bones lie on through his next death's
/// corpse (Thistle Tea's "released victims lose recovery while bones
/// survive their next corpse").
#[tokio::test]
async fn a_released_victim_is_told_so_and_the_bones_outlive_his_next_corpse() {
    let realm = harness::realm_with(data());
    let (mut a, alice, mut b, grom, map) = pair(&realm, ["P11GINS1A", "P11GINS1B"]).await;
    let (corpse, at) = fallen_and_released(&map, alice, grom).await;
    put(
        &map,
        &mut a,
        Vector3d {
            x: at.x + 1.0,
            ..at
        },
    )
    .await;
    a.clear();
    b.clear();
    map.query(move |m| {
        crate::game::map::spells::cast_triggered(m, alice, REMOVE_INSIGNIA, Some(corpse))
    })
    .await;
    a.wait_frame("SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |_| true)
        .await;
    let released = b
        .wait_for("SMSG_PLAYER_SKINNED", |m| match m {
            ServerOpcodeMessage::SMSG_PLAYER_SKINNED(s) => Some(s.spirit_released),
            _ => None,
        })
        .await;
    assert!(!released, "released before, not by this");
    let out = map
        .query(move |m| {
            let kept = m.players[&grom].row.state.corpse.is_some();
            let bones = bones_of(m, grom);
            // Up again, down again and released: a new corpse.
            death::resurrect(m, grom, 1.0);
            bg::died(m, grom, alice);
            death::kill(m, grom);
            death::release(m, grom);
            let new_corpse = m.corpses.contains_key(&corpse);
            (kept, bones.clone(), new_corpse, bones_of(m, grom) == bones)
        })
        .await
        .unwrap();
    assert!(!out.0, "no corpse to take back");
    assert_eq!(out.1.len(), 1, "his bones");
    assert!(out.2, "his next corpse");
    assert!(out.3, "the bones lie on beside it");
}

/// Grom, released, leaves the world; his corpse's insignia are still
/// Alice's to take: bones she finds his level's gold on, lying their hour
/// and gone after it (mangos's `EffectSkinPlayerCorpse` with no owner
/// found; Thistle Tea's "offline owners remain removable and bones expire
/// without restarting").
#[tokio::test]
async fn an_absent_owners_corpse_gives_its_insignia_and_the_bones_lie_their_hour() {
    let realm = harness::realm_with(data());
    let (mut a, alice, b, grom, map) = pair(&realm, ["P11GINS2A", "P11GINS2B"]).await;
    let (corpse, at) = fallen_and_released(&map, alice, grom).await;
    b.disconnect();
    harness::offline(&realm, grom).await;
    put(
        &map,
        &mut a,
        Vector3d {
            x: at.x + 1.0,
            ..at
        },
    )
    .await;
    a.clear();
    let before = map
        .query(move |m| (m.corpses.contains_key(&corpse), m.players[&alice].money()))
        .await
        .unwrap();
    assert!(before.0, "his corpse lies on");
    map.query(move |m| {
        crate::game::map::spells::cast_triggered(m, alice, REMOVE_INSIGNIA, Some(corpse))
    })
    .await;
    a.wait_frame("SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |_| true)
        .await;
    let bones = map.query(move |m| bones_of(m, grom)).await.unwrap();
    assert_eq!(bones.len(), 1, "bones in its place");
    a.send(CMSG_LOOT_MONEY {}).await;
    let mut money = before.1;
    for _ in 0..200 {
        money = map.query(move |m| m.players[&alice].money()).await.unwrap();
        if money > before.1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        money - before.1 >= bg::insignia::gold(60, 50),
        "a level 60's gold: {}",
        money - before.1
    );
    let lying = map
        .query(move |m| {
            m.now += bg::insignia::BONES_LIE - Duration::from_secs(60);
            crate::game::map::bones::update(m);
            let still = m.corpses.contains_key(&bones[0]);
            m.now += Duration::from_secs(61);
            crate::game::map::bones::update(m);
            (still, m.corpses.contains_key(&bones[0]))
        })
        .await
        .unwrap();
    assert_eq!(lying, (true, false), "an hour, and no longer");
}
