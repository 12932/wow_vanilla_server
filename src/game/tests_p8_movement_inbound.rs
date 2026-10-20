//! Packets in, round 8: a pet's commands from its master's bar, and the
//! layouts of what the server answers with.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_pets::{SUMMON_IMP, WOLF, cast, data, enter, learn, pet_bar, place, spell};

/// A ground spell a possessed creature may cast: fire on all enemies at a
/// place, three seconds to cast.
const RAIN: u32 = 900_801;
const THREE_SECONDS: u32 = 14;

/// Wait until `f` holds on `map`, up to two seconds.
async fn until(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&Map) -> bool + Send + Sync + Copy + 'static,
) {
    for _ in 0..200 {
        if map.query(move |m| f(m)).await.unwrap() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

/// What `c` sent is in: a line said, heard back.
async fn heard(c: &mut Client, line: &str) {
    c.say(line).await;
    let line = line.to_owned();
    c.wait_for("the line back", move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => (s.message == line).then_some(()),
        _ => None,
    })
    .await;
}

fn attacking(m: &Map, g: Guid) -> bool {
    m.combat(g).is_some_and(|c| c.attacking)
}

/// Thistle Tea's pet_messages_test, `CMSG_PET_STOP_ATTACK` and
/// `CMSG_PET_CAST_SPELL`: a stop from the master's bar stops their own pet
/// (not one a stranger names) and a player they possess; and a possessed
/// creature's ground spell is cast at the place the bar named (before, the
/// place was dropped, and only a unit kept).
#[tokio::test]
async fn a_pets_stop_and_a_possessed_casters_ground_spell_go_as_its_master_says() {
    let mut d = data();
    let mut rain = spell(
        RAIN,
        "Rain of Fire",
        [
            SpellEffect {
                effect: 2,
                base_points: 9,
                target_a: 16,
                ..Default::default()
            },
            SpellEffect::default(),
            SpellEffect::default(),
        ],
    );
    rain.casting_time_index = THREE_SECONDS;
    d.spells.spells.insert(RAIN, rain);
    d.spells.cast_times.insert(THREE_SECONDS, 3000);
    let realm = harness::realm_with(d);
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let (mut s, _gs, _) = enter(&realm, "S", "Stranger", Race::Human, Class::Warrior).await;
    let (_c, gc, _) = enter(&realm, "C", "Carol", Race::Human, Class::Warrior).await;
    learn(&map, gw, &[SUMMON_IMP]);
    let wolf = place(&map, gw, WOLF, 0x70_0801, 10.0).await;
    let other = place(&map, gw, WOLF, 0x70_0802, 6.0).await;
    w.sees(wolf).await;
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    map.query(move |m| crate::game::map::pets::attack_target(m, imp, wolf))
        .await
        .unwrap();
    until(&map, "the imp attacks", move |m| attacking(m, imp)).await;

    // A stranger's word is nothing to it.
    s.send(CMSG_PET_STOP_ATTACK { pet: imp }).await;
    heard(&mut s, "stop").await;
    assert!(
        map.query(move |m| attacking(m, imp)).await.unwrap(),
        "a stranger's stop is refused"
    );
    // Its master's stops it.
    w.send(CMSG_PET_STOP_ATTACK { pet: imp }).await;
    until(&map, "the imp stops", move |m| !attacking(m, imp)).await;

    // Carol, possessed by the warlock, at the wolf: stopped from the bar.
    map.query(move |m| {
        let now = m.now;
        m.players.get_mut(&gc).unwrap().owned.possessed_by =
            Some(crate::game::map::pets::possess::Possession {
                master: gw,
                spell: 605,
                was_faction: 1,
                manual: true,
                next_think: now,
                next_cast_at: now,
                melee_fallback: false,
            });
        crate::game::map::combat::start_attack(m, gc, wolf);
    })
    .await
    .unwrap();
    until(&map, "Carol attacks", move |m| attacking(m, gc)).await;
    w.send(CMSG_PET_STOP_ATTACK { pet: gc }).await;
    until(&map, "Carol stops", move |m| !attacking(m, gc)).await;

    // A wolf possessed, its rain at a place on the ground.
    let place_at = map
        .query(move |m| {
            crate::game::map::pets::charmed(m, gw, other, true);
            let at = m.players[&gw].position();
            Vector3d {
                x: at.x + 3.0,
                y: at.y - 2.0,
                z: at.z,
            }
        })
        .await
        .unwrap();
    let targets = crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
        dest: Some(place_at),
        ..Default::default()
    });
    w.send(CMSG_PET_CAST_SPELL {
        guid: other,
        id: RAIN,
        targets,
    })
    .await;
    until(&map, "the wolf casts", move |m| {
        m.caster(other).is_some_and(|c| c.casting.is_some())
    })
    .await;
    let dest = map
        .query(move |m| {
            m.caster(other)
                .and_then(|c| c.casting.as_ref())
                .map(|c| (c.spell, c.targets.dest))
        })
        .await
        .unwrap();
    assert_eq!(dest, Some((RAIN, Some(place_at))), "at the place named");
}

/// Thistle Tea's smsg_attackerstateupdate_test: a blow's line carries its
/// school's index, and its absorb and resist each on its own. The word
/// after the victim's state is mangos-zero's: -1 with an absorb (Thistle
/// Tea and mangos-classic send 0; the client's parse reads it either way).
#[test]
fn a_blows_line_carries_its_school_and_its_absorb_and_resist_apart() {
    use crate::game::map::combat;
    let frame = combat::attacker_state_full(Guid::new(1), Guid::new(2), 0x62, 15, 1, 0, 10, 75, 2);
    let mut want = Vec::new();
    want.extend(0x62u32.to_le_bytes());
    want.extend([1, 1, 1, 2]);
    want.extend(15u32.to_le_bytes());
    want.push(1);
    want.extend(2u32.to_le_bytes());
    want.extend(15.0f32.to_le_bytes());
    for w in [15u32, 10, 75, 1, u32::MAX, 0, 0] {
        want.extend(w.to_le_bytes());
    }
    assert_eq!(&frame[4..], &want[..]);
    assert_eq!(
        u16::from_le_bytes([frame[2], frame[3]]),
        combat::SMSG_ATTACKERSTATEUPDATE
    );
    // No absorb: the word is 0, as Thistle Tea has it.
    let frame = combat::attacker_state_full(Guid::new(1), Guid::new(2), 0x42, 15, 1, 0, 0, 75, 2);
    assert_eq!(&frame[frame.len() - 12..frame.len() - 8], &[0, 0, 0, 0]);
}

/// Thistle Tea's auction_messages_test, "encodes conditional command-result
/// trailers": an auction started, a bid placed (its next increment), one
/// refused for the inventory (its reason, a u32) and one outbid (the higher
/// bidder, their bid and the increment), as mangos's
/// `SendAuctionCommandResult` writes them. Before, the inventory error's
/// reason went as a byte (wow_messages'), which the client couldn't read.
#[test]
fn an_auction_commands_result_carries_the_trailer_its_result_wants() {
    type A = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction;
    type R = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResult;
    type R2 = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResultTwo;
    let body = |action: A| {
        let f = crate::game::social::auction::command_result_frame(9, &action);
        assert_eq!(u16::from_le_bytes([f[2], f[3]]), 0x25B);
        f[4..].to_vec()
    };
    let words = |w: &[u32]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    assert_eq!(body(A::Started { result2: R2::Ok }), words(&[9, 0, 0]));
    assert_eq!(
        body(A::BidPlaced {
            result: R::Ok { auction_outbid1: 5 }
        }),
        words(&[9, 2, 0, 5])
    );
    assert_eq!(
        body(A::Removed {
            result2: R2::ErrInventory {
                inventory_result2: InventoryResult::ItemNotFound
            }
        }),
        words(&[9, 1, 1, InventoryResult::ItemNotFound.as_int().into()])
    );
    let mut higher = words(&[9, 2, 5]);
    higher.extend(13u64.to_le_bytes());
    higher.extend(words(&[100, 5]));
    assert_eq!(
        body(A::BidPlaced {
            result: R::ErrHigherBid {
                higher_bidder: Guid::new(13),
                new_bid: 100,
                auction_outbid2: 5,
            }
        }),
        higher
    );
}
