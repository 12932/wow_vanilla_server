//! Round 12's zone channels, as mangos's `Player::UpdateLocalChannels`
//! moves them: General and LocalDefense follow the player into each zone
//! they enter ("Changed Channel", the old one left without a word, the
//! client having swapped it itself); Trade stays "Trade - City" from one
//! capital to the next and is left outside them; WorldDefense, global,
//! stays.

use std::sync::Arc;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::social::channels::{LEFT, YOU_JOINED, YOU_LEFT};

const ELWYNN: u32 = 12;
const WESTFALL: u32 = 40;
const STORMWIND: u32 = 1519;
const IRONFORGE: u32 = 1537;

fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, name, flags) in [
        (ELWYNN, "Elwynn Forest", 0),
        (WESTFALL, "Westfall", 0),
        (STORMWIND, "Stormwind City", AREA_FLAG_CAPITAL),
        (IRONFORGE, "Ironforge", AREA_FLAG_CAPITAL),
    ] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map: 0,
                zone: 0,
                explore_bit: id,
                flags,
                level: 1,
                name: name.into(),
                team: 0,
            },
        );
    }
    d
}

fn realm() -> Arc<Realm> {
    harness::realm_with(data())
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

async fn zone(c: &mut Client, zone: u32) {
    c.send(CMSG_ZONEUPDATE {
        area: Area::try_from(zone).unwrap(),
    })
    .await;
    handled(c).await;
}

async fn join(c: &mut Client, channel: &str) {
    c.send(CMSG_JOIN_CHANNEL {
        channel_name: channel.into(),
        channel_password: String::new(),
    })
    .await;
    handled(c).await;
}

/// The channel notices `c` was sent since it last looked: kind and name.
async fn notices(c: &mut Client) -> Vec<(u8, String)> {
    handled(c).await;
    c.take_frames(crate::game::social::channels::SMSG_CHANNEL_NOTIFY)
        .iter()
        .map(|b| {
            let end = 1 + b[1..].iter().position(|c| *c == 0).unwrap();
            (b[0], String::from_utf8_lossy(&b[1..end]).into_owned())
        })
        .collect()
}

fn channels_of(realm: &Arc<Realm>, c: &Client) -> Vec<String> {
    let mut names = realm.social.channels.lock().of(c.guid);
    names.sort();
    names
}

#[tokio::test]
async fn zone_channels_follow_the_player() {
    let realm = realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    zone(&mut a, ELWYNN).await;
    zone(&mut b, WESTFALL).await;
    join(&mut a, "General - Elwynn Forest").await;
    join(&mut a, "LocalDefense - Elwynn Forest").await;
    join(&mut a, "WorldDefense").await;
    join(&mut b, "General - Westfall").await;
    a.clear();
    b.clear();

    // Into Westfall: its General and LocalDefense, Elwynn's left quietly.
    zone(&mut a, WESTFALL).await;
    let mut told = notices(&mut a).await;
    told.sort();
    assert_eq!(
        told,
        vec![
            (YOU_JOINED, "General - Westfall".to_owned()),
            (YOU_JOINED, "LocalDefense - Westfall".to_owned()),
        ],
        "joined, and no YOU_LEFT"
    );
    assert_eq!(
        channels_of(&realm, &a),
        vec![
            "General - Westfall".to_owned(),
            "LocalDefense - Westfall".to_owned(),
            "WorldDefense".to_owned(),
        ]
    );
    // Bob, in Westfall's General, hears her there.
    a.send(CMSG_MESSAGECHAT {
        chat_type: CMSG_MESSAGECHAT_ChatType::Channel {
            channel: "General - Westfall".into(),
        },
        language: Language::Common,
        message: "hello Westfall".into(),
    })
    .await;
    let heard = b
        .wait_for("the line", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(l) => Some(l.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(heard, "hello Westfall");
    assert!(
        !notices(&mut b)
            .await
            .iter()
            .any(|(k, _)| *k == LEFT || *k == YOU_LEFT),
        "nothing told of the move"
    );

    // Into Stormwind, and Trade - City; on to Ironforge, Trade stays.
    zone(&mut a, STORMWIND).await;
    join(&mut a, "Trade - City").await;
    zone(&mut a, IRONFORGE).await;
    assert_eq!(
        channels_of(&realm, &a),
        vec![
            "General - Ironforge".to_owned(),
            "LocalDefense - Ironforge".to_owned(),
            "Trade - City".to_owned(),
            "WorldDefense".to_owned(),
        ]
    );
    // Out to Elwynn: Trade is left, as the client leaves it.
    a.clear();
    zone(&mut a, ELWYNN).await;
    assert_eq!(
        channels_of(&realm, &a),
        vec![
            "General - Elwynn Forest".to_owned(),
            "LocalDefense - Elwynn Forest".to_owned(),
            "WorldDefense".to_owned(),
        ]
    );
    assert!(
        !notices(&mut a).await.iter().any(|(k, _)| *k == YOU_LEFT),
        "left without a word"
    );
}
