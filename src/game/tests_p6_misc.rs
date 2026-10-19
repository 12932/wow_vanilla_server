//! Round 6's remaining tests, ported from Thistle Tea's
//! `test/db/mangos/addon_auras_test.exs`, `test/game/core/party/raid_test.exs`
//! and `test/game/core/guild/petitions_test.exs` (docs/parity/matrix.csv):
//! a creature addon's auras as read, a raid's assistants, invitations and
//! leaders, and a guild charter's one owner, name and signatures.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::social::guilds::CHARTER_ITEM;
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::handled;

/// Thistle Tea's `AddonAuras.parse`: a creature addon's auras are spell
/// ids apart by spaces; a word, a negative and 0 are none (mangos's
/// `ConvertCreatureAddonAuras` finds no such spell), and nothing is none.
#[test]
fn a_creature_addons_auras_are_its_spell_ids() {
    use crate::game::data::load::parse_ids;
    assert_eq!(parse_ids("12544"), [12_544]);
    assert_eq!(parse_ids("8279 6408"), [8_279, 6_408]);
    assert_eq!(parse_ids("  8279   abc 0 -5 6408 "), [8_279, 6_408]);
    assert!(parse_ids("").is_empty());
}

/// A realm with no GMs (who group across sides), its charters wanting
/// `signatures`.
fn realm(signatures: u32) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.petition_signatures = signatures;
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

async fn player(realm: &Arc<Realm>, name: &str, race: Race) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, race, Class::Warrior).await;
    c.clear();
    c
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

/// `c` takes the invitation it has, and is in.
async fn accepts(c: &mut Client) {
    invited(c).await;
    c.send(CMSG_GROUP_ACCEPT {}).await;
    c.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
}

async fn joins(by: &mut Client, c: &mut Client, name: &str) {
    assert_eq!(invite(by, name).await, Some(PartyResult::Success));
    accepts(c).await;
}

/// `g`'s group: whether a raid, its members, its leader, its assistants.
fn group(realm: &Arc<Realm>, g: Guid) -> Option<(bool, Vec<Guid>, Guid, Vec<Guid>)> {
    let groups = realm.social.groups.lock();
    groups.group_of(g).map(|gr| {
        (
            gr.raid,
            gr.members.iter().map(|m| m.guid).collect(),
            gr.leader,
            gr.members
                .iter()
                .filter(|m| m.assistant)
                .map(|m| m.guid)
                .collect(),
        )
    })
}

/// Alice's raid of Alice and Bob.
async fn raid(realm: &Arc<Realm>) -> (Client, Client) {
    let mut a = player(realm, "Alice", Race::Human).await;
    let mut b = player(realm, "Bob", Race::Human).await;
    joins(&mut a, &mut b, "Bob").await;
    a.send(CMSG_GROUP_RAID_CONVERT {}).await;
    b.wait_for("the raid's roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (l.group_type == GroupType::Raid).then_some(()),
        _ => None,
    })
    .await;
    (a, b)
}

/// Thistle Tea's `Party.set_assistant` "grants management while keeping
/// leadership and loot restricted": a raid's assistant (flag 0x80) invites
/// and takes an ordinary member out, but not the leader (not the leader),
/// and makes no leader, no assistant and no loot rule; demoted, they
/// invite no one.
#[tokio::test]
async fn a_raids_assistant_invites_and_takes_out_but_doesnt_lead() {
    let realm = realm(1);
    let (mut a, mut b) = raid(&realm).await;
    let mut c = player(&realm, "Cid", Race::Dwarf).await;
    let mut d = player(&realm, "Dee", Race::Gnome).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    b.clear();
    a.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gb,
        set_assistant: true,
    })
    .await;
    let flags = b
        .wait_for("Bob's flags", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => Some(l.flags),
            _ => None,
        })
        .await;
    assert_eq!(flags, 0x80, "an assistant");
    // Bob invites, and Cid comes in.
    joins(&mut b, &mut c, "Cid").await;
    // Bob makes no assistant, no leader and no loot rule.
    b.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gc,
        set_assistant: true,
    })
    .await;
    b.send(CMSG_GROUP_SET_LEADER { guid: gc }).await;
    b.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::FreeForAll,
        loot_master: Guid::zero(),
        loot_threshold: ItemQuality::Epic,
    })
    .await;
    handled(&mut b).await;
    let (raid, members, leader, assistants) = group(&realm, ga).unwrap();
    assert!(raid);
    assert_eq!(
        (members, leader, assistants),
        (vec![ga, gb, gc], ga, vec![gb])
    );
    let rules = {
        let groups = realm.social.groups.lock();
        let gr = groups.group_of(ga).unwrap();
        (gr.loot_method, gr.threshold)
    };
    assert_eq!(rules, (GroupLootSetting::GroupLoot, ItemQuality::Uncommon));
    // Not the leader out; Cid out.
    b.clear();
    b.send(CMSG_GROUP_UNINVITE_GUID { guid: ga }).await;
    assert_eq!(told(&mut b).await, Some(PartyResult::NotLeader));
    b.send(CMSG_GROUP_UNINVITE_GUID { guid: gc }).await;
    c.wait_for("SMSG_GROUP_UNINVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_UNINVITE).then_some(())
    })
    .await;
    assert!(group(&realm, gc).is_none(), "Cid out");
    assert_eq!(group(&realm, ga).unwrap().1, vec![ga, gb]);
    // Demoted, Bob invites no one.
    a.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gb,
        set_assistant: false,
    })
    .await;
    handled(&mut a).await;
    assert_eq!(
        invite(&mut b, "Dee").await,
        Some(PartyResult::NotLeader),
        "demoted"
    );
    assert!(
        d.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await
        .is_none()
    );
}

/// Thistle Tea's `Party.leave` "pending invitations stay with the original
/// raid and disappear on disband": one with an invitation pending invites
/// no one (mangos's `HandleGroupInviteOpcode` takes the group they are
/// invited to, not theirs to lead: not the leader; Thistle Tea answers
/// already in a group); an invitation the assistant gave outlives their
/// leaving, into the raid as its leader has it; and a raid disbanded takes
/// its invitations with it.
#[tokio::test]
async fn an_invitation_is_the_raids_and_goes_with_it() {
    let realm = realm(1);
    let (mut a, mut b) = raid(&realm).await;
    let mut c = player(&realm, "Cid", Race::Dwarf).await;
    let mut d = player(&realm, "Dee", Race::Gnome).await;
    let mut e = player(&realm, "Eve", Race::NightElf).await;
    let (ga, gb, gc, gd, ge) = (a.guid, b.guid, c.guid, d.guid, e.guid);
    joins(&mut a, &mut c, "Cid").await;
    a.send(CMSG_GROUP_ASSISTANT_LEADER {
        guid: gb,
        set_assistant: true,
    })
    .await;
    handled(&mut a).await;
    assert_eq!(invite(&mut b, "Dee").await, Some(PartyResult::Success));
    invited(&mut d).await;
    assert_eq!(
        invite(&mut d, "Eve").await,
        Some(PartyResult::NotLeader),
        "invited, not leading"
    );
    assert!(
        realm.social.groups.lock().group_of(gd).is_none(),
        "no group of Dee's own"
    );
    // Bob leaves; Dee's invitation is the raid's still.
    b.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut b).await;
    assert!(group(&realm, gb).is_none());
    d.send(CMSG_GROUP_ACCEPT {}).await;
    d.wait_for("the raid's roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    let (raid, members, leader, _) = group(&realm, gd).unwrap();
    assert!(raid);
    assert_eq!((members, leader), (vec![ga, gc, gd], ga));
    // Eve invited; the raid goes, and her invitation with it.
    assert_eq!(invite(&mut a, "Eve").await, Some(PartyResult::Success));
    invited(&mut e).await;
    c.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut c).await;
    d.send(CMSG_GROUP_DISBAND {}).await;
    a.wait_for("SMSG_GROUP_DESTROYED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_DESTROYED).then_some(())
    })
    .await;
    e.send(CMSG_GROUP_ACCEPT {}).await;
    handled(&mut e).await;
    assert!(group(&realm, ge).is_none(), "no invitation left");
    assert!(group(&realm, ga).is_none());
    assert_eq!(
        invite(&mut a, "Eve").await,
        Some(PartyResult::Success),
        "free to be invited again"
    );
}

/// Thistle Tea's `Party.leave` "retains raid mode on leadership transfer
/// and removes the final group": the leader leaving a raid of three hands
/// it on, a raid still; one of the last two leaving ends it for both.
#[tokio::test]
async fn a_raid_stays_one_as_its_leader_goes_and_ends_with_two() {
    let realm = realm(1);
    let (mut a, mut b) = raid(&realm).await;
    let mut c = player(&realm, "Cid", Race::Dwarf).await;
    let (gb, gc) = (b.guid, c.guid);
    joins(&mut a, &mut c, "Cid").await;
    c.clear();
    a.send(CMSG_GROUP_DISBAND {}).await;
    let name = c
        .wait_for("SMSG_GROUP_SET_LEADER", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_SET_LEADER(l) => Some(l.name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(name, "Bob");
    let kind = c
        .wait_for("the roster", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => {
                Some((l.group_type, l.leader))
            }
            _ => None,
        })
        .await;
    assert_eq!(kind, (GroupType::Raid, gb));
    let (raid, members, leader, _) = group(&realm, gb).unwrap();
    assert!(raid, "a raid still");
    assert_eq!((members, leader), (vec![gb, gc], gb));
    c.send(CMSG_GROUP_DISBAND {}).await;
    for x in [&mut b, &mut c] {
        x.wait_for("SMSG_GROUP_DESTROYED", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_DESTROYED).then_some(())
        })
        .await;
    }
    assert!(group(&realm, gb).is_none() && group(&realm, gc).is_none());
}

/// A petitioner beside `g`, seen.
async fn petitioner(c: &mut Client, map: &crate::game::map::MapHandle) -> Guid {
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

fn buy(npc: Guid, name: &str) -> CMSG_PETITION_BUY {
    CMSG_PETITION_BUY {
        npc,
        name: name.into(),
        index: 1,
        ..Default::default()
    }
}

/// The petitions `owner` has, by name.
fn petitions_of(realm: &Realm, owner: Guid) -> Vec<String> {
    let owner = crate::game::guid::counter(owner);
    let mut names: Vec<String> = realm
        .social
        .guilds
        .lock()
        .petitions
        .values()
        .filter(|p| p.owner == owner)
        .map(|p| p.name.clone())
        .collect();
    names.sort();
    names
}

fn charters(m: &crate::game::map::Map, g: Guid) -> Vec<Guid> {
    m.players[&g]
        .items
        .iter()
        .filter(|i| i.entry == CHARTER_ITEM)
        .map(|i| i.guid)
        .collect()
}

/// The next guild command result `c` is told.
async fn guild_result(c: &mut Client) -> (String, GuildCommandResult) {
    c.wait_for("SMSG_GUILD_COMMAND_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_GUILD_COMMAND_RESULT(r) => Some((r.string.clone(), r.result)),
        _ => None,
    })
    .await
}

/// Thistle Tea's `Petitions.create` "keeps one charter per owner and
/// validates the guild name": a name of one letter, or with a digit, buys
/// no charter (invalid); one charter is carried at a time (its limit, the
/// client told); and a new one, the first gone, ends the owner's older
/// petition, whose name is free again (mangos's `HandlePetitionBuyOpcode`
/// deletes the owner's petitions).
#[tokio::test]
async fn a_charter_is_one_an_owner_and_of_a_valid_name() {
    let realm = realm(1);
    let mut a = player(&realm, "Alice", Race::Human).await;
    let ga = a.guid;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let npc = petitioner(&mut a, &map).await;
    for bad in ["X", "Grunts1"] {
        a.clear();
        a.send(buy(npc, bad)).await;
        assert_eq!(
            guild_result(&mut a).await,
            (bad.to_owned(), GuildCommandResult::GuildNameInvalid),
            "{bad}"
        );
    }
    a.send(buy(npc, "Fellowship")).await;
    handled(&mut a).await;
    assert_eq!(on(&map, move |m| charters(m, ga)).await.len(), 1);
    assert_eq!(petitions_of(&realm, ga), ["Fellowship"]);
    // A second while the first is carried: refused, told.
    a.clear();
    a.send(buy(npc, "Another Guild")).await;
    a.wait_frame("SMSG_INVENTORY_CHANGE_FAILURE", 0x112, |_| true)
        .await;
    assert_eq!(on(&map, move |m| charters(m, ga)).await.len(), 1);
    assert_eq!(petitions_of(&realm, ga), ["Fellowship"]);
    // The first destroyed, a new one ends its petition.
    on(&map, move |m| {
        let i = m.players[&ga]
            .items
            .iter()
            .position(|i| i.entry == CHARTER_ITEM)
            .unwrap();
        crate::game::map::inventory::take_item(m, ga, i, 1);
    })
    .await;
    a.send(buy(npc, "Another Guild")).await;
    handled(&mut a).await;
    assert_eq!(on(&map, move |m| charters(m, ga)).await.len(), 1);
    assert_eq!(petitions_of(&realm, ga), ["Another Guild"]);
    assert!(
        !realm.social.guilds.lock().name_taken("Fellowship"),
        "its name free"
    );
}

/// The result of `c`'s signing `charter`: the petition's answer, or the
/// guild command's refusal.
async fn sign(c: &mut Client, charter: Guid) -> Result<PetitionResult, GuildCommandResult> {
    c.clear();
    c.send(CMSG_PETITION_SIGN {
        petition: charter,
        unknown1: 0,
    })
    .await;
    c.wait_for("the signature's answer", |m| match m {
        ServerOpcodeMessage::SMSG_PETITION_SIGN_RESULTS(r) => Some(Ok(r.result)),
        ServerOpcodeMessage::SMSG_GUILD_COMMAND_RESULT(r) => Some(Err(r.result)),
        _ => None,
    })
    .await
}

/// Thistle Tea's `Petitions.sign` "enforces unique accounts, faction, and
/// nine signatures" (wvs's `petition_signatures`, two here): the owner
/// can't sign their own, nor one of the other side (not allied); a
/// signature is one an account, another of the signer's characters
/// refused as signed already; and a charter with its signatures takes no
/// more.
#[tokio::test]
async fn a_charter_is_signed_once_an_account_by_its_side_to_its_count() {
    let realm = realm(2);
    let mut a = player(&realm, "Alice", Race::Human).await;
    let ga = a.guid;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let npc = petitioner(&mut a, &map).await;
    a.send(buy(npc, "Fellowship")).await;
    handled(&mut a).await;
    let charter = on(&map, move |m| charters(m, ga)).await[0];
    assert_eq!(sign(&mut a, charter).await, Ok(PetitionResult::CantSignOwn));
    let mut orc = player(&realm, "Grom", Race::Orc).await;
    assert_eq!(
        sign(&mut orc, charter).await,
        Err(GuildCommandResult::GuildNotAllied)
    );
    let mut b = player(&realm, "Bob", Race::Human).await;
    assert_eq!(sign(&mut b, charter).await, Ok(PetitionResult::Ok));
    assert_eq!(
        sign(&mut b, charter).await,
        Ok(PetitionResult::AlreadySigned)
    );
    // Bob's account's other character.
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    b.enter("Bobby", Race::Human, Class::Warrior).await;
    assert_eq!(
        sign(&mut b, charter).await,
        Ok(PetitionResult::AlreadySigned),
        "one an account"
    );
    let mut c = player(&realm, "Cid", Race::Dwarf).await;
    assert_eq!(sign(&mut c, charter).await, Ok(PetitionResult::Ok));
    let mut d = player(&realm, "Dee", Race::Gnome).await;
    assert_eq!(
        sign(&mut d, charter).await,
        Ok(PetitionResult::AlreadySigned),
        "it has its two"
    );
    let signers = realm
        .social
        .guilds
        .lock()
        .petitions
        .get(&crate::game::guid::counter(charter))
        .map(|p| p.signers.len());
    assert_eq!(signers, Some(2));
}
