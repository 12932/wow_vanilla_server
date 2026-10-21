//! Round 12's guilds, as mangos-classic runs them: the roster shows an
//! offline member's days away and the zone they left in (`Guild::Roster`);
//! a charter's petition reserves no name, so one destroyed frees it
//! (R4-22), and of two charters of one name the first turned in founds it
//! (`HandlePetitionBuyOpcode`, `HandleTurnInPetitionOpcode`).

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::social::guilds::CHARTER_ITEM;

async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, Race::Human, Class::Warrior).await;
    c.clear();
    c
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

async fn on<R: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> R + Send + 'static,
) -> R {
    map.query(f).await.expect("the map answers")
}

/// mangos's `Guild::Roster`: a member offline is shown with the days
/// since they left and the zone they left in. Before, every one was
/// "<1 hour", nowhere.
#[tokio::test]
async fn the_roster_shows_when_and_where_a_member_left() {
    let realm = harness::realm();
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let gb = b.guid;
    a.send(CMSG_GUILD_CREATE {
        guild_name: "Keepers".into(),
    })
    .await;
    a.send(CMSG_GUILD_INVITE {
        invited_player: "Bob".into(),
    })
    .await;
    handled(&mut a).await;
    b.send(CMSG_GUILD_ACCEPT {}).await;
    b.send(CMSG_ZONEUPDATE {
        area: Area::try_from(40).unwrap(),
    })
    .await;
    handled(&mut b).await;
    b.disconnect();
    harness::offline(&realm, gb).await;
    // Two days ago, as far as the roster can tell.
    {
        let mut guilds = realm.social.guilds.lock();
        let m = guilds
            .guilds
            .values_mut()
            .flat_map(|g| g.members.iter_mut())
            .find(|m| m.guid == crate::game::guid::counter(gb))
            .expect("Bob in the guild");
        assert_eq!(m.logout_zone, 40, "left in Westfall");
        assert!(m.logout_at > 0);
        m.logout_at -= 2 * 86_400;
    }
    a.clear();
    a.send(CMSG_GUILD_ROSTER {}).await;
    let (status, area) = a
        .wait_for("SMSG_GUILD_ROSTER", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_ROSTER(r) => r
                .members
                .iter()
                .find(|m| m.guid == gb)
                .map(|m| (m.status, m.area)),
            _ => None,
        })
        .await;
    let GuildMember_GuildMemberStatus::Offline { time_offline } = status else {
        panic!("Bob is offline: {status:?}");
    };
    assert!((time_offline - 2.0).abs() < 0.01, "{time_offline} days");
    assert_eq!(area.as_int(), 40);
}

/// A realm with charters that need no signature.
fn charter_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.petition_signatures = 0;
    let mut d = crate::game::data::GameData::empty();
    d.items.insert(
        CHARTER_ITEM,
        crate::game::data::ItemTemplate {
            entry: CHARTER_ITEM,
            name: "Guild Charter".into(),
            stackable: 1,
            max_count: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

async fn petitioner(c: &mut Client, map: &MapHandle) -> Guid {
    let g = c.guid;
    let npc = on(map, move |m| {
        m.players.get_mut(&g).unwrap().set_money(100_000);
        crate::game::tests_p6_player::npc_beside(
            m,
            g,
            crate::game::map::npc::NPC_FLAG_PETITIONER,
            2.0,
        )
    })
    .await;
    c.sees(npc).await;
    npc
}

async fn buy(c: &mut Client, npc: Guid, name: &str) {
    c.send(CMSG_PETITION_BUY {
        npc,
        name: name.into(),
        index: 1,
        ..Default::default()
    })
    .await;
    handled(c).await;
}

async fn charter(map: &MapHandle, g: Guid) -> Option<Guid> {
    on(map, move |m| {
        m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == CHARTER_ITEM)
            .map(|i| i.guid)
    })
    .await
}

/// R4-22, as mangos: a charter's name is a guild's to take, not the
/// charter's to keep. One destroyed leaves its name to the next buyer;
/// two of one name are both sold, and the second turned in is told the
/// name is a guild's.
#[tokio::test]
async fn a_charter_reserves_no_name() {
    let realm = charter_realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Carol").await;
    let npc = petitioner(&mut a, &map).await;
    let _ = petitioner(&mut b, &map).await;
    let _ = petitioner(&mut c, &map).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    buy(&mut a, npc, "Fellowship").await;
    assert!(charter(&map, ga).await.is_some());
    // Alice's thrown away: Bob buys one of its name.
    on(&map, move |m| {
        let i = m.players[&ga]
            .items
            .iter()
            .position(|i| i.entry == CHARTER_ITEM)
            .unwrap();
        crate::game::map::inventory::take_item(m, ga, i, 1);
    })
    .await;
    buy(&mut b, npc, "Fellowship").await;
    let bobs = charter(&map, gb)
        .await
        .expect("Bob's charter, the name free");
    // Carol's of the same name is sold too.
    buy(&mut c, npc, "Fellowship").await;
    let carols = charter(&map, gc).await.expect("Carol's charter");
    // Bob's turned in first founds it; Carol's is told the name is taken.
    b.send(CMSG_TURN_IN_PETITION { petition: bobs }).await;
    b.wait_for("SMSG_TURN_IN_PETITION_RESULTS", |m| match m {
        ServerOpcodeMessage::SMSG_TURN_IN_PETITION_RESULTS(r) => {
            (r.result == PetitionResult::Ok).then_some(())
        }
        _ => None,
    })
    .await;
    c.clear();
    c.send(CMSG_TURN_IN_PETITION { petition: carols }).await;
    let told = c
        .wait_for("SMSG_GUILD_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_COMMAND_RESULT(r) => Some(r.result),
            _ => None,
        })
        .await;
    assert_eq!(told, GuildCommandResult::GuildNameExistsS);
    let guilds = realm.social.guilds.lock();
    assert_eq!(guilds.name_of(gb), "Fellowship");
    assert_eq!(guilds.name_of(gc), "");
}
