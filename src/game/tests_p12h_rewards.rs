//! Round 12's stage h, kill rewards as mangos-classic gives them: the grey
//! level at 60 its `IsTrivialLevelDifference`'s (47, not the colour's 51),
//! and a group's share of a kill anywhere in a dungeon or raid copy
//! (`IsAtGroupRewardDistance`).

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::{FactionDef, KillReputation, RANK_EXALTED};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::reputation::standing;
use crate::game::map::{Map, MapKey, combat};

const ARGENT: u32 = 529;
const GHOUL: u32 = 20_310;
const DEADMINES: u32 = 36;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.misc.factions.insert(
        ARGENT,
        FactionDef {
            id: ARGENT,
            index: 13,
            name: "Argent Dawn".into(),
            variants: [(0xFF, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(13, ARGENT);
    d.misc.kill_reputation.insert(
        GHOUL,
        vec![KillReputation {
            faction: ARGENT,
            value: 100,
            max_rank: RANK_EXALTED,
            team: 0,
            team_award: false,
        }],
    );
    d.maps.insert(
        DEADMINES,
        MapInfo {
            id: DEADMINES,
            internal_name: "DeadminesInstance".into(),
            name: "Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

/// A ghoul of `level` beside `g`.
fn ghoul(m: &mut Map, g: Guid, level: u8) -> Guid {
    let at = m.players[&g].position();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let t = CreatureTemplate {
        entry: GHOUL,
        name: "Ghoul".into(),
        display_ids: [903, 0, 0, 0],
        health_max: 100,
        ..Default::default()
    };
    let c = crate::game::map::creature::Creature::new(
        crate::game::guid::unit(GHOUL, counter | 0x80_0000),
        &t,
        level,
        Vector3d {
            x: at.x + 2.0,
            ..at
        },
        0.0,
    );
    m.add_creature(c)
}

fn set_level(m: &mut Map, g: Guid, level: u32) {
    m.players
        .get_mut(&g)
        .unwrap()
        .fields
        .set_u32(UNIT_FIELD_LEVEL, level);
}

/// mangos's `IsTrivialLevelDifference`, by which its kills reward (XP,
/// honor, reputation, soul shards, procs): at 60 only a gap over 12 is
/// trivial, so a level 48 ghoul gives a 60 its whole reputation, where wvs
/// took 51 and below as grey (a fifth).
#[tokio::test]
async fn at_60_a_level_48_kill_isnt_grey() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let got = map
        .query(move |m| {
            let data = m.realm.data.clone();
            set_level(m, g, 60);
            let v = ghoul(m, g, 48);
            combat::kill(m, g, v);
            let at_48 = standing(&m.players[&g], &data, ARGENT);
            let v = ghoul(m, g, 47);
            combat::kill(m, g, v);
            (at_48, standing(&m.players[&g], &data, ARGENT) - at_48)
        })
        .await
        .unwrap();
    assert_eq!(got, (100, 20), "48 whole, 47 a fifth");
    // The levels below 60 were mangos's already.
    for (level, grey) in [
        (5, 0),
        (9, 4),
        (19, 13),
        (29, 22),
        (39, 31),
        (49, 39),
        (59, 47),
        (60, 47),
    ] {
        assert_eq!(combat::gray_level(level), grey, "at {level}");
    }
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
            _ => None,
        })
        .await;
}

/// Send `c` to `map` (its group's copy, for a dungeon) and wait for it to
/// land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) -> MapKey {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| {
        m.teleport(
            g,
            MapKey::world(map),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).unwrap().map().key
}

/// A kill by `ga`, `gb` 200 yards off: `gb`'s reputation and XP from it.
async fn far_share(realm: &Arc<Realm>, key: MapKey, ga: Guid, gb: Guid) -> (i32, u32) {
    let map = realm.maps.get(realm, key);
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&gb).unwrap();
        p.movement.position.x += 200.0;
        let xp = m.players[&gb].fields.get_u32(PLAYER_XP);
        let v = ghoul(m, ga, 1);
        combat::kill(m, ga, v);
        let p = &m.players[&gb];
        (standing(p, &data, ARGENT), p.fields.get_u32(PLAYER_XP) - xp)
    })
    .await
    .unwrap()
}

/// mangos's `IsAtGroupRewardDistance`: in a dungeon or raid every member
/// on the map shares a kill's XP, reputation and quest credit, however
/// far; outside, only those within 74 yards (wvs held to 74 everywhere).
#[tokio::test]
async fn in_a_dungeon_the_whole_group_shares_a_kill() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    let outside = far_share(&realm, MapKey::world(0), ga, gb).await;
    assert_eq!(outside, (0, 0), "too far in the open world");
    let key = go(&realm, &mut a, DEADMINES).await;
    assert_eq!(go(&realm, &mut b, DEADMINES).await, key, "one copy");
    let inside = far_share(&realm, key, ga, gb).await;
    assert_eq!(inside.0, 100, "the reputation");
    assert!(inside.1 > 0, "a share of the XP");
}
