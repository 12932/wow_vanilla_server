//! Packets in, round 8: a pet's commands from its master's bar, and the
//! layouts of what the server answers with.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_pets::{
    SUMMON_IMP, WOLF, cast, data, enter, learn, pet_bar, place, place_in, spell,
};

/// A ground spell a possessed creature may cast: fire on all enemies at a
/// place, three seconds to cast.
const RAIN: u32 = 900_801;
const THREE_SECONDS: u32 = 14;
/// What holds the wolf: a possession's aura, without which it goes free
/// at its next step.
const POSSESS: u32 = 900_802;

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
    let mut hold = SpellEffect {
        effect: 6,
        target_a: 6,
        ..Default::default()
    };
    hold.aura = crate::game::map::auras::AURA_MOD_POSSESS;
    d.spells.spells.insert(
        POSSESS,
        spell(
            POSSESS,
            "Possession",
            [hold, SpellEffect::default(), SpellEffect::default()],
        ),
    );
    let realm = harness::realm_with(d);
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let (mut s, _gs, _) = enter(&realm, "S", "Stranger", Race::Human, Class::Warrior).await;
    let (_c, gc, _) = enter(&realm, "C", "Carol", Race::Human, Class::Warrior).await;
    learn(&map, gw, &[SUMMON_IMP]);
    let wolf = place(&map, gw, WOLF, 0x70_0801, 10.0).await;
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

    // A wolf possessed, its rain at a place on the ground. It comes as it
    // is taken, and an aura holds it: taken bare, it went free at its next
    // step and turned on the warlock, and her imp killed it (a dead wolf
    // casts nothing) whenever an update came before her cast.
    let (other, place_at) = map
        .query(move |m| {
            let other = place_in(m, gw, WOLF, 0x70_0802, 6.0);
            let possess = m.realm.data.spells.get(POSSESS).cloned().unwrap();
            crate::game::map::auras::apply(m, gw, other, &possess, 0, 0);
            // A possessed unit casts what its bar has (its charm spells).
            if let Some(s) = crate::game::map::pets::state_mut(m, other) {
                s.spells.push(RAIN);
            }
            let at = m.players[&gw].position();
            let place_at = Vector3d {
                x: at.x + 3.0,
                y: at.y - 2.0,
                z: at.z,
            };
            (other, place_at)
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

/// A blow's hit info is a set of flags in 1.12: a crushing blow (0x8002)
/// and a main hand's miss (0x12), as the Eranikus fight sends them, read
/// back by wow_messages (as the harness and peon read them) with each bit.
/// Before, wow_messages read the hit info as one value and refused both.
#[test]
fn a_blow_of_several_flags_reads_back_with_each() {
    use crate::game::map::combat::{self, *};
    let read = |hit_info| {
        let f = combat::attacker_state(
            Guid::new(1),
            Guid::new(2),
            hit_info,
            7,
            VICTIMSTATE_NORMAL,
            0,
        );
        match ServerOpcodeMessage::read_unencrypted(&f[..]) {
            Ok(ServerOpcodeMessage::SMSG_ATTACKERSTATEUPDATE(m)) => m.hit_info,
            other => panic!("{hit_info:#x} read as {other:?}"),
        }
    };
    let crushing = read(HITINFO_NORMALSWING2 | HITINFO_CRUSHING);
    assert!(crushing.get_affects_victim() && crushing.get_crushing() && !crushing.get_miss());
    let miss = read(HITINFO_NORMALSWING2 | HITINFO_MISS);
    assert!(miss.get_affects_victim() && miss.get_miss() && !miss.get_crushing());
}

/// Thistle Tea's auction_messages_test, "encodes conditional command-result
/// trailers": an auction started, a bid placed (its next increment), one
/// refused for the inventory (its reason, a u32) and one outbid (the higher
/// bidder, their bid and the increment), as mangos's
/// `SendAuctionCommandResult` writes them, and each read back. Before, the
/// inventory error's reason went as a byte (wow_messages'), which the
/// client couldn't read, and wvs wrote the packet by hand.
#[test]
fn an_auction_commands_result_carries_the_trailer_its_result_wants() {
    type A = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction;
    type R = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResult;
    type R2 = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResultTwo;
    let body = |action: A| {
        let m = SMSG_AUCTION_COMMAND_RESULT {
            auction_id: 9,
            action,
        };
        let f = crate::game::conn::encode(&m).expect("encodes");
        assert_eq!(u16::from_le_bytes([f[2], f[3]]), 0x25B);
        match ServerOpcodeMessage::read_unencrypted(&f[..]) {
            Ok(ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(back)) => assert_eq!(back, m),
            other => panic!("read back as {other:?}"),
        }
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

/// Whether `c` is told it is queued for Arathi Basin.
async fn queued_for_arathi(c: &mut Client) {
    c.wait_frame(
        "queued",
        crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS,
        |f| f.len() >= 17 && u32::from_le_bytes([f[4], f[5], f[6], f[7]]) == 529,
    )
    .await;
}

/// Thistle Tea's battleground_messages_test, "dispatches the build-5875
/// queue-list payload and preserves group and instance selection": the
/// 1.12.1 client joins from a list shown with no battlemaster (the
/// window's, `CMSG_BATTLEFIELD_LIST`) with `CMSG_BATTLEFIELD_JOIN`: the
/// map, the instance and the group's byte (its `0x4AA00B`). Read by
/// wow_messages, as the reader task reads it, a leader's join with the
/// byte set queues the group; one without it queues its sender alone.
/// Before, wow_messages wanted the map alone and read nothing the client
/// sent (wvs read it by hand).
#[tokio::test]
async fn a_join_from_the_windows_list_queues_as_the_client_asks() {
    assert!(
        !crate::game::conn::RAW_OPCODES.contains(&0x23E),
        "read by wow_messages"
    );
    // As the client writes it: the header (its size big-endian, the opcode
    // a u32), the map, the instance and the group's byte.
    let read = |as_group: u8| {
        let mut p = vec![0, 13, 0x3E, 0x02, 0, 0];
        p.extend(529u32.to_le_bytes());
        p.extend(0u32.to_le_bytes());
        p.push(as_group);
        match wow_world_messages::vanilla::opcodes::ClientOpcodeMessage::read_unencrypted(&p[..]) {
            Ok(
                wow_world_messages::vanilla::opcodes::ClientOpcodeMessage::CMSG_BATTLEFIELD_JOIN(m),
            ) => m,
            other => panic!("the client's join, read as {other:?}"),
        }
    };
    let (group, alone) = (read(1), read(0));
    assert_eq!(
        (group.map, group.instance_id, group.join_as_group),
        (wow_world_messages::vanilla::Map::ArathiBasin, 0, true)
    );
    assert!(!alone.join_as_group);
    let realm = harness::realm_with(crate::game::tests_pvp::arathi());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Warrior).await;
    let (ao, bo) = (
        realm.registry.online(a.guid).unwrap(),
        realm.registry.online(b.guid).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &ao, &bo));
    a.send(group).await;
    queued_for_arathi(&mut a).await;
    queued_for_arathi(&mut b).await;
    c.send(alone).await;
    queued_for_arathi(&mut c).await;
}

/// Until the realm's count of those online is `n`.
async fn online(realm: &std::sync::Arc<crate::game::Realm>, n: usize) {
    for _ in 0..1000 {
        if realm.registry.online_count() == n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(realm.registry.online_count(), n, "online");
}

/// Thistle Tea's sessions_test: the realm's population (what shortens a
/// vendor's restock, past 2,500) counts each account once, a second
/// login of the account taking the first's place, and drops one as it
/// goes. wvs counts those in the world (`Registry::online_count`), not
/// those on the character screen, where Thistle Tea counts each session
/// authenticated.
#[tokio::test]
async fn the_population_counts_each_account_in_the_world_once() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let first = a.enter("Firstone", Race::Human, Class::Warrior).await;
    online(&realm, 1).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    online(&realm, 2).await;
    // The account again, with another character: the first goes.
    let mut again = Client::connect(&realm, "A");
    again
        .send(CMSG_CHAR_CREATE {
            name: "Secondone".to_owned(),
            race: Race::Human,
            class: Class::Warrior,
            gender: Gender::Female,
            skin_color: 1,
            face: 1,
            hair_style: 1,
            hair_color: 1,
            facial_hair: 1,
        })
        .await;
    again
        .wait_for("SMSG_CHAR_CREATE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_CHAR_CREATE(_)).then_some(())
        })
        .await;
    let second = realm.registry.guid_by_name("Secondone").unwrap();
    again.send(CMSG_PLAYER_LOGIN { guid: second }).await;
    again
        .wait_for("the first login's refusal, as it kicks the first", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(_)).then_some(())
        })
        .await;
    for _ in 0..1000 {
        if realm.registry.online(first).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    again.login(second).await;
    online(&realm, 2).await;
    assert!(realm.registry.online(first).is_none(), "the first has gone");
    drop(a);
    b.disconnect();
    online(&realm, 1).await;
    again.disconnect();
    online(&realm, 0).await;
}

/// Thistle Tea's update_batcher_test, "personalizes updates drained out of
/// the mailbox too": every creature in an update's batch is each viewer's
/// own view of it, not the first alone. Two raptors tapped by Ana in one
/// update reach Bea grey, both in the one packet, and Ana not.
#[tokio::test]
async fn every_creature_in_an_updates_batch_is_its_viewers_own() {
    use crate::game::tests_client_emu::{START, spawn, template};
    const RAPTOR: u32 = 900_810;
    const TAPPED: u32 = 0x4;
    let mut d = crate::game::data::GameData::empty();
    let mut raptor = template(RAPTOR, "Raptor", 7);
    raptor.speed_walk = 0.01;
    raptor.speed_run = 0.01;
    raptor.detection_range = 0.1;
    d.creature_templates.insert(RAPTOR, raptor);
    d.creature_spawns.insert(
        0,
        vec![
            spawn(81, RAPTOR, (START.0 + 8.0, START.1, START.2)),
            spawn(82, RAPTOR, (START.0 + 8.0, START.1 + 4.0, START.2)),
        ],
    );
    let realm = harness::realm_with(d);
    let (mut a, ga, map) = enter(&realm, "A", "Ana", Race::Human, Class::Warrior).await;
    let (mut b, _gb, _) = enter(&realm, "B", "Bea", Race::Human, Class::Warrior).await;
    let (r1, r2) = (
        crate::game::guid::unit(RAPTOR, 81),
        crate::game::guid::unit(RAPTOR, 82),
    );
    for c in [&mut a, &mut b] {
        c.sees(r1).await;
        c.sees(r2).await;
        c.clear();
    }
    map.query(move |m| {
        crate::game::map::ai::attacked(m, r1, ga);
        crate::game::map::ai::attacked(m, r2, ga);
    })
    .await
    .unwrap();
    let both = |m: &ServerOpcodeMessage| {
        let flags = |of: Guid| {
            harness::values(m)
                .into_iter()
                .filter_map(|(g, mask)| match mask {
                    UpdateMask::Unit(u) if g == of => u.unit_dynamic_flags(),
                    _ => None,
                })
                .next_back()
        };
        Some((flags(r1)? as u32, flags(r2)? as u32))
    };
    let (f1, f2) = b.wait_for("both raptors, in one packet", both).await;
    assert!(
        f1 & TAPPED != 0 && f2 & TAPPED != 0,
        "grey to Bea: {f1:#x} {f2:#x}"
    );
    let (f1, f2) = a.wait_for("both raptors, in one packet", both).await;
    assert!(
        f1 & TAPPED == 0 && f2 & TAPPED == 0,
        "not to Ana: {f1:#x} {f2:#x}"
    );
}
