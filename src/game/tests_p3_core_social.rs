//! Phase 3's `/who` and friends, Thistle Tea's `WhoTest` and `SocialTest`
//! ported: one's own side listed, by level, race, class and zone, a
//! battleground's zone from one's own copy, no more than 49 listed; the
//! friend and ignore lists' refusals, their limits, and removing.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::registry::NameEntry;

const STORMWIND: u32 = 1519;
const ELWYNN: u32 = 12;
const DUN_MOROGH: u32 = 1;
const WARSONG_GULCH: u32 = 3277;

fn data() -> GameData {
    let mut d = GameData::empty();
    let start = |map, x, y, z| StartInfo {
        map,
        zone: 0,
        x,
        y,
        z,
        orientation: 0.0,
        items: Vec::new(),
        spells: Vec::new(),
        actions: Vec::new(),
    };
    for key in [(1, 1), (1, 8), (3, 1), (2, 1)] {
        d.start.insert(key, start(0, -8949.95, -132.493, 83.5312));
    }
    // Gnome warriors start in Kalimdor, so on a map of their own.
    d.start.insert((7, 1), start(1, 10311.3, 832.463, 1326.41));
    d
}

/// A realm whose players are no GMs, whom `/who` sees by side.
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
}

async fn player(
    realm: &Arc<Realm>,
    name: &str,
    level: u8,
    race: Race,
    class: Class,
    zone: u32,
) -> Client {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, race, class).await;
    realm.registry.set_level(g, level);
    c.send(CMSG_ZONEUPDATE {
        area: Area::try_from(zone).unwrap(),
    })
    .await;
    handled(&mut c).await;
    c.clear();
    c
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => {
            Some(())
        }
        _ => None,
    })
    .await;
}

#[derive(Default)]
struct Who {
    levels: (u8, u8),
    name: &'static str,
    race_mask: u32,
    class_mask: u32,
    zones: Vec<u32>,
}

/// `/who`: how many listed, how many counted online, the names listed.
async fn who(a: &mut Client, q: Who) -> (u32, u32, Vec<String>) {
    a.clear();
    a.send(CMSG_WHO {
        minimum_level: Level::new(q.levels.0),
        maximum_level: Level::new(if q.levels.1 == 0 { 100 } else { q.levels.1 }),
        player_name: q.name.into(),
        guild_name: String::new(),
        race_mask: q.race_mask,
        class_mask: q.class_mask,
        zones: q.zones,
        search_strings: Vec::new(),
    })
    .await;
    let b = a
        .wait_frame("SMSG_WHO", crate::game::map::social::SMSG_WHO, |_| true)
        .await;
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let (listed, online) = (u32_at(0), u32_at(4));
    let mut names = Vec::new();
    let mut at = 8;
    for _ in 0..listed {
        let end = at + b[at..].iter().position(|c| *c == 0).unwrap();
        names.push(String::from_utf8_lossy(&b[at..end]).into_owned());
        at = end + 1;
        let guild_end = at + b[at..].iter().position(|c| *c == 0).unwrap();
        at = guild_end + 1 + 16;
    }
    names.sort();
    (listed, online, names)
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// Thistle Tea's `Who.list/3`: the asker's side only; by level range,
/// race and class; the zones named.
#[tokio::test]
async fn who_lists_ones_own_side_by_level_race_class_and_zone() {
    let realm = realm();
    let mut a = player(&realm, "Asker", 40, Race::Human, Class::Warrior, STORMWIND).await;
    let _b = player(&realm, "Bryn", 12, Race::Dwarf, Class::Warrior, STORMWIND).await;
    let _c = player(&realm, "Cedric", 20, Race::Human, Class::Mage, ELWYNN).await;
    let _d = player(&realm, "Dagny", 60, Race::Dwarf, Class::Warrior, DUN_MOROGH).await;
    let _g = player(&realm, "Grukk", 40, Race::Orc, Class::Warrior, STORMWIND).await;
    let all = names(&["Asker", "Bryn", "Cedric", "Dagny"]);
    assert_eq!(who(&mut a, Who::default()).await.2, all);
    let by_level = Who {
        levels: (30, 60),
        ..Default::default()
    };
    assert_eq!(who(&mut a, by_level).await.2, names(&["Asker", "Dagny"]));
    let dwarves = Who {
        race_mask: 1 << 3,
        ..Default::default()
    };
    assert_eq!(who(&mut a, dwarves).await.2, names(&["Bryn", "Dagny"]));
    let mages = Who {
        class_mask: 1 << 8,
        ..Default::default()
    };
    assert_eq!(who(&mut a, mages).await.2, names(&["Cedric"]));
    let stormwind = Who {
        zones: vec![STORMWIND],
        ..Default::default()
    };
    assert_eq!(who(&mut a, stormwind).await.2, names(&["Asker", "Bryn"]));
}

/// Thistle Tea's `/who` in a battleground: the asker's zone from their own
/// copy alone, the other copy's player listed when no zone is asked for.
#[tokio::test]
async fn who_in_a_battleground_lists_ones_own_copy() {
    let realm = realm();
    let mut a = player(
        &realm,
        "Asker",
        60,
        Race::Human,
        Class::Warrior,
        WARSONG_GULCH,
    )
    .await;
    let _same = player(
        &realm,
        "Same",
        60,
        Race::Human,
        Class::Warrior,
        WARSONG_GULCH,
    )
    .await;
    // A map of its own, the same zone.
    let _other = player(
        &realm,
        "Other",
        60,
        Race::Gnome,
        Class::Warrior,
        WARSONG_GULCH,
    )
    .await;
    let wsg = Who {
        zones: vec![WARSONG_GULCH],
        ..Default::default()
    };
    assert_eq!(who(&mut a, wsg).await.2, names(&["Asker", "Same"]));
    assert!(
        who(&mut a, Who::default())
            .await
            .2
            .contains(&"Other".to_owned())
    );
}

/// Thistle Tea's 49: no more listed, and everyone online counted past
/// that; a short list counts what it lists.
#[tokio::test]
async fn who_lists_at_most_49_and_counts_everyone_past_that() {
    let realm = realm();
    let mut players = Vec::new();
    for n in 0..51 {
        let name = format!("Player{}", (b'a' + (n / 26) as u8) as char).to_owned()
            + &((b'a' + (n % 26) as u8) as char).to_string();
        players.push(player(&realm, &name, 10, Race::Human, Class::Warrior, STORMWIND).await);
    }
    let a = &mut players[0];
    let (listed, online, _) = who(a, Who::default()).await;
    assert_eq!((listed, online), (49, 51));
    let one = Who {
        name: "playerab",
        ..Default::default()
    };
    let (listed, online, named) = who(a, one).await;
    assert_eq!((listed, online, named), (1, 51, names(&["Playerab"])));
}

/// `SMSG_FRIEND_STATUS`'s result, for what `c` last asked.
async fn friend_status(c: &mut Client) -> u8 {
    let b = c
        .wait_frame(
            "SMSG_FRIEND_STATUS",
            crate::game::social::friends::SMSG_FRIEND_STATUS,
            |_| true,
        )
        .await;
    b[0]
}

/// Characters, not online, to befriend and ignore.
fn characters(realm: &Arc<Realm>, n: u64) -> Vec<String> {
    (0..n)
        .map(|k| {
            let name = format!(
                "Pal{}{}",
                (b'a' + (k / 26) as u8) as char,
                (b'a' + (k % 26) as u8) as char
            );
            realm.registry.add_name(NameEntry {
                guid: Guid::new(10_000 + k),
                account: "PALS".into(),
                name: name.as_str().into(),
                race: 1,
                class: 1,
                gender: 0,
                level: 1.into(),
            });
            name
        })
        .collect()
}

/// Thistle Tea's `Social.add/3` and `remove/3`: oneself and no one
/// refused; fifty friends and twenty-five ignored, each list on its own,
/// one more refused and none put out for it, one already there refused;
/// one taken off makes room; a friend taken off stays ignored, and taking
/// them off again tells the same.
#[tokio::test]
async fn friends_and_ignores_keep_their_own_limits() {
    let realm = realm();
    let mut a = player(&realm, "Alice", 10, Race::Human, Class::Warrior, STORMWIND).await;
    let pals = characters(&realm, 51);
    a.send(CMSG_ADD_FRIEND {
        name: "Alice".into(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x09, "oneself");
    a.send(CMSG_ADD_FRIEND {
        name: "Nobody".into(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x04, "no one");
    a.send(CMSG_ADD_IGNORE {
        name: "Alice".into(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x0C, "oneself");
    a.send(CMSG_ADD_IGNORE {
        name: "Nobody".into(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x0D, "no one");

    for pal in &pals[..50] {
        a.send(CMSG_ADD_FRIEND { name: pal.clone() }).await;
        assert_eq!(friend_status(&mut a).await, 0x07, "added, offline");
    }
    for pal in &pals[..25] {
        a.send(CMSG_ADD_IGNORE { name: pal.clone() }).await;
        assert_eq!(friend_status(&mut a).await, 0x0F, "ignored");
    }
    a.send(CMSG_ADD_FRIEND {
        name: pals[50].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x01, "friends full");
    a.send(CMSG_ADD_IGNORE {
        name: pals[50].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x0B, "ignore full");
    a.send(CMSG_ADD_FRIEND {
        name: pals[0].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x08, "a friend already");
    a.send(CMSG_ADD_IGNORE {
        name: pals[0].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x0E, "ignored already");

    // Pal aa off the friends: room for the fifty-first, still ignored.
    a.send(CMSG_DEL_FRIEND {
        guid: Guid::new(10_000),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x05);
    a.send(CMSG_DEL_FRIEND {
        guid: Guid::new(10_000),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x05, "removed again");
    a.send(CMSG_ADD_FRIEND {
        name: pals[50].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x07);
    a.send(CMSG_ADD_IGNORE {
        name: pals[0].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x0E, "still ignored");
    a.send(CMSG_DEL_IGNORE {
        guid: Guid::new(10_000),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x10);
    a.send(CMSG_ADD_IGNORE {
        name: pals[50].clone(),
    })
    .await;
    assert_eq!(friend_status(&mut a).await, 0x0F, "room again");
}
