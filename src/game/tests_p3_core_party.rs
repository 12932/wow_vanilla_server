//! Phase 3's parties, Thistle Tea's `PartyTest` ported: who may be invited
//! and by whom, accepting and declining, leaving and the leader's place,
//! taking a member or an invitation back, the leader's own rights, one
//! side to a party, and round robin's turn passing over those away.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const RABBIT: u32 = 3160;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        RABBIT,
        CreatureTemplate {
            entry: RABBIT,
            name: "Rabbit".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 1,
            level_max: 1,
            faction: 7,
            unit_class: 1,
            health_max: 10,
            gold_min: 5,
            gold_max: 5,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        (1..=4)
            .map(|n| CreatureSpawn {
                guid: n,
                entries: vec![RABBIT],
                map: 0,
                x: START.0 + 3.0,
                y: START.1 + n as f32,
                z: START.2,
                respawn_min: 300,
                respawn_max: 300,
                ..Default::default()
            })
            .collect(),
    );
    d
}

/// A realm whose players are no GMs, who may group across the sides.
fn realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data(), None)
}

async fn player(realm: &Arc<Realm>, name: &str, race: Race) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, race, Class::Warrior).await;
    c.clear();
    c
}

/// Wait until `c`'s earlier packets have been handled by its map, and the
/// group service they reach.
async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// What `c` was told of a party command since it last looked.
async fn told(c: &mut Client) -> Option<PartyResult> {
    handled(c).await;
    c.gets(Duration::from_millis(20), |m| match m {
        ServerOpcodeMessage::SMSG_PARTY_COMMAND_RESULT(r) => Some(r.result),
        _ => None,
    })
    .await
}

async fn invite(by: &mut Client, name: &str) -> Option<PartyResult> {
    by.clear();
    by.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    told(by).await
}

async fn invited(c: &mut Client) {
    c.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
}

async fn joins(leader: &mut Client, c: &mut Client, name: &str) {
    assert_eq!(invite(leader, name).await, Some(PartyResult::Success));
    invited(c).await;
    c.send(CMSG_GROUP_ACCEPT {}).await;
    c.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
}

/// `g`'s group: its members, its leader, and whom it has invited.
fn group(realm: &Arc<Realm>, g: Guid) -> Option<(Vec<Guid>, Guid, Vec<Guid>)> {
    let groups = realm.social.groups.lock();
    groups.group_of(g).map(|gr| {
        (
            gr.members.iter().map(|m| m.guid).collect(),
            gr.leader,
            gr.invited.clone(),
        )
    })
}

/// Thistle Tea's `invite/4`, `accept/3` and `decline/2`: no invitation for
/// one already invited or grouped, none from a member who doesn't lead,
/// none into a full party; accepting with no invitation, or into a party
/// filled since, does nothing; a declined invitation tells the inviter and
/// is gone, and declining none does nothing.
#[tokio::test]
async fn invitations_go_to_the_free_from_the_leader_into_room() {
    let realm = realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    let mut b = player(&realm, "Bob", Race::Human).await;
    let mut c = player(&realm, "Cid", Race::Dwarf).await;
    let mut d = player(&realm, "Dee", Race::Gnome).await;
    let mut e = player(&realm, "Eve", Race::NightElf).await;
    let mut f = player(&realm, "Fay", Race::Human).await;
    let (ga, gb) = (a.guid, b.guid);

    // Accepting with no invitation does nothing.
    b.send(CMSG_GROUP_ACCEPT {}).await;
    handled(&mut b).await;
    assert!(group(&realm, gb).is_none());

    assert_eq!(invite(&mut a, "Bob").await, Some(PartyResult::Success));
    invited(&mut b).await;
    assert_eq!(
        invite(&mut c, "Bob").await,
        Some(PartyResult::AlreadyInGroup),
        "invited already"
    );
    // Declined: the inviter told, the invitation gone.
    b.send(CMSG_GROUP_DECLINE {}).await;
    let by = a
        .wait_for("SMSG_GROUP_DECLINE", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_DECLINE(d) => Some(d.name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(by, "Bob");
    b.send(CMSG_GROUP_ACCEPT {}).await;
    handled(&mut b).await;
    assert!(group(&realm, gb).is_none(), "the invitation gone");
    a.clear();
    b.send(CMSG_GROUP_DECLINE {}).await;
    handled(&mut b).await;
    handled(&mut a).await;
    let again = a
        .gets(Duration::from_millis(20), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_DECLINE(_)).then_some(())
        })
        .await;
    assert!(again.is_none(), "no invitation to decline");

    joins(&mut a, &mut b, "Bob").await;
    assert_eq!(
        invite(&mut c, "Bob").await,
        Some(PartyResult::AlreadyInGroup),
        "grouped already"
    );
    assert_eq!(
        invite(&mut b, "Cid").await,
        Some(PartyResult::NotLeader),
        "not the leader"
    );
    joins(&mut a, &mut c, "Cid").await;
    joins(&mut a, &mut d, "Dee").await;
    // Two invited with one place left: the second to accept finds it full.
    assert_eq!(invite(&mut a, "Eve").await, Some(PartyResult::Success));
    assert_eq!(invite(&mut a, "Fay").await, Some(PartyResult::Success));
    invited(&mut e).await;
    invited(&mut f).await;
    e.send(CMSG_GROUP_ACCEPT {}).await;
    handled(&mut e).await;
    f.clear();
    f.send(CMSG_GROUP_ACCEPT {}).await;
    assert_eq!(told(&mut f).await, Some(PartyResult::GroupFull));
    let (members, leader, _) = group(&realm, ga).unwrap();
    assert_eq!((members.len(), leader), (5, ga));
    assert!(group(&realm, f.guid).is_none());
    assert_eq!(
        invite(&mut a, "Fay").await,
        Some(PartyResult::GroupFull),
        "full"
    );
}

/// Thistle Tea's `leave/2`, `uninvite/3`, `set_leader/3` and `set_loot/5`:
/// a member leaves a party of three and the rest stay; the leader leaving
/// hands it on; leaving none does nothing; the leader takes a member out,
/// or an invitation back; a member can't, nor take out one not in the
/// party, nor make a leader, nor set the loot.
#[tokio::test]
async fn the_leader_alone_takes_members_out_and_sets_the_rules() {
    let realm = realm();
    let mut a = player(&realm, "Alice", Race::Human).await;
    let mut b = player(&realm, "Bob", Race::Human).await;
    let mut c = player(&realm, "Cid", Race::Dwarf).await;
    let mut d = player(&realm, "Dee", Race::Gnome).await;
    let (ga, gb, gc, gd) = (a.guid, b.guid, c.guid, d.guid);

    // Leaving no party does nothing.
    a.send(CMSG_GROUP_DISBAND {}).await;
    assert_eq!(told(&mut a).await, None);

    joins(&mut a, &mut b, "Bob").await;
    joins(&mut a, &mut c, "Cid").await;
    // A member takes no one out, makes no leader, sets no loot.
    b.clear();
    b.send(CMSG_GROUP_UNINVITE { name: "Cid".into() }).await;
    assert_eq!(told(&mut b).await, Some(PartyResult::NotLeader));
    b.send(CMSG_GROUP_SET_LEADER { guid: gb }).await;
    b.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::FreeForAll,
        loot_master: Guid::zero(),
        loot_threshold: ItemQuality::Epic,
    })
    .await;
    handled(&mut b).await;
    let rules = {
        let groups = realm.social.groups.lock();
        let gr = groups.group_of(ga).unwrap();
        (gr.leader, gr.loot_method, gr.threshold)
    };
    assert_eq!(
        rules,
        (ga, GroupLootSetting::GroupLoot, ItemQuality::Uncommon)
    );
    // The leader may.
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::MasterLoot,
        loot_master: ga,
        loot_threshold: ItemQuality::Rare,
    })
    .await;
    handled(&mut a).await;
    let rules = {
        let groups = realm.social.groups.lock();
        let gr = groups.group_of(ga).unwrap();
        (gr.loot_method, gr.looter, gr.threshold)
    };
    assert_eq!(rules, (GroupLootSetting::MasterLoot, ga, ItemQuality::Rare));

    // One not in the party isn't taken out; an invitation is taken back.
    a.clear();
    a.send(CMSG_GROUP_UNINVITE { name: "Dee".into() }).await;
    assert_eq!(told(&mut a).await, Some(PartyResult::TargetNotInGroup));
    assert_eq!(invite(&mut a, "Dee").await, Some(PartyResult::Success));
    invited(&mut d).await;
    a.send(CMSG_GROUP_UNINVITE { name: "Dee".into() }).await;
    handled(&mut a).await;
    assert_eq!(group(&realm, ga).unwrap().2, Vec::<Guid>::new());
    d.send(CMSG_GROUP_ACCEPT {}).await;
    handled(&mut d).await;
    assert!(group(&realm, gd).is_none(), "the invitation taken back");

    // A member taken out of three: told, the rest stay.
    a.send(CMSG_GROUP_UNINVITE { name: "Cid".into() }).await;
    c.wait_for("SMSG_GROUP_UNINVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_UNINVITE).then_some(())
    })
    .await;
    assert_eq!(group(&realm, ga).unwrap().0, vec![ga, gb]);
    assert!(group(&realm, gc).is_none());

    // A member leaves three: the rest stay, the leader too.
    joins(&mut a, &mut c, "Cid").await;
    b.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut b).await;
    assert_eq!(group(&realm, ga).unwrap().0, vec![ga, gc]);
    assert!(group(&realm, gb).is_none());
    // The leader leaves three: the next leads.
    joins(&mut a, &mut b, "Bob").await;
    c.clear();
    a.send(CMSG_GROUP_DISBAND {}).await;
    let name = c
        .wait_for("SMSG_GROUP_SET_LEADER", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_SET_LEADER(l) => Some(l.name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(name, "Cid");
    let (members, leader, _) = group(&realm, gc).unwrap();
    assert_eq!((members, leader), (vec![gc, gb], gc));
}

/// Thistle Tea's `same_team?/2`: the Alliance's races group together, the
/// Horde's together, and one side doesn't invite the other.
#[tokio::test]
async fn a_party_is_of_one_side() {
    let realm = realm();
    let mut human = player(&realm, "Alice", Race::Human).await;
    let mut gnome = player(&realm, "Gnoma", Race::Gnome).await;
    let mut orc = player(&realm, "Orcy", Race::Orc).await;
    let mut troll = player(&realm, "Trolly", Race::Troll).await;
    joins(&mut human, &mut gnome, "Gnoma").await;
    joins(&mut orc, &mut troll, "Trolly").await;
    let tauren = player(&realm, "Moo", Race::Tauren).await;
    assert_eq!(
        invite(&mut human, "Moo").await,
        Some(PartyResult::PlayerWrongFaction)
    );
    assert!(group(&realm, tauren.guid).is_none());
}

/// Thistle Tea's `update_looter/3`: round robin hands each corpse to the
/// next member near it, after the last looter, passing over one away.
#[tokio::test]
async fn round_robin_passes_over_a_member_away() {
    let realm = harness::realm_with(data());
    let mut a = player(&realm, "Alice", Race::Human).await;
    let mut b = player(&realm, "Bob", Race::Human).await;
    let mut c = player(&realm, "Cid", Race::Human).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    joins(&mut a, &mut b, "Bob").await;
    joins(&mut a, &mut c, "Cid").await;
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::RoundRobin,
        loot_master: Guid::zero(),
        loot_threshold: ItemQuality::Uncommon,
    })
    .await;
    handled(&mut a).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let kill = |n: u32| {
        let map = map.clone();
        async move {
            map.query(move |m| {
                let k = guid::unit(RABBIT, n);
                crate::game::map::combat::kill(m, ga, k);
                crate::game::map::loot::loot_ref(m, k).and_then(|l| l.assigned)
            })
            .await
            .unwrap()
        }
    };
    assert_eq!(kill(1).await, Some(ga));
    assert_eq!(kill(2).await, Some(gb));
    // Bob away: after him, Cid.
    map.query(move |m| {
        let p = m.players.get_mut(&gb).unwrap();
        p.movement.position.x += 500.0;
    })
    .await
    .unwrap();
    assert_eq!(kill(3).await, Some(gc));
    assert_eq!(kill(4).await, Some(ga), "round again, Bob passed over");
}
