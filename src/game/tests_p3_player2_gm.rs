//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/dev_commands_test.exs`
//! (docs/parity/matrix.csv): what the GM commands ask for, and what they
//! refuse.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{CreatureSpawn, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_p3_combat::{HELPER, START};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

const SPAWN: u32 = 70;
/// Mana Shield's kind: an absorb of all schools.
const WARD: u32 = 64_700;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: SPAWN,
            entries: vec![HELPER],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    let mut ward = crate::game::tests_p3_entity::spell(
        WARD,
        &[crate::game::tests_p3_entity::aura_effect(
            69, 100_000, 127, 1,
        )],
    );
    ward.duration_index = 0;
    d.spells.spells.insert(WARD, ward);
    d
}

async fn setup() -> (Arc<Realm>, Client, Guid, MapHandle, Guid) {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let c = guid::unit(HELPER, SPAWN);
    a.sees(c).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map, c)
}

/// The system message `line` brings `a`, handled.
async fn says(a: &mut Client, line: &str) -> Option<String> {
    a.clear();
    a.say(line).await;
    handled(a).await;
    a.gets(std::time::Duration::from_millis(1), |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. }) =>
        {
            Some(c.message.clone())
        }
        _ => None,
    })
    .await
}

fn health(m: &crate::game::map::Map, c: Guid) -> Option<u32> {
    m.creature(c)
        .filter(|c| c.alive())
        .map(|c| c.fields.get_u32(UNIT_FIELD_HEALTH))
}

/// Thistle Tea's ".respawn requires a selected creature", ".kill requires
/// a selected creature", ".damage requires a selected creature and a
/// positive amount" and ".respawn asks the selected creature in this world
/// to respawn even if alive": with nothing selected each asks for a
/// selection; no amount, nothing, or one not a number, the usage; a living
/// creature respawned comes back whole.
#[tokio::test]
async fn the_creature_commands_ask_for_a_creature_and_a_real_amount() {
    let (_realm, mut a, _ga, map, c) = setup().await;
    let select = "Select a living unit.";
    assert_eq!(says(&mut a, ".kill").await.as_deref(), Some(select));
    assert_eq!(says(&mut a, ".damage 500").await.as_deref(), Some(select));
    assert_eq!(
        says(&mut a, ".respawn").await.as_deref(),
        Some("Select a creature.")
    );
    a.send(CMSG_SET_SELECTION { target: c }).await;
    let whole = on(&map, move |m| health(m, c)).await.unwrap();
    for args in ["", " 0", " lots"] {
        assert_eq!(
            says(&mut a, &format!(".damage{args}")).await.as_deref(),
            Some("Usage: .damage <amount>"),
            "{args:?}"
        );
    }
    assert_eq!(on(&map, move |m| health(m, c)).await, Some(whole), "unhurt");
    assert_eq!(says(&mut a, ".damage 10").await, None);
    let hurt = on(&map, move |m| health(m, c)).await.unwrap();
    assert!(hurt < whole, "{hurt} of {whole}");
    assert_eq!(
        says(&mut a, ".respawn").await.as_deref(),
        Some("Respawning.")
    );
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, move |m| health(m, c)).await != Some(whole) {
        assert!(tokio::time::Instant::now() < deadline, "never back whole");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Thistle Tea's ".go xyz rejects malformed coordinates without
/// teleporting": too few numbers, words for them, a stray word past the
/// facing, or a facing not a number, and the GM stays where they are.
#[tokio::test]
async fn go_takes_three_numbers_a_map_and_a_facing_and_nothing_else() {
    let (_realm, mut a, ga, map, _c) = setup().await;
    let usage = "Usage: .go [xyz] <x> <y> <z> [map] [facing]";
    for args in [
        "",
        "1",
        "1 2",
        "1 2 3 0 extra",
        "1 2 3 0 1 2",
        "bad 2 3",
        "1 2m 3",
        "1 2 3m",
        "1 2 3 here 1x",
    ] {
        assert_eq!(
            says(&mut a, &format!(".go xyz {args}")).await.as_deref(),
            Some(usage),
            "{args:?}"
        );
    }
    for args in ["1 2 3 -1", "1 2 3 0x"] {
        let said = says(&mut a, &format!(".go xyz {args}")).await;
        assert!(
            said.as_deref()
                .is_some_and(|s| s.ends_with("isn't a map id.")),
            "{args:?}: {said:?}"
        );
    }
    let at = on(&map, move |m| m.players.get(&ga).map(|p| p.position().x)).await;
    assert_eq!(at, Some(START.0), "not moved");
}

/// Thistle Tea's ".mail rejects a missing message", ".mail posts an
/// immediate letter to an offline character" and ".battleground shows
/// usage for unsupported subcommands".
#[tokio::test]
async fn mail_wants_a_message_and_reaches_one_logged_out() {
    let (realm, mut a, _ga, _map, _c) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    assert_eq!(
        says(&mut a, ".mail Bob").await.as_deref(),
        Some("Usage: .mail <recipient> <message>")
    );
    assert_eq!(
        says(&mut a, ".mail Bob hello there").await.as_deref(),
        Some("Mailed Bob.")
    );
    let letters: Vec<(String, bool)> = {
        let mails = realm.social.mail.lock();
        mails
            .mails
            .values()
            .filter(|m| m.receiver == guid::counter(gb))
            .map(|m| {
                (
                    m.body.clone(),
                    m.deliver_at <= crate::game::social::mail::now(),
                )
            })
            .collect()
    };
    assert_eq!(letters, vec![("hello there".to_string(), true)]);
    // And `.battleground`'s usage for what it doesn't know.
    let said = says(&mut a, ".battleground nonsense").await;
    assert!(
        said.as_deref()
            .is_some_and(|s| s.starts_with("Usage: .bg join")),
        "{said:?}"
    );
}

/// Thistle Tea's ".die and .revive die through shields without spending
/// mana, then revive at full health": a shield that would take any blow
/// doesn't stop `.die`; `.revive` brings the dead back whole. (A warrior
/// has no mana to spend.)
#[tokio::test]
async fn die_goes_through_a_shield_and_revive_brings_back_whole() {
    let (_realm, mut a, ga, map, _c) = setup().await;
    let shielded = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, WARD, Some(ga));
        m.players[&ga].auras.has_spell(WARD)
    })
    .await;
    assert!(shielded);
    says(&mut a, ".die").await;
    let dead = on(&map, move |m| {
        m.players[&ga].death != crate::game::map::death::Death::Alive
    })
    .await;
    assert!(dead, "dead through the shield");
    says(&mut a, ".revive").await;
    let (alive, health, max) = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.death == crate::game::map::death::Death::Alive,
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert!(alive);
    assert_eq!(health, max, "whole");
}
