//! Round 6's entity tests, ported from Thistle Tea's
//! `test/game/world/entity/*_test.exs` (docs/parity/matrix.csv): what a
//! client is told of an object it doesn't know, and a caught bobber's
//! loot outlasting its time.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::spells::Targets;

/// An object the client was never told of is nothing to it: what it does
/// (an emote, sent to its observers) doesn't reach the client, nor does
/// its going (Thistle Tea's "drops source-scoped packets for untracked
/// entities" and "drops destroy packets for untracked entities"; wvs sends
/// to an object's observers only, and forgets to those who knew it).
#[tokio::test]
async fn what_a_client_never_knew_it_is_never_told_of() {
    use crate::game::tests_pets::{WOLF, enter};
    let realm = crate::game::harness::realm_with(crate::game::tests_pets::data());
    let (mut a, ga, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    let far = map
        .query(move |m| {
            let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
            let mut at = m.players[&ga].position();
            at.x += 300.0;
            m.add_creature(Creature::new(guid::unit(WOLF, 0x70_0200), &t, 1, at, 0.0))
        })
        .await
        .unwrap();
    crate::game::harness::an_update(&map).await;
    crate::game::harness::an_update(&map).await;
    a.clear();
    map.query(move |m| {
        if let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE {
            emote: Emote::OneshotRoar,
            guid: far,
        }) {
            m.send_to_observers(far, &frame, false);
        }
        m.remove_creature(far);
    })
    .await;
    crate::game::harness::an_update(&map).await;
    let told = a
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == far => Some("its emote"),
            m if crate::game::harness::forgotten(m).contains(&far) => Some("its going"),
            m if crate::game::harness::creates(m).contains(&far) => Some("it"),
            _ => None,
        })
        .await;
    assert_eq!(told, None);
}

/// A caught fish's bobber outlasts its line's time while its loot is open:
/// the bobber stays in the water and the fish is still taken (Thistle
/// Tea's "caught bobbers survive their cast expiry while loot is open").
#[tokio::test]
async fn a_caught_bobber_outlasts_its_time_while_its_loot_is_open() {
    use crate::game::tests_loot::{FISH, FISHING, alone, learn};
    let (_realm, mut a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let bobber = map
        .query(move |m| {
            learn(m, g, FISHING, 356);
            crate::game::map::spells::cast(m, g, FISHING, Targets::default(), None);
            for b in m.loot_state.bobbers.iter_mut() {
                b.bite_at = m.now;
            }
            m.loot_state.bobbers.first().map(|b| b.guid)
        })
        .await
        .flatten()
        .expect("a bobber in the water");
    a.wait_for("the bite", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_CUSTOM_ANIM(c) if c.guid == bobber => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_GAMEOBJ_USE { guid: bobber }).await;
    a.wait_for("the loot window", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
    })
    .await;
    // Its time, and long past it.
    let still = map
        .query(move |m| {
            m.now += Duration::from_secs(60);
            crate::game::map::professions::respawn_nodes(m);
            m.gameobject(bobber).is_some()
        })
        .await
        .unwrap();
    assert!(still, "the bobber outlasts its time");
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("the fish", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == FISH => Some(()),
        _ => None,
    })
    .await;
}
