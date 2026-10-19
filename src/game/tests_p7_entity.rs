//! Round 7's entity tests, ported from Thistle Tea's
//! `test/game/world/entity/*_test.exs` (docs/parity/matrix.csv): the
//! forced movements' order counter, and roots reaching the client that
//! moves the unit.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_ROOT, AURA_WATER_WALK};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, aura_effect, spell};

const ROOT: u32 = 97_700;
const WATER_WALK: u32 = 97_701;

fn world() -> GameData {
    let mut d = data();
    d.spells
        .spells
        .insert(ROOT, spell(ROOT, &[aura_effect(AURA_MOD_ROOT, 1, 0, 6)]));
    d.spells.spells.insert(
        WATER_WALK,
        spell(WATER_WALK, &[aura_effect(AURA_WATER_WALK, 1, 0, 1)]),
    );
    d
}

/// The order counter in a forced movement (`SMSG_FORCE_MOVE_ROOT` and
/// `_UNROOT`, `SMSG_MOVE_WATER_WALK`, `MSG_MOVE_TELEPORT_ACK`): the
/// client's own, one more for each order it is sent, as mangos-classic's
/// session numbers them (`GetOrderCounter`, `IncrementOrderCounter`) and
/// Thistle Tea's `movement_counter` stamps them at the send ("sequences
/// acknowledged movement packets at the send boundary").
#[tokio::test]
async fn forced_movements_carry_the_clients_next_order_counter() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let data = map
        .query(|m| {
            let s = |id| m.realm.data.spells.get(id).cloned().unwrap();
            (s(ROOT), s(WATER_WALK))
        })
        .await
        .unwrap();
    let (root, walk) = data;
    a.clear();
    map.query(move |m| {
        auras::apply(m, ga, ga, &root, 0, 0);
        auras::remove_spell(m, ga, ROOT, None);
        auras::apply(m, ga, ga, &walk, 0, 0);
        let at = m.players[&ga].position();
        crate::game::map::movement::teleport_within(m, ga, at, 1.0);
    })
    .await;
    let mut counters = Vec::new();
    for _ in 0..4 {
        let c = a
            .wait_for("a forced movement", |m| match m {
                ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if r.guid == ga => {
                    Some(("root", r.counter))
                }
                ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) if r.guid == ga => {
                    Some(("unroot", r.counter))
                }
                ServerOpcodeMessage::SMSG_MOVE_WATER_WALK(r) if r.guid == ga => {
                    Some(("water walk", r.counter))
                }
                ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(r) if r.guid == ga => {
                    Some(("teleport", r.movement_counter))
                }
                _ => None,
            })
            .await;
        counters.push(c);
    }
    let names: Vec<&str> = counters.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["root", "unroot", "water walk", "teleport"]);
    let first = counters[0].1;
    let numbers: Vec<u32> = counters.iter().map(|(_, c)| *c).collect();
    assert_eq!(
        numbers,
        [first, first + 1, first + 2, first + 3],
        "one more each"
    );
}

/// A root reaches the client that moves the unit: a player's own, the
/// possessor's for a creature a player's client moves (its GUID the
/// creature's, the counter the possessor's); a creature no client moves,
/// charmed or not, sends no forced movement at all (Thistle Tea's
/// "root changes reach only the possessed creature's controller" and
/// "ordinary and charmed creatures do not send forced movement packets";
/// mangos-classic's `SetImmobilizedState`, to `GetClientControlling`).
#[tokio::test]
async fn a_root_goes_to_the_client_that_moves_the_unit() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let mut b = harness_client(&realm, "B", "Bob").await;
    let gb = b.guid;
    let (held, free) = map
        .query(move |m| {
            let held = spawn(m, ga, BRUTE, 10, 5.0);
            let free = spawn(m, ga, BRUTE, 10, 8.0);
            (held, free)
        })
        .await
        .unwrap();
    a.clear();
    b.clear();
    let root = map
        .query(|m| m.realm.data.spells.get(ROOT).cloned().unwrap())
        .await
        .unwrap();
    let again = root.clone();
    map.query(move |m| {
        // Bob's client moves the one (Mind Control's charm and mover).
        m.creature_mut(held)
            .unwrap()
            .fields
            .set_guid(UNIT_FIELD_CHARMEDBY, gb);
        m.players.get_mut(&gb).unwrap().owned.mover = Some(held);
        for g in [free, held] {
            auras::apply(m, ga, g, &root, 0, 0);
        }
        auras::remove_spell(m, held, ROOT, None);
    })
    .await;
    let forced = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) => Some((true, r.guid, r.counter)),
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) => Some((false, r.guid, r.counter)),
        _ => None,
    };
    let first = b.wait_for("the possessed one's root", forced).await;
    let second = b.wait_for("and its unroot", forced).await;
    assert_eq!((first.0, first.1), (true, held));
    assert_eq!((second.0, second.1), (false, held));
    assert_eq!(second.2, first.2 + 1, "Bob's counter");
    assert!(
        b.gets(NONE_WITHIN, |m| forced(m).filter(|f| f.1 == free))
            .await
            .is_none(),
        "nothing for the creature no client moves"
    );
    assert!(
        a.gets(NONE_WITHIN, forced).await.is_none(),
        "nor to anyone else"
    );
    // Charmed but not moved by its charmer's client: none either.
    map.query(move |m| {
        m.players.get_mut(&gb).unwrap().owned.mover = None;
        auras::apply(m, ga, held, &again, 0, 0);
    })
    .await;
    assert!(
        b.gets(Duration::from_millis(300), forced).await.is_none(),
        "a charmed creature its client doesn't move"
    );
    let _ = START;
}

async fn harness_client(
    realm: &std::sync::Arc<crate::game::Realm>,
    account: &str,
    name: &str,
) -> crate::game::harness::Client {
    let mut c = crate::game::harness::Client::connect(realm, account);
    c.enter(name, Race::Human, Class::Warrior).await;
    c.heartbeat(START.0, START.1, START.2).await;
    c
}
