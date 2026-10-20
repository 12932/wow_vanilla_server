//! Guilds and their charters, as mangos-zero runs them: ranks and rights,
//! the roster with notes, the MOTD and info, invites, promotions and
//! demotions, leaving and removal, a new leader, disbanding, the emblem,
//! guild and officer chat, and the events members hear. Each guild and
//! charter is kept as a record.

use std::sync::Arc;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::conn::encode;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::registry::{Online, Team};
use crate::game::social::{not_ignoring, send_frames, send_to};
use crate::game::store::Records;

pub const KIND_GUILD: &str = "guild";
pub const KIND_PETITION: &str = "petition";

/// The guild charter, and what it costs.
pub const CHARTER_ITEM: u32 = 5863;
pub const CHARTER_DISPLAY: u32 = 16161;
pub const CHARTER_COST: u32 = 1000;
/// A new emblem's cost: ten gold.
pub const EMBLEM_COST: u32 = 100_000;
pub const MAX_RANKS: usize = 10;
pub const MIN_RANKS: usize = 5;

// Rights.
pub const RIGHT_EMPTY: u32 = 0x0040;
pub const RIGHT_CHAT_LISTEN: u32 = 0x0041;
pub const RIGHT_CHAT_SPEAK: u32 = 0x0042;
pub const RIGHT_OFFICER_LISTEN: u32 = 0x0044;
pub const RIGHT_OFFICER_SPEAK: u32 = 0x0048;
pub const RIGHT_PROMOTE: u32 = 0x00C0;
pub const RIGHT_DEMOTE: u32 = 0x0140;
pub const RIGHT_INVITE: u32 = 0x0050;
pub const RIGHT_REMOVE: u32 = 0x0060;
pub const RIGHT_SET_MOTD: u32 = 0x1040;
pub const RIGHT_EDIT_PUBLIC_NOTE: u32 = 0x2040;
pub const RIGHT_VIEW_OFFICER_NOTE: u32 = 0x4040;
pub const RIGHT_EDIT_OFFICER_NOTE: u32 = 0x8040;
pub const RIGHT_ALL: u32 = 0xF1FF;

fn has(rights: u32, right: u32) -> bool {
    rights & right == right
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rank {
    pub name: String,
    pub rights: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Member {
    /// The character's GUID counter.
    pub guid: u32,
    pub rank: u32,
    pub note: String,
    pub officer_note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Emblem {
    pub style: u32,
    pub color: u32,
    pub border_style: u32,
    pub border_color: u32,
    pub background: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Guild {
    pub id: u32,
    pub name: String,
    pub leader: u32,
    pub motd: String,
    pub info: String,
    pub ranks: Vec<Rank>,
    pub members: Vec<Member>,
    #[serde(default)]
    pub emblem: Emblem,
    /// Day, month (0-based) and year since 2000.
    pub created: (u32, u32, u32),
}

impl Guild {
    fn member(&self, g: Guid) -> Option<&Member> {
        let c = guid::counter(g);
        self.members.iter().find(|m| m.guid == c)
    }

    fn member_mut(&mut self, g: Guid) -> Option<&mut Member> {
        let c = guid::counter(g);
        self.members.iter_mut().find(|m| m.guid == c)
    }

    fn rights(&self, g: Guid) -> u32 {
        self.member(g)
            .and_then(|m| self.ranks.get(m.rank as usize))
            .map_or(0, |r| r.rights)
    }

    fn rank_names(&self) -> [String; 10] {
        std::array::from_fn(|i| {
            self.ranks
                .get(i)
                .map(|r| r.name.clone())
                .unwrap_or_default()
        })
    }

    fn online(&self, realm: &Realm) -> Vec<Arc<Online>> {
        self.members
            .iter()
            .filter_map(|m| realm.registry.online(guid::player(m.guid)))
            .collect()
    }

    fn event(&self, realm: &Realm, event: GuildEvent, strings: &[&str]) {
        let Some(frame) = encode(&SMSG_GUILD_EVENT {
            event,
            event_descriptions: strings.iter().map(|s| s.to_string()).collect(),
        }) else {
            return;
        };
        for o in self.online(realm) {
            o.outbound.send_frame(frame.clone());
        }
    }
}

/// A guild charter: its owner, the guild's name and who has signed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Petition {
    /// The charter item's GUID counter.
    pub id: u32,
    pub owner: u32,
    pub name: String,
    pub signers: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct Guilds {
    pub guilds: AHashMap<u32, Guild>,
    /// Members (by GUID counter) to their guild.
    of: AHashMap<u32, u32>,
    invites: AHashMap<Guid, (u32, Guid)>,
    next_id: u32,
    pub petitions: AHashMap<u32, Petition>,
}

impl Guilds {
    /// The guilds and charters kept in the store.
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut g = Self::default();
        for (kind, id, data) in records {
            let id = *id as u32;
            match kind.as_str() {
                KIND_GUILD => {
                    // Every id counts, read or not (`Mails::load`).
                    g.next_id = g.next_id.max(id);
                    let Some(mut guild) =
                        crate::game::store::parse_record::<Guild>(KIND_GUILD, id.into(), data)
                    else {
                        continue;
                    };
                    guild.id = id;
                    for m in &guild.members {
                        g.of.insert(m.guid, guild.id);
                    }
                    g.guilds.insert(guild.id, guild);
                }
                KIND_PETITION => {
                    if let Some(mut p) =
                        crate::game::store::parse_record::<Petition>(KIND_PETITION, id.into(), data)
                    {
                        p.id = id;
                        g.petitions.insert(id, p);
                    }
                }
                _ => {}
            }
        }
        g
    }

    pub fn of(&self, g: Guid) -> Option<&Guild> {
        self.of
            .get(&guid::counter(g))
            .and_then(|id| self.guilds.get(id))
    }

    fn of_mut(&mut self, g: Guid) -> Option<&mut Guild> {
        let id = *self.of.get(&guid::counter(g))?;
        self.guilds.get_mut(&id)
    }

    pub fn name_taken(&self, name: &str) -> bool {
        self.guilds
            .values()
            .any(|g| g.name.eq_ignore_ascii_case(name))
            || self
                .petitions
                .values()
                .any(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// The guild's name, for `/who`.
    pub fn name_of(&self, g: Guid) -> String {
        self.of(g).map(|g| g.name.clone()).unwrap_or_default()
    }
}

fn save(realm: &Realm, guild: &Guild) {
    realm
        .store
        .put_record(KIND_GUILD, u64::from(guild.id), guild);
}

fn command_result(
    realm: &Realm,
    to: Guid,
    command: GuildCommand,
    string: &str,
    result: GuildCommandResult,
) {
    send_to(
        realm,
        to,
        &SMSG_GUILD_COMMAND_RESULT {
            command,
            string: string.to_owned(),
            result,
        },
    );
}

/// Set a member's guild fields, in whatever map they are.
fn set_fields(realm: &Realm, g: Guid, guild: u32, rank: u32) {
    let Some(o) = realm.registry.online(g) else {
        return;
    };
    o.map().call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.fields.set_u32(PLAYER_GUILDID, guild);
            p.fields.set_u32(PLAYER_GUILDRANK, rank);
        }
    });
}

fn today() -> (u32, u32, u32) {
    use chrono::Datelike;
    let d = chrono::Local::now();
    (d.day0(), d.month0(), (d.year() - 2000).max(0) as u32)
}

fn default_ranks() -> Vec<Rank> {
    let chat = RIGHT_CHAT_LISTEN | RIGHT_CHAT_SPEAK;
    [
        ("Guild Master", RIGHT_ALL),
        ("Officer", RIGHT_ALL),
        ("Veteran", chat),
        ("Member", chat),
        ("Initiate", chat),
    ]
    .into_iter()
    .map(|(name, rights)| Rank {
        name: name.to_owned(),
        rights,
    })
    .collect()
}

/// A new guild with `leader` at its head and `members` at its lowest rank.
pub fn found(realm: &Realm, name: &str, leader: Guid, members: &[Guid]) -> Option<u32> {
    let mut guilds = realm.social.guilds.lock();
    if guilds.name_taken(name) || guilds.of(leader).is_some() {
        return None;
    }
    guilds.next_id += 1;
    let id = guilds.next_id;
    let ranks = default_ranks();
    let lowest = ranks.len() as u32 - 1;
    let mut guild = Guild {
        id,
        name: name.to_owned(),
        leader: guid::counter(leader),
        motd: String::new(),
        info: String::new(),
        ranks,
        members: vec![Member {
            guid: guid::counter(leader),
            rank: 0,
            ..Default::default()
        }],
        emblem: Emblem::default(),
        created: today(),
    };
    for m in members
        .iter()
        .filter(|m| guilds.of(**m).is_none() && **m != leader)
    {
        guild.members.push(Member {
            guid: guid::counter(*m),
            rank: lowest,
            ..Default::default()
        });
    }
    for m in &guild.members {
        guilds.of.insert(m.guid, id);
        set_fields(realm, guid::player(m.guid), id, m.rank);
    }
    save(realm, &guild);
    guilds.guilds.insert(id, guild);
    Some(id)
}

/// `CMSG_GUILD_CREATE`: for GMs.
pub fn create(realm: &Realm, me: &Online, name: &str) {
    if !me.gm {
        return;
    }
    if !valid_name(name) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            name,
            GuildCommandResult::GuildNameInvalid,
        );
        return;
    }
    // One guild a founder, then a name of its own (Thistle Tea's
    // `Guild.create/3`).
    let result = if realm.social.guilds.lock().of(me.guid).is_some() {
        GuildCommandResult::AlreadyInGuild
    } else if found(realm, name, me.guid, &[]).is_none() {
        GuildCommandResult::GuildNameExistsS
    } else {
        return;
    };
    command_result(realm, me.guid, GuildCommand::Create, name, result);
}

pub fn valid_name(name: &str) -> bool {
    (2..=24).contains(&name.chars().count())
        && name.chars().all(|c| c.is_alphabetic() || c == ' ')
        && !name.starts_with(' ')
        && !name.ends_with(' ')
}

/// `CMSG_GUILD_INVITE`.
pub fn invite(realm: &Realm, me: &Online, name: &str) {
    let Some(them) = realm.registry.online_by_name(name) else {
        command_result(
            realm,
            me.guid,
            GuildCommand::Invite,
            name,
            GuildCommandResult::GuildPlayerNotFoundS,
        );
        return;
    };
    let mut guilds = realm.social.guilds.lock();
    let Some(guild) = guilds.of(me.guid) else {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPlayerNotInGuild,
        );
        return;
    };
    if Team::of_race(them.race) != Team::of_race(me.race) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Invite,
            name,
            GuildCommandResult::GuildNotAllied,
        );
        return;
    }
    if guilds.of(them.guid).is_some() {
        command_result(
            realm,
            me.guid,
            GuildCommand::Invite,
            name,
            GuildCommandResult::AlreadyInGuildS,
        );
        return;
    }
    if guilds.invites.contains_key(&them.guid) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Invite,
            name,
            GuildCommandResult::AlreadyInvitedToGuildS,
        );
        return;
    }
    if !has(guild.rights(me.guid), RIGHT_INVITE) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    let (id, guild_name) = (guild.id, guild.name.clone());
    guilds.invites.insert(them.guid, (id, me.guid));
    drop(guilds);
    them.outbound.send(&SMSG_GUILD_INVITE {
        player_name: me.name.to_string(),
        guild_name,
    });
    command_result(
        realm,
        me.guid,
        GuildCommand::Invite,
        name,
        GuildCommandResult::PlayerNoMoreInGuild,
    );
}

/// `CMSG_GUILD_ACCEPT`.
pub fn accept(realm: &Realm, me: &Online) {
    let mut guilds = realm.social.guilds.lock();
    let Some((id, _)) = guilds.invites.remove(&me.guid) else {
        return;
    };
    if guilds.of(me.guid).is_some() {
        return;
    }
    let Some(guild) = guilds.guilds.get_mut(&id) else {
        return;
    };
    let rank = guild.ranks.len() as u32 - 1;
    guild.members.push(Member {
        guid: guid::counter(me.guid),
        rank,
        ..Default::default()
    });
    save(realm, guild);
    guild.event(realm, GuildEvent::Joined, &[&me.name]);
    guilds.of.insert(guid::counter(me.guid), id);
    drop(guilds);
    set_fields(realm, me.guid, id, rank);
}

/// `CMSG_GUILD_DECLINE`: the invite goes, as in mangos, without a word.
pub fn decline(realm: &Realm, me: &Online) {
    realm.social.guilds.lock().invites.remove(&me.guid);
}

/// `CMSG_GUILD_INFO`.
pub fn info(realm: &Realm, me: &Online) {
    let guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of(me.guid) else {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPlayerNotInGuild,
        );
        return;
    };
    let accounts: ahash::AHashSet<Arc<str>> = g
        .members
        .iter()
        .filter_map(|m| realm.registry.name(guid::player(m.guid)))
        .map(|n| n.account.clone())
        .collect();
    me.outbound.send(&SMSG_GUILD_INFO {
        guild_name: g.name.clone(),
        created_day: g.created.0,
        created_month: g.created.1,
        created_year: g.created.2,
        amount_of_characters_in_guild: g.members.len() as u32,
        amount_of_accounts_in_guild: accounts.len() as u32,
    });
}

/// `CMSG_GUILD_ROSTER`.
pub fn roster(realm: &Realm, me: &Online) {
    let guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of(me.guid) else {
        return;
    };
    let officer = has(g.rights(me.guid), RIGHT_VIEW_OFFICER_NOTE);
    let members = g
        .members
        .iter()
        .filter_map(|m| {
            let gm = guid::player(m.guid);
            let n = realm.registry.name(gm)?;
            let online = realm.registry.online(gm);
            Some(GuildMember {
                guid: gm,
                status: match &online {
                    Some(_) => GuildMember_GuildMemberStatus::Online,
                    None => GuildMember_GuildMemberStatus::Offline { time_offline: 0.0 },
                },
                name: n.name.to_string(),
                rank: m.rank,
                level: Level::new(n.level.load(std::sync::atomic::Ordering::Relaxed)),
                class: Class::try_from(n.class).unwrap_or_default(),
                area: Area::try_from(online.map_or(0, |o| o.zone())).unwrap_or_default(),
                public_note: m.note.clone(),
                officer_note: if officer {
                    m.officer_note.clone()
                } else {
                    String::new()
                },
            })
        })
        .collect();
    me.outbound.send(&SMSG_GUILD_ROSTER {
        motd: g.motd.clone(),
        guild_info: g.info.clone(),
        rights: g.ranks.iter().map(|r| r.rights).collect(),
        members,
    });
}

/// `CMSG_GUILD_QUERY`: the guild's name, ranks and emblem.
pub fn query(realm: &Realm, me: &Online, id: u32) {
    let guilds = realm.social.guilds.lock();
    let Some(g) = guilds.guilds.get(&id) else {
        return;
    };
    me.outbound.send(&SMSG_GUILD_QUERY_RESPONSE {
        id,
        name: g.name.clone(),
        rank_names: g.rank_names(),
        emblem_style: g.emblem.style,
        emblem_color: g.emblem.color,
        border_style: g.emblem.border_style,
        border_color: g.emblem.border_color,
        background_color: g.emblem.background,
    });
}

/// The member named `name`, as `me` may act on them: their GUID and
/// rank, after the checks a promotion, demotion or removal share.
fn target(
    realm: &Realm,
    me: &Online,
    g: &Guild,
    name: &str,
    command: GuildCommand,
) -> Option<(Guid, u32)> {
    let Some(t) = realm.registry.guid_by_name(name) else {
        command_result(
            realm,
            me.guid,
            command,
            name,
            GuildCommandResult::GuildPlayerNotFoundS,
        );
        return None;
    };
    let Some(m) = g.member(t) else {
        command_result(
            realm,
            me.guid,
            command,
            name,
            GuildCommandResult::GuildPlayerNotInGuildS,
        );
        return None;
    };
    Some((t, m.rank))
}

/// `CMSG_GUILD_PROMOTE` and `CMSG_GUILD_DEMOTE`.
pub fn promote(realm: &Realm, me: &Online, name: &str, up: bool) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    let right = if up { RIGHT_PROMOTE } else { RIGHT_DEMOTE };
    if !has(g.rights(me.guid), right) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    let Some((t, rank)) = target(realm, me, g, name, GuildCommand::Create) else {
        return;
    };
    let my_rank = g.member(me.guid).map_or(u32::MAX, |m| m.rank);
    // Up to one below oneself, down to the lowest; never the leader.
    let new = if up {
        if rank <= my_rank + 1 || t == me.guid {
            command_result(
                realm,
                me.guid,
                GuildCommand::Create,
                name,
                GuildCommandResult::GuildRankTooHighS,
            );
            return;
        }
        rank - 1
    } else {
        if rank <= my_rank || rank + 1 >= g.ranks.len() as u32 {
            command_result(
                realm,
                me.guid,
                GuildCommand::Create,
                name,
                GuildCommandResult::GuildRankTooLowS,
            );
            return;
        }
        rank + 1
    };
    if let Some(m) = g.member_mut(t) {
        m.rank = new;
    }
    let rank_name = g.ranks[new as usize].name.clone();
    let tname = realm
        .registry
        .name(t)
        .map(|n| n.name.to_string())
        .unwrap_or_default();
    g.event(
        realm,
        if up {
            GuildEvent::Promotion
        } else {
            GuildEvent::Demotion
        },
        &[&me.name, &tname, &rank_name],
    );
    save(realm, g);
    let id = g.id;
    drop(guilds);
    set_fields(realm, t, id, new);
}

/// `CMSG_GUILD_LEAVE`: anyone but the leader.
pub fn leave(realm: &Realm, me: &Online) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    if g.leader == guid::counter(me.guid) && g.members.len() > 1 {
        command_result(
            realm,
            me.guid,
            GuildCommand::Quit,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    if g.members.len() <= 1 {
        let id = g.id;
        drop(guilds);
        disband_guild(realm, id);
        return;
    }
    g.event(realm, GuildEvent::Left, &[&me.name]);
    let c = guid::counter(me.guid);
    g.members.retain(|m| m.guid != c);
    save(realm, g);
    let name = g.name.clone();
    guilds.of.remove(&c);
    drop(guilds);
    set_fields(realm, me.guid, 0, 0);
    command_result(
        realm,
        me.guid,
        GuildCommand::Quit,
        &name,
        GuildCommandResult::PlayerNoMoreInGuild,
    );
}

/// `CMSG_GUILD_REMOVE`.
pub fn remove(realm: &Realm, me: &Online, name: &str) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    if !has(g.rights(me.guid), RIGHT_REMOVE) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    let Some((t, rank)) = target(realm, me, g, name, GuildCommand::Create) else {
        return;
    };
    let my_rank = g.member(me.guid).map_or(u32::MAX, |m| m.rank);
    if t == me.guid || rank <= my_rank || guid::counter(t) == g.leader {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            name,
            GuildCommandResult::GuildRankTooHighS,
        );
        return;
    }
    let tname = realm
        .registry
        .name(t)
        .map(|n| n.name.to_string())
        .unwrap_or_default();
    g.event(realm, GuildEvent::Removed, &[&tname, &me.name]);
    let c = guid::counter(t);
    g.members.retain(|m| m.guid != c);
    save(realm, g);
    guilds.of.remove(&c);
    drop(guilds);
    set_fields(realm, t, 0, 0);
}

/// `CMSG_GUILD_LEADER`: the leader hands over.
pub fn set_leader(realm: &Realm, me: &Online, name: &str) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    if g.leader != guid::counter(me.guid) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    let Some((t, _)) = target(realm, me, g, name, GuildCommand::Create) else {
        return;
    };
    g.leader = guid::counter(t);
    if let Some(m) = g.member_mut(t) {
        m.rank = 0;
    }
    if let Some(m) = g.member_mut(me.guid) {
        m.rank = 1;
    }
    let tname = realm
        .registry
        .name(t)
        .map(|n| n.name.to_string())
        .unwrap_or_default();
    g.event(realm, GuildEvent::LeaderChanged, &[&me.name, &tname]);
    save(realm, g);
    let id = g.id;
    drop(guilds);
    set_fields(realm, t, id, 0);
    set_fields(realm, me.guid, id, 1);
}

fn disband_guild(realm: &Realm, id: u32) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.guilds.remove(&id) else {
        return;
    };
    g.event(realm, GuildEvent::Disbanded, &[]);
    for m in &g.members {
        guilds.of.remove(&m.guid);
        set_fields(realm, guid::player(m.guid), 0, 0);
    }
    realm.store.delete_record(KIND_GUILD, u64::from(id));
}

/// `CMSG_GUILD_DISBAND`: the leader ends the guild.
pub fn disband(realm: &Realm, me: &Online) {
    let id = {
        let guilds = realm.social.guilds.lock();
        match guilds.of(me.guid) {
            Some(g) if g.leader == guid::counter(me.guid) => g.id,
            Some(_) => {
                command_result(
                    realm,
                    me.guid,
                    GuildCommand::Create,
                    "",
                    GuildCommandResult::GuildPermissionsOrLeader,
                );
                return;
            }
            None => return,
        }
    };
    disband_guild(realm, id);
}

/// `CMSG_GUILD_MOTD`.
pub fn set_motd(realm: &Realm, me: &Online, motd: &str) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    if !has(g.rights(me.guid), RIGHT_SET_MOTD) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    g.motd = motd.chars().take(128).collect();
    g.event(realm, GuildEvent::Motd, &[&g.motd]);
    save(realm, g);
}

/// `CMSG_GUILD_INFO_TEXT`.
pub fn set_info(realm: &Realm, me: &Online, info: &str) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    if !has(g.rights(me.guid), RIGHT_SET_MOTD) {
        return;
    }
    g.info = info.chars().take(500).collect();
    save(realm, g);
}

/// `CMSG_GUILD_SET_PUBLIC_NOTE` and `CMSG_GUILD_SET_OFFICER_NOTE`.
pub fn set_note(realm: &Realm, me: &Online, name: &str, note: &str, officer: bool) {
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    let right = if officer {
        RIGHT_EDIT_OFFICER_NOTE
    } else {
        RIGHT_EDIT_PUBLIC_NOTE
    };
    if !has(g.rights(me.guid), right) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    let Some((t, _)) = target(realm, me, g, name, GuildCommand::Create) else {
        return;
    };
    if let Some(m) = g.member_mut(t) {
        let note: String = note.chars().take(31).collect();
        if officer {
            m.officer_note = note;
        } else {
            m.note = note;
        }
    }
    save(realm, g);
    drop(guilds);
    roster(realm, me);
}

/// `CMSG_GUILD_RANK`, `CMSG_GUILD_ADD_RANK` and `CMSG_GUILD_DEL_RANK`: the
/// leader's.
pub fn edit_rank(realm: &Realm, me: &Online, edit: RankEdit) {
    let mut moved = Vec::new();
    let mut guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of_mut(me.guid) else {
        return;
    };
    if g.leader != guid::counter(me.guid) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildPermissionsOrLeader,
        );
        return;
    }
    match edit {
        RankEdit::Set { rank, rights, name } => {
            if rank as usize >= g.ranks.len() {
                return;
            }
            // The leader's rank may be renamed, and keeps every right
            // (mangos's `HandleGuildRankOpcode`, Thistle Tea's).
            let rights = if rank == 0 { RIGHT_ALL } else { rights };
            g.ranks[rank as usize] = Rank {
                name: name.chars().take(15).collect(),
                rights: rights | RIGHT_EMPTY,
            };
        }
        RankEdit::Add(name) => {
            if g.ranks.len() >= MAX_RANKS {
                return;
            }
            g.ranks.push(Rank {
                name: name.chars().take(15).collect(),
                rights: RIGHT_CHAT_LISTEN | RIGHT_CHAT_SPEAK,
            });
        }
        RankEdit::DeleteLowest => {
            let lowest = g.ranks.len() as u32 - 1;
            if g.ranks.len() <= MIN_RANKS {
                return;
            }
            g.ranks.pop();
            // Its members drop to the rank now lowest (Thistle Tea's
            // `delete_rank`; mangos leaves them a rank that is gone).
            for m in g.members.iter_mut().filter(|m| m.rank == lowest) {
                m.rank = lowest - 1;
                moved.push(guid::player(m.guid));
            }
        }
    }
    save(realm, g);
    let id = g.id;
    drop(guilds);
    for m in moved {
        set_fields(realm, m, id, lowest_after(realm, id));
    }
    roster(realm, me);
}

/// A guild's lowest rank.
fn lowest_after(realm: &Realm, id: u32) -> u32 {
    realm
        .social
        .guilds
        .lock()
        .guilds
        .get(&id)
        .map_or(0, |g| g.ranks.len() as u32 - 1)
}

pub enum RankEdit {
    Set {
        rank: u32,
        rights: u32,
        name: String,
    },
    Add(String),
    DeleteLowest,
}

/// Guild and officer chat, to the members whose ranks may hear it.
pub fn chat(
    realm: &Realm,
    me: &Online,
    officer: bool,
    text: &str,
    language: Language,
    tag: PlayerChatTag,
) {
    let guilds = realm.social.guilds.lock();
    let Some(g) = guilds.of(me.guid) else {
        return;
    };
    let (speak, listen) = if officer {
        (RIGHT_OFFICER_SPEAK, RIGHT_OFFICER_LISTEN)
    } else {
        (RIGHT_CHAT_SPEAK, RIGHT_CHAT_LISTEN)
    };
    if !has(g.rights(me.guid), speak) {
        return;
    }
    // Under the lock, only who may hear it: each member's rights by their
    // own rank, not looked up again (a guild of N had N² compares).
    let mut hearers: Vec<Guid> = g
        .members
        .iter()
        .filter(|m| has(g.ranks.get(m.rank as usize).map_or(0, |r| r.rights), listen))
        .map(|m| guid::player(m.guid))
        .collect();
    drop(guilds);
    let chat_type = if officer {
        SMSG_MESSAGECHAT_ChatType::Officer { sender2: me.guid }
    } else {
        SMSG_MESSAGECHAT_ChatType::Guild { sender2: me.guid }
    };
    let Some(frame) = encode(&SMSG_MESSAGECHAT {
        chat_type,
        language,
        message: text.to_owned(),
        tag,
    }) else {
        return;
    };
    not_ignoring(realm, me.guid, &mut hearers);
    send_frames(realm, &hearers, &frame);
}

/// `MSG_SAVE_GUILD_EMBLEM`: the leader buys the guild an emblem at a
/// tabard designer, and is told the guild as it is now (mangos's
/// `HandleSaveGuildEmblemOpcode`, its checks in its order).
pub fn save_emblem(map: &mut Map, g: Guid, m: &MSG_SAVE_GUILD_EMBLEM_Client) {
    let realm = map.realm.clone();
    let Some(money) = map.players.get(&g).map(|p| p.money()) else {
        return;
    };
    let designer = crate::game::map::npc::NPC_FLAG_TABARDDESIGNER;
    let at_designer = crate::game::map::npc::npc_entry(map, g, m.vendor, designer).is_some();
    let mut guilds = realm.social.guilds.lock();
    let mut saved = None;
    let result = match guilds.of_mut(g) {
        // 5, `ERR_GUILDEMBLEM_INVALIDVENDOR`.
        _ if !at_designer => GuildEmblemResult::NoMessage,
        None => GuildEmblemResult::NoGuild,
        Some(guild) if guild.leader != guid::counter(g) => GuildEmblemResult::NotGuildMaster,
        Some(_) if money < EMBLEM_COST => GuildEmblemResult::NotEnoughMoney,
        Some(guild) => {
            guild.emblem = Emblem {
                style: m.emblem_style,
                color: m.emblem_color,
                border_style: m.border_style,
                border_color: m.border_color,
                background: m.background_color,
            };
            let mut records = Records::new();
            records.put(KIND_GUILD, u64::from(guild.id), guild);
            guild.event(&realm, GuildEvent::TabardChanged, &[]);
            if let Some(p) = map.players.get_mut(&g) {
                p.set_money(money - EMBLEM_COST);
            }
            // The emblem and what it cost, together, under the guilds'
            // lock.
            map.save_together(&[g], records);
            saved = Some(guild.id);
            GuildEmblemResult::Success
        }
    };
    drop(guilds);
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&MSG_SAVE_GUILD_EMBLEM_Server { result });
        if let Some(id) = saved {
            query(&realm, &p.online, id);
        }
    }
}

/// At login: the guild's fields, the MOTD, and the others told.
pub fn logged_in(map: &mut Map, g: Guid) {
    let realm = map.realm.clone();
    let guilds = realm.social.guilds.lock();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(guild) = guilds.of(g) else {
        p.fields.set_u32(PLAYER_GUILDID, 0);
        p.fields.set_u32(PLAYER_GUILDRANK, 0);
        return;
    };
    let rank = guild.member(g).map_or(0, |m| m.rank);
    p.fields.set_u32(PLAYER_GUILDID, guild.id);
    p.fields.set_u32(PLAYER_GUILDRANK, rank);
    if let Some(frame) = encode(&SMSG_GUILD_EVENT {
        event: GuildEvent::Motd,
        event_descriptions: vec![guild.motd.clone()],
    }) {
        p.outbound.send_frame(frame);
    }
    let name = p.name.to_string();
    guild.event(&realm, GuildEvent::SignedOn, &[&name]);
}

/// Going offline: the others are told.
pub fn logged_out(realm: &Realm, g: Guid) {
    let invites = {
        let mut guilds = realm.social.guilds.lock();
        guilds.invites.remove(&g);
        guilds.of(g).map(|guild| guild.id)
    };
    if invites.is_none() {
        return;
    }
    let Some(name) = realm.registry.name(g) else {
        return;
    };
    let guilds = realm.social.guilds.lock();
    if let Some(guild) = guilds.of(g) {
        let Some(frame) = encode(&SMSG_GUILD_EVENT {
            event: GuildEvent::SignedOff,
            event_descriptions: vec![name.name.to_string()],
        }) else {
            return;
        };
        for o in guild.online(realm).iter().filter(|o| o.guid != g) {
            o.outbound.send_frame(frame.clone());
        }
    }
}

fn save_petition(realm: &Realm, p: &Petition) {
    realm.store.put_record(KIND_PETITION, u64::from(p.id), p);
}

/// `CMSG_PETITION_SHOWLIST`: the charter a guild master sells.
pub fn petition_showlist(map: &Map, g: Guid, npc: Guid) {
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_PETITION_SHOWLIST {
            npc,
            petitions: vec![PetitionShowlist {
                index: 1,
                charter_entry: CHARTER_ITEM,
                charter_display_id: CHARTER_DISPLAY,
                guild_charter_cost: CHARTER_COST,
                unknown1: 0,
            }],
        });
    }
}

/// `CMSG_PETITION_BUY`: a charter for a guild of that name.
pub fn petition_buy(map: &mut Map, g: Guid, npc: Guid, name: &str) {
    let realm = map.realm.clone();
    let Some(me) = realm.registry.online(g) else {
        return;
    };
    let fail = |result| command_result(&realm, g, GuildCommand::Create, name, result);
    {
        let guilds = realm.social.guilds.lock();
        if guilds.of(g).is_some() {
            fail(GuildCommandResult::AlreadyInGuild);
            return;
        }
        if !valid_name(name) {
            fail(GuildCommandResult::GuildNameInvalid);
            return;
        }
        if guilds.name_taken(name) {
            fail(GuildCommandResult::GuildNameExistsS);
            return;
        }
    }
    let Some(p) = map.players.get(&g) else {
        return;
    };
    // Short of money, told so as a vendor tells it (mangos's
    // `HandlePetitionBuyOpcode`, `BUY_ERR_NOT_ENOUGHT_MONEY`).
    if p.money() < CHARTER_COST {
        p.outbound.send(&SMSG_BUY_FAILED {
            guid: npc,
            item: CHARTER_ITEM,
            result: BuyResult::NotEnoughtMoney,
        });
        return;
    }
    let before: Vec<Guid> = p.items.iter().map(|i| i.guid).collect();
    // No room, or one already carried (the charter's limit): told so.
    if let Err(fail) = crate::game::map::inventory::add_item(map, g, CHARTER_ITEM, 1, true) {
        if let Some(p) = map.players.get(&g) {
            crate::game::map::inventory::send_fail(p, fail, Guid::zero());
        }
        return;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(item) = p
        .items
        .iter()
        .find(|i| i.entry == CHARTER_ITEM && !before.contains(&i.guid))
        .map(|i| i.guid)
    else {
        return;
    };
    let money = p.money();
    p.set_money(money - CHARTER_COST);
    let petition = Petition {
        id: guid::counter(item),
        owner: guid::counter(me.guid),
        name: name.to_owned(),
        signers: Vec::new(),
    };
    // The charter, its petition and its cost together.
    let mut records = Records::new();
    records.put(KIND_PETITION, u64::from(petition.id), &petition);
    let mut guilds = realm.social.guilds.lock();
    // One charter an owner: a new one ends the owner's older petitions
    // (mangos's `HandlePetitionBuyOpcode` deletes them), their names free.
    let older: Vec<u32> = guilds
        .petitions
        .values()
        .filter(|p| p.owner == petition.owner)
        .map(|p| p.id)
        .collect();
    for id in older {
        guilds.petitions.remove(&id);
        records.delete(KIND_PETITION, u64::from(id));
    }
    guilds.petitions.insert(petition.id, petition);
    map.save_together(&[g], records);
    drop(guilds);
}

fn signatures(petition: &Petition, item: Guid) -> SMSG_PETITION_SHOW_SIGNATURES {
    SMSG_PETITION_SHOW_SIGNATURES {
        item,
        owner: guid::player(petition.owner),
        petition: petition.id,
        signatures: petition
            .signers
            .iter()
            .map(|s| PetitionSignature {
                signer: guid::player(*s),
                unknown1: 0,
            })
            .collect(),
    }
}

/// `CMSG_PETITION_SHOW_SIGNATURES`.
pub fn petition_signatures(realm: &Realm, me: &Online, item: Guid) {
    let guilds = realm.social.guilds.lock();
    if let Some(p) = guilds.petitions.get(&guid::counter(item)) {
        me.outbound.send(&signatures(p, item));
    }
}

/// `CMSG_PETITION_QUERY`.
pub fn petition_query(realm: &Realm, me: &Online, item: Guid) {
    let guilds = realm.social.guilds.lock();
    let Some(p) = guilds.petitions.get(&guid::counter(item)) else {
        return;
    };
    let needed = realm.config().petition_signatures;
    me.outbound.send(&SMSG_PETITION_QUERY_RESPONSE {
        petition_id: p.id,
        charter_owner: guid::player(p.owner),
        guild_name: p.name.clone(),
        body_text: String::new(),
        unknown_flags: 1,
        minimum_signatures: needed,
        maximum_signatures: needed,
        deadline: 0,
        issue_date: 0,
        allowed_guild_id: 0,
        allowed_class: AllowedClass::empty(),
        allowed_race: AllowedRace::empty(),
        allowed_genders: 0,
        allowed_minimum_level: Level::new(0),
        allowed_maximum_level: Level::new(0),
        todo_amount_of_signers: 0,
        number_of_choices: 0,
    });
}

/// `CMSG_OFFER_PETITION`: the charter shown to someone to sign.
pub fn petition_offer(realm: &Realm, me: &Online, item: Guid, to: Guid) {
    let Some(them) = realm.registry.online(to) else {
        return;
    };
    if Team::of_race(them.race) != Team::of_race(me.race) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildNotAllied,
        );
        return;
    }
    let guilds = realm.social.guilds.lock();
    if let Some(p) = guilds
        .petitions
        .get(&guid::counter(item))
        .filter(|p| p.owner == guid::counter(me.guid))
    {
        them.outbound.send(&signatures(p, item));
    }
}

/// `CMSG_PETITION_SIGN`.
pub fn petition_sign(realm: &Realm, me: &Online, item: Guid) {
    let mut guilds = realm.social.guilds.lock();
    let in_guild = guilds.of(me.guid).is_some();
    // One invited to a guild signs no charter (mangos's
    // `HandlePetitionSignOpcode`, `ERR_ALREADY_INVITED_TO_GUILD_S`).
    if !in_guild && guilds.invites.contains_key(&me.guid) {
        drop(guilds);
        command_result(
            realm,
            me.guid,
            GuildCommand::Invite,
            &me.name,
            GuildCommandResult::AlreadyInvitedToGuildS,
        );
        return;
    }
    let Some(p) = guilds.petitions.get_mut(&guid::counter(item)) else {
        return;
    };
    let owner = guid::player(p.owner);
    let me_c = guid::counter(me.guid);
    // One signature an account, as mangos has it.
    let same_account = |c: u32| {
        realm
            .registry
            .name(guid::player(c))
            .is_some_and(|n| n.account == me.account)
    };
    // The owner's side alone, and no more than the charter needs
    // (Thistle Tea's `Petitions.sign`).
    let other_side = realm
        .registry
        .name(owner)
        .is_some_and(|n| Team::of_race(n.race) != Team::of_race(me.race));
    if other_side {
        drop(guilds);
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            "",
            GuildCommandResult::GuildNotAllied,
        );
        return;
    }
    let full = p.signers.len() >= realm.config().petition_signatures as usize;
    let result = if p.owner == me_c {
        PetitionResult::CantSignOwn
    } else if full {
        PetitionResult::AlreadySigned
    } else if in_guild {
        PetitionResult::AlreadyInGuild
    } else if p.signers.contains(&me_c)
        || same_account(p.owner)
        || p.signers.iter().any(|s| same_account(*s))
    {
        PetitionResult::AlreadySigned
    } else {
        p.signers.push(me_c);
        save_petition(realm, p);
        PetitionResult::Ok
    };
    drop(guilds);
    let reply = SMSG_PETITION_SIGN_RESULTS {
        petition: item,
        owner: me.guid,
        result,
    };
    me.outbound.send(&reply);
    if result == PetitionResult::Ok {
        send_to(realm, owner, &reply);
    }
}

/// `MSG_PETITION_DECLINE`: the owner hears who wouldn't.
pub fn petition_decline(realm: &Realm, me: &Online, item: Guid) {
    let owner = realm
        .social
        .guilds
        .lock()
        .petitions
        .get(&guid::counter(item))
        .map(|p| guid::player(p.owner));
    if let Some(owner) = owner {
        send_to(realm, owner, &MSG_PETITION_DECLINE { petition: me.guid });
    }
}

/// `MSG_PETITION_RENAME`.
pub fn petition_rename(realm: &Realm, me: &Online, item: Guid, name: &str) {
    let mut guilds = realm.social.guilds.lock();
    if !valid_name(name) || guilds.name_taken(name) {
        command_result(
            realm,
            me.guid,
            GuildCommand::Create,
            name,
            GuildCommandResult::GuildNameExistsS,
        );
        return;
    }
    let Some(p) = guilds
        .petitions
        .get_mut(&guid::counter(item))
        .filter(|p| p.owner == guid::counter(me.guid))
    else {
        return;
    };
    p.name = name.to_owned();
    save_petition(realm, p);
    me.outbound.send(&MSG_PETITION_RENAME {
        petition: item,
        new_name: name.to_owned(),
    });
}

/// `CMSG_TURN_IN_PETITION`: enough signatures make the guild.
pub fn petition_turn_in(map: &mut Map, g: Guid, item: Guid) {
    let realm = map.realm.clone();
    // The charter in hand (mangos's `HandleTurnInPetitionOpcode`): one
    // sold or destroyed founds nothing.
    if map
        .players
        .get(&g)
        .and_then(|p| p.item_by_guid(item))
        .is_none()
    {
        return;
    }
    let needed = realm.config().petition_signatures as usize;
    let petition = {
        let guilds = realm.social.guilds.lock();
        let Some(p) = guilds
            .petitions
            .get(&guid::counter(item))
            .filter(|p| p.owner == guid::counter(g))
        else {
            return;
        };
        if guilds.of(g).is_some() {
            send_to(
                &realm,
                g,
                &SMSG_TURN_IN_PETITION_RESULTS {
                    result: PetitionResult::AlreadyInGuild,
                },
            );
            return;
        }
        if p.signers.len() < needed {
            send_to(
                &realm,
                g,
                &SMSG_TURN_IN_PETITION_RESULTS {
                    result: PetitionResult::NeedMore,
                },
            );
            return;
        }
        p.clone()
    };
    let signers: Vec<Guid> = petition.signers.iter().map(|s| guid::player(*s)).collect();
    // The charter holds its name until the guild takes it.
    realm.social.guilds.lock().petitions.remove(&petition.id);
    if found(&realm, &petition.name, g, &signers).is_none() {
        realm
            .social
            .guilds
            .lock()
            .petitions
            .insert(petition.id, petition.clone());
        command_result(
            &realm,
            g,
            GuildCommand::Create,
            &petition.name,
            GuildCommandResult::GuildNameExistsS,
        );
        return;
    }
    realm
        .store
        .delete_record(KIND_PETITION, u64::from(petition.id));
    if let Some(i) = map.players.get(&g).and_then(|p| p.item_by_guid(item)) {
        crate::game::map::inventory::take_item(map, g, i, 1);
    }
    send_to(
        &realm,
        g,
        &SMSG_TURN_IN_PETITION_RESULTS {
            result: PetitionResult::Ok,
        },
    );
}

/// Whether a character leads a guild, even one of themselves alone: they
/// may not be deleted (mangos's `HandleCharDeleteOpcode`,
/// `CHAR_DELETE_FAILED_GUILD_LEADER`), but pass it on or disband it first.
pub fn leads(realm: &Realm, counter: u32) -> bool {
    let guilds = realm.social.guilds.lock();
    guilds
        .of
        .get(&counter)
        .and_then(|id| guilds.guilds.get(id))
        .is_some_and(|g| g.leader == counter)
}

/// A character deleted: out of their guild (which goes if they were all of
/// it), and their charters and signatures with them.
pub fn character_deleted(realm: &Realm, counter: u32) {
    let disband = {
        let mut guilds = realm.social.guilds.lock();
        let id = guilds.of.remove(&counter);
        let mut disband = None;
        if let Some(id) = id
            && let Some(g) = guilds.guilds.get_mut(&id)
        {
            g.members.retain(|m| m.guid != counter);
            if g.members.is_empty() {
                disband = Some(id);
            } else {
                save(realm, g);
            }
        }
        let owned: Vec<u32> = guilds
            .petitions
            .values()
            .filter(|p| p.owner == counter)
            .map(|p| p.id)
            .collect();
        for id in owned {
            guilds.petitions.remove(&id);
            realm.store.delete_record(KIND_PETITION, u64::from(id));
        }
        let signed: Vec<u32> = guilds
            .petitions
            .values()
            .filter(|p| p.signers.contains(&counter))
            .map(|p| p.id)
            .collect();
        for id in signed {
            if let Some(p) = guilds.petitions.get_mut(&id) {
                p.signers.retain(|s| *s != counter);
                save_petition(realm, p);
            }
        }
        disband
    };
    if let Some(id) = disband {
        disband_guild(realm, id);
    }
}
