//! Round 12's groups, as mangos-classic keeps them: saved and back after
//! a restart (its `groups` and `group_member` tables, R4-19); a leaving
//! leader's lead to a member online, a raid's assistant first
//! (`Group::_chooseLeader`); a leader offline five minutes replaced
//! (`Group::UpdateOfflineLeader`); a loot threshold no lower than
//! Uncommon.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};

fn realm_on(db: &std::path::Path) -> Arc<Realm> {
    let (store, startup) = crate::game::store::Store::open(db).unwrap();
    Realm::new(harness::test_config(), store, startup, Default::default(), None)
}

/// A player named `name`, on the account of that name in capitals (as the
/// store keeps an account's name).
async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, &name.to_uppercase());
    c.enter(name, Race::Human, Class::Warrior).await;
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

async fn joins(leader: &mut Client, c: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    c.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    c.send(CMSG_GROUP_ACCEPT {}).await;
    c.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
}

/// A party led by the first of `cs`, the rest invited in order.
async fn party(cs: &mut [Client]) {
    let (leader, rest) = cs.split_first_mut().unwrap();
    for c in rest {
        let name = realm_name(&c.account);
        joins(leader, c, &name).await;
    }
    handled(leader).await;
}

/// A character's name from its account's: "ALICE" is Alice.
fn realm_name(account: &str) -> String {
    let lower = account.to_lowercase();
    let mut cs = lower.chars();
    cs.next()
        .map(|f| f.to_uppercase().chain(cs).collect())
        .unwrap_or_default()
}

fn leader_of(realm: &Arc<Realm>, g: Guid) -> Option<Guid> {
    realm.social.groups.lock().group_of(g).map(|gr| gr.leader)
}

async fn gone(realm: &Arc<Realm>, c: Client) {
    let g = c.guid;
    c.disconnect();
    harness::offline(realm, g).await;
}

/// R4-19, as mangos's `groups` and `group_member`: a raid is kept with its
/// leader, members, subgroups, assistant, loot rules and icons, and is
/// there again after a restart; its leader, back, is sent it. Before, a
/// restart dissolved every group.
#[tokio::test]
async fn a_group_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("wvs-12j-groups-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("characters.sqlite");
    let realm = realm_on(&db);
    let mut cs = vec![
        player(&realm, "Alice").await,
        player(&realm, "Bob").await,
        player(&realm, "Carol").await,
    ];
    party(&mut cs).await;
    let (ga, gb, gc) = (cs[0].guid, cs[1].guid, cs[2].guid);
    let a = &mut cs[0];
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    a.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gc,
        set_assistant: true,
    })
    .await;
    a.send(CMSG_GROUP_CHANGE_SUB_GROUP {
        name: "Bob".into(),
        group_number: 3,
    })
    .await;
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::MasterLoot,
        loot_master: gb,
        loot_threshold: ItemQuality::Rare,
    })
    .await;
    a.send(MSG_RAID_TARGET_UPDATE_Client {
        target_index: MSG_RAID_TARGET_UPDATE_Client_RaidTargetIndex::Unknown7 { target: gb },
    })
    .await;
    handled(a).await;
    realm.shutdown().await;
    drop(cs);
    drop(realm);

    let realm = realm_on(&db);
    {
        let groups = realm.social.groups.lock();
        let gr = groups.group_of(ga).expect("the group is back");
        assert!(gr.raid);
        assert_eq!(gr.leader, ga);
        let members: Vec<(Guid, &str, u8, bool)> = gr
            .members
            .iter()
            .map(|m| (m.guid, &*m.name, m.subgroup, m.assistant))
            .collect();
        assert_eq!(
            members,
            vec![
                (ga, "Alice", 0, false),
                (gb, "Bob", 3, false),
                (gc, "Carol", 0, true)
            ]
        );
        assert_eq!(
            (gr.loot_method, gr.looter, gr.threshold),
            (GroupLootSetting::MasterLoot, gb, ItemQuality::Rare)
        );
        assert_eq!(gr.icons[7], gb, "the skull");
        assert_eq!(groups.group_of(gb).map(|g| g.id), Some(gr.id));
    }
    let mut a = Client::connect(&realm, "ALICE");
    a.login(ga).await;
    let listed = a
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some((
                l.group_type,
                l.leader,
                l.members.iter().map(|m| m.guid).collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .await;
    assert_eq!(listed, (GroupType::Raid, ga, vec![gb, gc]));
    // A new group's id is past the kept one's.
    let mut d = player(&realm, "Dave").await;
    let mut e = player(&realm, "Erin").await;
    joins(&mut d, &mut e, "Erin").await;
    let ids = {
        let groups = realm.social.groups.lock();
        (groups.group_of(ga).unwrap().id, groups.group_of(d.guid).unwrap().id)
    };
    assert!(ids.1 > ids.0, "{ids:?}");
    realm.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// A group disbanded is gone from the store: it isn't back after a
/// restart.
#[tokio::test]
async fn a_disbanded_group_stays_gone_after_a_restart() {
    let dir = std::env::temp_dir().join(format!("wvs-12j-disband-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("characters.sqlite");
    let realm = realm_on(&db);
    let mut cs = vec![player(&realm, "Alice").await, player(&realm, "Bob").await];
    party(&mut cs).await;
    let ga = cs[0].guid;
    cs[1].send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut cs[1]).await;
    assert!(leader_of(&realm, ga).is_none());
    realm.shutdown().await;
    drop(cs);
    drop(realm);
    let realm = realm_on(&db);
    assert!(leader_of(&realm, ga).is_none(), "no group is back");
    realm.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// mangos's `_chooseLeader`: a leader leaving hands the lead to a member
/// online, not to the first member left (offline); in a raid, to an
/// assistant online before the rest.
#[tokio::test]
async fn a_leaving_leader_hands_the_lead_to_one_online() {
    let realm = harness::realm();
    let mut cs = vec![
        player(&realm, "Alice").await,
        player(&realm, "Bob").await,
        player(&realm, "Carol").await,
    ];
    party(&mut cs).await;
    let gc = cs[2].guid;
    let bob = cs.remove(1);
    gone(&realm, bob).await;
    let a = &mut cs[0];
    a.send(CMSG_GROUP_DISBAND {}).await;
    handled(a).await;
    assert_eq!(leader_of(&realm, gc), Some(gc), "Carol, online, leads");

    let mut cs = vec![
        player(&realm, "Dave").await,
        player(&realm, "Erin").await,
        player(&realm, "Fred").await,
        player(&realm, "Gina").await,
    ];
    party(&mut cs).await;
    let gg = cs[3].guid;
    let d = &mut cs[0];
    d.send(CMSG_GROUP_RAID_CONVERT {}).await;
    d.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gg,
        set_assistant: true,
    })
    .await;
    d.send(CMSG_GROUP_DISBAND {}).await;
    handled(d).await;
    assert_eq!(leader_of(&realm, gg), Some(gg), "the assistant leads the raid");
}

/// mangos's `Group::UpdateOfflineLeader`: a leader offline
/// `Group.OfflineLeaderDelay` (300 s) hands the lead to a member online,
/// and everyone is told; not before, and not while no one is online.
#[tokio::test]
async fn an_offline_leader_is_replaced_after_five_minutes() {
    use crate::game::social::groups::replace_offline_leaders;
    let realm = harness::realm();
    let mut cs = vec![
        player(&realm, "Alice").await,
        player(&realm, "Bob").await,
        player(&realm, "Carol").await,
    ];
    party(&mut cs).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    let alice = cs.remove(0);
    gone(&realm, alice).await;
    let now = chrono::Utc::now().timestamp();
    replace_offline_leaders(&realm, now + 290);
    assert_eq!(leader_of(&realm, gb), Some(ga), "still Alice's, at 290 s");
    cs[0].clear();
    replace_offline_leaders(&realm, now + 301);
    assert_eq!(leader_of(&realm, gb), Some(gb), "Bob's, past 300 s");
    let told = cs[0]
        .wait_for("SMSG_GROUP_SET_LEADER", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_SET_LEADER(l) => Some(l.name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(told, "Bob");
    // No one online: the lead stays where it is.
    for c in cs.drain(..) {
        gone(&realm, c).await;
    }
    replace_offline_leaders(&realm, now + 10_000);
    assert_eq!(leader_of(&realm, gb), Some(gb));
}

/// A loot threshold below Uncommon, which 1.12's client doesn't offer, is
/// taken as Uncommon: a Poor one would roll on every grey.
#[tokio::test]
async fn a_loot_threshold_is_uncommon_at_least() {
    let realm = harness::realm();
    let mut cs = vec![player(&realm, "Alice").await, player(&realm, "Bob").await];
    party(&mut cs).await;
    let ga = cs[0].guid;
    for (asked, kept) in [
        (ItemQuality::Poor, ItemQuality::Uncommon),
        (ItemQuality::Epic, ItemQuality::Epic),
    ] {
        cs[0]
            .send(CMSG_LOOT_METHOD {
                loot_setting: GroupLootSetting::GroupLoot,
                loot_master: Guid::zero(),
                loot_threshold: asked,
            })
            .await;
        handled(&mut cs[0]).await;
        let t = realm.social.groups.lock().group_of(ga).map(|g| g.threshold);
        assert_eq!(t, Some(kept), "{asked:?}");
    }
    let _ = Duration::ZERO;
}

const DUNGEON: u32 = 36;

/// Send `c` into `map`'s copy (or the open world), and wait for it to land.
async fn go(realm: &Arc<Realm>, c: &mut Client, map: u32) {
    use crate::game::map::MapKey;
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
}

/// mangos's `HandleGroupInviteOpcode`: two players in different copies of
/// one dungeon can't group (`ERR_ALREADY_IN_GROUP_S`); one out of it can
/// be invited.
#[tokio::test]
async fn no_invite_across_two_copies_of_a_dungeon() {
    let mut d = crate::game::data::GameData::empty();
    d.maps.insert(
        DUNGEON,
        crate::game::data::MapInfo {
            id: DUNGEON,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, d, None);
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    go(&realm, &mut a, DUNGEON).await;
    go(&realm, &mut b, DUNGEON).await;
    let key = |c: &Client| realm.registry.online(c.guid).unwrap().map().key;
    assert_ne!(key(&a), key(&b), "two copies");
    invite_bob(&mut a).await;
    let told = a
        .wait_for("SMSG_PARTY_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_PARTY_COMMAND_RESULT(r) => Some(r.result),
            _ => None,
        })
        .await;
    assert_eq!(told, PartyResult::AlreadyInGroup);
    go(&realm, &mut b, 0).await;
    invite_bob(&mut a).await;
    let told = a
        .wait_for("SMSG_PARTY_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_PARTY_COMMAND_RESULT(r) => Some(r.result),
            _ => None,
        })
        .await;
    assert_eq!(told, PartyResult::Success);
}

async fn invite_bob(c: &mut Client) {
    c.clear();
    c.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
}

/// mangos's `HandleRequestPartyMemberStatsOpcode` and
/// `GetGroupMemberStatus`: a member's stats carry their pet (its GUID,
/// name, model, health, power and auras, `GROUP_UPDATE_FLAG_PET_*`) and
/// their PvP flag (`MEMBER_STATUS_PVP`). Before, they stopped at the
/// player's own, and the PvP icon never showed.
#[tokio::test]
async fn member_stats_carry_the_pet_and_the_pvp_flag() {
    use crate::game::social::groups::{
        SMSG_PARTY_MEMBER_STATS_FULL, STAT_PET, STAT_PLAYER, STATUS_PVP,
    };
    let realm = harness::realm_with(crate::game::tests_pets::data());
    let (mut h, gh, map) =
        crate::game::tests_pets::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    let mut b = player(&realm, "Bob").await;
    joins(&mut h, &mut b, "Bob").await;
    h.say(".pet").await;
    h.system_message("Tamed").await;
    // The hunter flags themselves for PvP, as /pvp does.
    h.send(CMSG_TOGGLE_PVP { set: None }).await;
    handled(&mut h).await;
    let flagged = map
        .query(move |m| {
            m.players[&gh].fields.has_flag(
                crate::game::fields::UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
            )
        })
        .await
        .unwrap();
    assert!(flagged, "the hunter is flagged");
    let pet = map
        .query(move |m| m.players[&gh].owned.pet)
        .await
        .unwrap()
        .expect("the boar out");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (mask, status, body) = loop {
        b.clear();
        b.send(CMSG_REQUEST_PARTY_MEMBER_STATS { guid: gh }).await;
        let f = b
            .wait_frame("the full stats", SMSG_PARTY_MEMBER_STATS_FULL, |_| true)
            .await;
        let at = 1 + f[0].count_ones() as usize;
        let mask = u32::from_le_bytes(f[at..at + 4].try_into().unwrap());
        let status = f[at + 4];
        if mask & STAT_PET == STAT_PET && status & STATUS_PVP != 0 {
            break (mask, status, f);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no pet nor PvP in {mask:#x}, {status:#x}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(mask, STAT_PLAYER | STAT_PET);
    assert_ne!(status & STATUS_PVP, 0);
    let pet_guid = pet.guid().to_le_bytes();
    let at = body
        .windows(8)
        .position(|w| w == pet_guid)
        .expect("the pet's GUID");
    assert!(
        body[at + 8..].starts_with(b"Tame Boar\0"),
        "its name after its GUID"
    );
}
