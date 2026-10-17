//! Phase 3's guilds, Thistle Tea's `GuildTest` ported: founding one,
//! inviting and its refusals, handing the lead over, editing, adding and
//! deleting ranks, the tabard's emblem, and a guild of one left.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;

async fn player(realm: &Arc<Realm>, name: &str, race: Race) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, race, Class::Warrior).await;
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

/// What `c` was told of a guild command since it last looked.
async fn told(c: &mut Client) -> Option<GuildCommandResult> {
    handled(c).await;
    let r = c
        .gets(Duration::from_millis(20), |m| match m {
            ServerOpcodeMessage::SMSG_GUILD_COMMAND_RESULT(r) => Some(r.result),
            _ => None,
        })
        .await;
    c.clear();
    r
}

async fn create(c: &mut Client, name: &str) -> Option<GuildCommandResult> {
    c.clear();
    c.send(CMSG_GUILD_CREATE {
        guild_name: name.into(),
    })
    .await;
    told(c).await
}

async fn invite(c: &mut Client, name: &str) -> Option<GuildCommandResult> {
    c.clear();
    c.send(CMSG_GUILD_INVITE {
        invited_player: name.into(),
    })
    .await;
    told(c).await
}

async fn accept(c: &mut Client) {
    c.send(CMSG_GUILD_ACCEPT {}).await;
    handled(c).await;
}

/// `g`'s guild: its name, leader, ranks' names and rights, and each
/// member's rank.
#[allow(clippy::type_complexity)]
fn guild(
    realm: &Arc<Realm>,
    g: Guid,
) -> Option<(String, u32, Vec<(String, u32)>, Vec<(u32, u32)>)> {
    let guilds = realm.social.guilds.lock();
    guilds.of(g).map(|gr| {
        (
            gr.name.clone(),
            gr.leader,
            gr.ranks
                .iter()
                .map(|r| (r.name.clone(), r.rights))
                .collect(),
            gr.members.iter().map(|m| (m.guid, m.rank)).collect(),
        )
    })
}

fn rank_of(realm: &Arc<Realm>, g: Guid) -> Option<u32> {
    guild(realm, g).and_then(|(_, _, _, members)| {
        members
            .iter()
            .find(|(m, _)| *m == crate::game::guid::counter(g))
            .map(|(_, r)| *r)
    })
}

/// Thistle Tea's `create/3` and `invite/3`: a guild's name its own,
/// whatever its case, one guild a founder, no name of a letter; no
/// invitation across the sides, nor twice, nor from a rank without the
/// right, nor to one in a guild; the invited at the lowest rank, and
/// accepting with no invitation doing nothing.
#[tokio::test]
async fn a_guild_is_founded_and_invites_by_its_rules() {
    let realm = harness::realm();
    let mut founder = player(&realm, "Founder", Race::Human).await;
    let mut other = player(&realm, "Other", Race::Human).await;
    let mut ally = player(&realm, "Ally", Race::Dwarf).await;
    let mut third = player(&realm, "Third", Race::Human).await;
    let _enemy = player(&realm, "Enemy", Race::Orc).await;
    assert_eq!(create(&mut founder, "The Fellowship").await, None);
    let (name, leader, ranks, members) = guild(&realm, founder.guid).unwrap();
    assert_eq!(
        (name.as_str(), leader, ranks.len()),
        (
            "The Fellowship",
            crate::game::guid::counter(founder.guid),
            5
        )
    );
    assert_eq!(members, vec![(crate::game::guid::counter(founder.guid), 0)]);
    assert_eq!(
        create(&mut other, "THE FELLOWSHIP").await,
        Some(GuildCommandResult::GuildNameExistsS)
    );
    assert_eq!(
        create(&mut founder, "Another Guild").await,
        Some(GuildCommandResult::AlreadyInGuild)
    );
    assert_eq!(
        create(&mut other, "X").await,
        Some(GuildCommandResult::GuildNameInvalid)
    );

    assert_eq!(
        invite(&mut founder, "Enemy").await,
        Some(GuildCommandResult::GuildNotAllied)
    );
    assert_eq!(
        invite(&mut founder, "Ally").await,
        Some(GuildCommandResult::PlayerNoMoreInGuild),
        "invited (result 0)"
    );
    assert_eq!(
        invite(&mut founder, "Ally").await,
        Some(GuildCommandResult::AlreadyInvitedToGuildS)
    );
    accept(&mut ally).await;
    assert_eq!(rank_of(&realm, ally.guid), Some(4));
    // The invitation gone: inviting again tells them in the guild.
    assert_eq!(
        invite(&mut ally, "Third").await,
        Some(GuildCommandResult::GuildPermissionsOrLeader)
    );
    assert_eq!(
        invite(&mut founder, "Ally").await,
        Some(GuildCommandResult::AlreadyInGuildS)
    );
    accept(&mut third).await;
    assert!(guild(&realm, third.guid).is_none(), "no invitation");
}

/// Thistle Tea's `set_leader/3`: the leader of others doesn't leave; a
/// member makes no leader and takes the leader out of nothing; the lead
/// handed over, the new leader at the top, the old one an officer, who
/// may then leave.
#[tokio::test]
async fn the_lead_is_handed_over_and_the_old_leader_may_go() {
    let realm = harness::realm();
    let mut founder = player(&realm, "Founder", Race::Human).await;
    let mut ally = player(&realm, "Ally", Race::Human).await;
    create(&mut founder, "Fellowship").await;
    invite(&mut founder, "Ally").await;
    accept(&mut ally).await;
    founder.send(CMSG_GUILD_LEAVE {}).await;
    assert_eq!(
        told(&mut founder).await,
        Some(GuildCommandResult::GuildPermissionsOrLeader)
    );
    ally.send(CMSG_GUILD_LEADER {
        new_guild_leader_name: "Ally".into(),
    })
    .await;
    assert_eq!(
        told(&mut ally).await,
        Some(GuildCommandResult::GuildPermissionsOrLeader)
    );
    ally.send(CMSG_GUILD_REMOVE {
        player_name: "Founder".into(),
    })
    .await;
    assert_eq!(
        told(&mut ally).await,
        Some(GuildCommandResult::GuildPermissionsOrLeader)
    );
    founder
        .send(CMSG_GUILD_LEADER {
            new_guild_leader_name: "Ally".into(),
        })
        .await;
    handled(&mut founder).await;
    let (_, leader, _, _) = guild(&realm, ally.guid).unwrap();
    assert_eq!(leader, crate::game::guid::counter(ally.guid));
    assert_eq!(
        (rank_of(&realm, ally.guid), rank_of(&realm, founder.guid)),
        (Some(0), Some(1))
    );
    ally.send(CMSG_GUILD_LEAVE {}).await;
    assert_eq!(
        told(&mut ally).await,
        Some(GuildCommandResult::GuildPermissionsOrLeader)
    );
    founder.send(CMSG_GUILD_LEAVE {}).await;
    handled(&mut founder).await;
    let (_, _, _, members) = guild(&realm, ally.guid).unwrap();
    assert_eq!(members, vec![(crate::game::guid::counter(ally.guid), 0)]);
    assert!(guild(&realm, founder.guid).is_none());
}

/// Thistle Tea's `edit_rank/5`: the leader alone adds a rank; the
/// leader's own renamed keeps every right; another's rights set; a new
/// member at the new lowest rank, moved up a rank when it is deleted; five
/// ranks the fewest.
#[tokio::test]
async fn ranks_are_the_leaders_to_edit_add_and_delete() {
    let realm = harness::realm();
    let mut founder = player(&realm, "Founder", Race::Human).await;
    let mut ally = player(&realm, "Ally", Race::Human).await;
    let mut third = player(&realm, "Third", Race::Human).await;
    create(&mut founder, "Fellowship").await;
    invite(&mut founder, "Ally").await;
    accept(&mut ally).await;
    ally.send(CMSG_GUILD_ADD_RANK {
        rank_name: "Scout".into(),
    })
    .await;
    assert_eq!(
        told(&mut ally).await,
        Some(GuildCommandResult::GuildPermissionsOrLeader)
    );
    founder
        .send(CMSG_GUILD_ADD_RANK {
            rank_name: "Scout".into(),
        })
        .await;
    founder
        .send(CMSG_GUILD_RANK {
            rank_id: 0,
            rights: 0,
            rank_name: "Captain".into(),
        })
        .await;
    founder
        .send(CMSG_GUILD_RANK {
            rank_id: 5,
            rights: 0x50,
            rank_name: "Recruit".into(),
        })
        .await;
    handled(&mut founder).await;
    let (_, _, ranks, _) = guild(&realm, founder.guid).unwrap();
    assert_eq!(ranks.len(), 6);
    assert_eq!(
        ranks[0],
        (
            "Captain".to_owned(),
            crate::game::social::guilds::RIGHT_ALL | crate::game::social::guilds::RIGHT_EMPTY
        )
    );
    assert_eq!(ranks[5], ("Recruit".to_owned(), 0x50));

    invite(&mut founder, "Third").await;
    accept(&mut third).await;
    assert_eq!(rank_of(&realm, third.guid), Some(5));
    founder.send(CMSG_GUILD_DEL_RANK {}).await;
    handled(&mut founder).await;
    let (_, _, ranks, _) = guild(&realm, founder.guid).unwrap();
    assert_eq!(ranks.len(), 5);
    assert_eq!(rank_of(&realm, third.guid), Some(4), "moved up a rank");
    // Its fields too.
    let g = third.guid;
    let field = realm
        .maps
        .get(&realm, MapKey::world(0))
        .query(move |m| {
            m.players[&g]
                .fields
                .get_u32(crate::game::fields::PLAYER_GUILDRANK)
        })
        .await
        .unwrap();
    assert_eq!(field, 4);
    founder.send(CMSG_GUILD_DEL_RANK {}).await;
    handled(&mut founder).await;
    assert_eq!(
        guild(&realm, founder.guid).unwrap().2.len(),
        5,
        "the fewest"
    );
}

/// Thistle Tea's `set_emblem/3`: the leader alone sets the tabard's
/// emblem, for every member; and `disband/2`: the last member leaving ends
/// the guild and frees its name.
#[tokio::test]
async fn the_leader_sets_the_emblem_and_the_last_to_leave_ends_it() {
    let realm = harness::realm();
    let mut founder = player(&realm, "Founder", Race::Human).await;
    let mut ally = player(&realm, "Ally", Race::Human).await;
    create(&mut founder, "Fellowship").await;
    invite(&mut founder, "Ally").await;
    accept(&mut ally).await;
    for c in [&founder, &ally] {
        let g = c.guid;
        realm
            .maps
            .get(&realm, MapKey::world(0))
            .query(move |m| m.players.get_mut(&g).unwrap().set_money(1_000_000))
            .await
            .unwrap();
    }
    let emblem = MSG_SAVE_GUILD_EMBLEM_Client {
        vendor: Guid::zero(),
        emblem_style: 3,
        emblem_color: 4,
        border_style: 5,
        border_color: 6,
        background_color: 7,
    };
    let saved = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::MSG_SAVE_GUILD_EMBLEM(r) => Some(r.result),
        _ => None,
    };
    ally.send(emblem.clone()).await;
    assert_eq!(
        ally.wait_for("the emblem refused", saved).await,
        GuildEmblemResult::NotGuildMaster
    );
    founder.send(emblem).await;
    assert_eq!(
        founder.wait_for("the emblem saved", saved).await,
        GuildEmblemResult::Success
    );
    let style = {
        let guilds = realm.social.guilds.lock();
        let e = &guilds.of(ally.guid).unwrap().emblem;
        (
            e.style,
            e.color,
            e.border_style,
            e.border_color,
            e.background,
        )
    };
    assert_eq!(style, (3, 4, 5, 6, 7));

    // The last to leave ends it, and its name is free.
    let mut solo = player(&realm, "Solo", Race::Human).await;
    create(&mut solo, "Lonely Hearts").await;
    assert!(guild(&realm, solo.guid).is_some());
    solo.send(CMSG_GUILD_LEAVE {}).await;
    handled(&mut solo).await;
    assert!(guild(&realm, solo.guid).is_none());
    assert!(!realm.social.guilds.lock().name_taken("Lonely Hearts"));
}
