//! The map's side of the social services: whispers, AFK and DND, `/who`,
//! inspect, the opcodes that reach channels, friends and groups, and the
//! stats of grouped players for the members who can't see them.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage as C;
use wow_world_messages::vanilla::*;

use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::chat::tag_of;
use crate::game::map::player::{PLAYER_FLAGS_AFK, PLAYER_FLAGS_DND};
use crate::game::map::{duel, trade};
use crate::game::registry::Online;
use crate::game::social::groups::{self, MemberStats};
use crate::game::social::{auction, channels, friends, guilds, mail, same_side};

pub const SMSG_WHO: u16 = 0x063;
/// How often a grouped player's stats go to the members who can't see them.
pub const STATS_EVERY: Duration = Duration::from_secs(1);
/// The most a `/who` lists, and the default of `[world]
/// max_who_list_returns`.
pub const WHO_LIMIT: usize = 49;
/// Alterac Valley, Warsong Gulch and Arathi Basin.
const BATTLEGROUND_ZONES: [u32; 3] = [2597, 3277, 3358];

fn online(map: &Map, g: Guid) -> Option<Arc<Online>> {
    map.realm.registry.online(g)
}

/// Chat this map doesn't hear nearby: whispers, AFK and DND, channels,
/// party and raid.
pub fn message(map: &mut Map, g: Guid, m: CMSG_MESSAGECHAT) {
    let Some(me) = online(map, g) else {
        return;
    };
    let tag = tag_of(map, g);
    let realm = map.realm.clone();
    match m.chat_type {
        CMSG_MESSAGECHAT_ChatType::Whisper { target_player } => {
            whisper(map, &me, &target_player, &m.message, m.language, tag)
        }
        CMSG_MESSAGECHAT_ChatType::Afk => away(map, g, PLAYER_FLAGS_AFK, &m.message),
        CMSG_MESSAGECHAT_ChatType::Dnd => away(map, g, PLAYER_FLAGS_DND, &m.message),
        CMSG_MESSAGECHAT_ChatType::Channel { channel } => {
            channels::say(&realm, &me, &channel, &m.message, m.language, tag)
        }
        CMSG_MESSAGECHAT_ChatType::Guild => {
            guilds::chat(&realm, &me, false, &m.message, m.language, tag)
        }
        CMSG_MESSAGECHAT_ChatType::Officer => {
            guilds::chat(&realm, &me, true, &m.message, m.language, tag)
        }
        kind @ (CMSG_MESSAGECHAT_ChatType::Party
        | CMSG_MESSAGECHAT_ChatType::Raid
        | CMSG_MESSAGECHAT_ChatType::RaidLeader
        | CMSG_MESSAGECHAT_ChatType::RaidWarning) => {
            if !m.message.trim().is_empty() {
                groups::chat(&realm, &me, kind, &m.message, m.language, tag);
            }
        }
        other => tracing::trace!("chat type {other:?} not served"),
    }
}

/// A whisper: to the player by that name online on the same side, an echo
/// to the speaker, and their AFK or DND message back.
fn whisper(map: &Map, me: &Online, to: &str, text: &str, language: Language, tag: PlayerChatTag) {
    let realm = &map.realm;
    let Some(t) = realm.registry.online_by_name(to) else {
        me.outbound.send(&SMSG_CHAT_PLAYER_NOT_FOUND {
            name: to.to_owned(),
        });
        return;
    };
    // One on the other side is there, but can't be whispered: "wrong
    // faction", not "not found" (mangos's `SendWrongFactionNotice`).
    if !same_side(me, &t) {
        me.outbound.send(&SMSG_CHAT_WRONG_FACTION {});
        return;
    }
    if text.trim().is_empty() {
        return;
    }
    let flags = t.flags.load(Ordering::Relaxed);
    // DND holds whispers back; AFK lets them through with its message.
    if flags & PLAYER_FLAGS_DND == 0 {
        if realm.social.lists.lock().ignores(t.guid, me.guid) {
            me.outbound.send(&SMSG_MESSAGECHAT {
                chat_type: SMSG_MESSAGECHAT_ChatType::Ignored { sender2: t.guid },
                language: Language::Universal,
                message: String::new(),
                tag: PlayerChatTag::None,
            });
            return;
        }
        t.outbound.send(&SMSG_MESSAGECHAT {
            chat_type: SMSG_MESSAGECHAT_ChatType::Whisper { sender2: me.guid },
            language,
            message: text.to_owned(),
            tag,
        });
    }
    me.outbound.send(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::WhisperInform { sender2: t.guid },
        language,
        message: text.to_owned(),
        tag,
    });
    if flags & (PLAYER_FLAGS_AFK | PLAYER_FLAGS_DND) != 0 {
        let away = t.away.lock().clone();
        let chat_type = if flags & PLAYER_FLAGS_DND != 0 {
            SMSG_MESSAGECHAT_ChatType::Dnd { sender2: t.guid }
        } else {
            SMSG_MESSAGECHAT_ChatType::Afk { sender2: t.guid }
        };
        me.outbound.send(&SMSG_MESSAGECHAT {
            chat_type,
            language: Language::Universal,
            message: away,
            tag: PlayerChatTag::None,
        });
    }
}

/// `/afk` and `/dnd`, as mangos's `HandleMessagechatOpcode` and Thistle
/// Tea's `ChatStatus.change` take them: on, with the message (and the
/// other off); on already, a message changes the reply and none turns it
/// off. No one goes AFK in a fight.
fn away(map: &mut Map, g: Guid, flag: u32, text: &str) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let fighting = p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    );
    if flag == PLAYER_FLAGS_AFK && fighting {
        return;
    }
    let was = p.fields.has_flag(PLAYER_FLAGS, flag);
    let empty = text.trim().is_empty();
    if was && empty {
        p.fields.set_flag(PLAYER_FLAGS, flag, false);
    } else {
        // AFK and DND are one or the other.
        let other = if flag == PLAYER_FLAGS_AFK {
            PLAYER_FLAGS_DND
        } else {
            PLAYER_FLAGS_AFK
        };
        p.fields.set_flag(PLAYER_FLAGS, other, false);
        p.fields.set_flag(PLAYER_FLAGS, flag, true);
        let default = if flag == PLAYER_FLAGS_AFK {
            "Away"
        } else {
            "Busy"
        };
        *p.online.away.lock() = if empty {
            default.to_owned()
        } else {
            text.to_owned()
        };
    }
    let flags = p.fields.get_u32(PLAYER_FLAGS) & (PLAYER_FLAGS_AFK | PLAYER_FLAGS_DND);
    p.online.flags.store(flags, Ordering::Relaxed);
    // No one AFK in a battleground: out they go, a deserter, unless in GM
    // mode (mangos's `Player::ToggleAFK`, "afk player not allowed in
    // battleground").
    let gm_mode = p
        .fields
        .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM);
    if !was && flags & PLAYER_FLAGS_AFK != 0 && map.battleground.is_some() && !gm_mode {
        crate::game::battlegrounds::leave(map, g);
    }
}

/// `CMSG_WHO`: those online on the same side, by level, name, race, class,
/// zone and the search words, as mangos filters them.
pub fn who(map: &Map, g: Guid, m: &CMSG_WHO) {
    let realm = &map.realm;
    let Some(me) = online(map, g) else {
        return;
    };
    // More than the client can send is no query, as mangos's
    // HandleWhoOpcode and Thistle Tea's CmsgWho turn it away.
    if m.zones.len() > 10 || m.search_strings.len() > 4 {
        return;
    }
    let (low, high) = (
        u32::from(m.minimum_level.as_int()),
        u32::from(m.maximum_level.as_int()),
    );
    let name = m.player_name.to_lowercase();
    let guild_filter = m.guild_name.to_lowercase();
    // Any word, in the name, the guild or the zone (Thistle Tea's `Who`):
    // lowered once, not for each player.
    let words: Vec<String> = m
        .search_strings
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    let zones: Vec<u32> = m.zones.clone();
    let mut b = Body::default();
    let mut listed = 0u32;
    let mut rows = Body::default();
    let everyone = realm.registry.all_online();
    let here = me.map().key;
    let limit = map.cfg.max_who_list_returns.min(WHO_LIMIT);
    for o in everyone.iter().cloned() {
        // A full list (`[world] max_who_list_returns`, at most the client's
        // 49): the rest would only be looked at (the guilds' lock taken for
        // each) to be left out.
        if listed as usize >= limit {
            break;
        }
        if !same_side(&me, &o) || (o.gm && !me.gm) {
            continue;
        }
        let level = u32::from(o.level());
        if (low > 0 && level < low) || (high > 0 && level > high) {
            continue;
        }
        if m.race_mask != 0 && m.race_mask & (1 << o.race) == 0 && m.race_mask != u32::MAX {
            continue;
        }
        if m.class_mask != 0 && m.class_mask & (1 << o.class) == 0 && m.class_mask != u32::MAX {
            continue;
        }
        let lower = o.name.to_lowercase();
        if !name.is_empty() && !lower.contains(&name) {
            continue;
        }
        if !zones.is_empty() && !zones.contains(&o.zone()) {
            continue;
        }
        // In a battleground, its players from the asker's own copy alone.
        if !zones.is_empty()
            && o.zone() == me.zone()
            && BATTLEGROUND_ZONES.contains(&o.zone())
            && o.map().key != here
        {
            continue;
        }
        let guild = realm.social.guilds.lock().name_of(o.guid);
        let guild_lower = guild.to_lowercase();
        if !guild_filter.is_empty() && !guild_lower.contains(&guild_filter) {
            continue;
        }
        let zone_name = realm
            .data
            .areas
            .get(&o.zone())
            .map(|a| a.name.to_lowercase())
            .unwrap_or_default();
        if !words.is_empty()
            && !words.iter().any(|w| {
                lower.contains(w.as_str())
                    || guild_lower.contains(w.as_str())
                    || zone_name.contains(w.as_str())
            })
        {
            continue;
        }
        listed += 1;
        rows.cstr(&o.name)
            .cstr(&guild)
            .u32(level)
            .u32(u32::from(o.class))
            .u32(u32::from(o.race))
            .u32(o.zone());
    }
    // The count: everyone online past a full list, else those listed.
    let online = if everyone.len() > limit {
        everyone.len() as u32
    } else {
        listed
    };
    b.u32(listed).u32(online);
    b.0.extend_from_slice(&rows.0);
    if let Some(p) = map.players.get(&g) {
        p.outbound.send_frame(b.frame(SMSG_WHO));
    }
}

/// `CMSG_INSPECT`: the target hears of it; what they wear the inspector
/// already sees in their fields.
pub fn inspect(map: &mut Map, g: Guid, target: Guid) {
    if let Some(p) = map.players.get_mut(&g) {
        p.fields.set_guid(UNIT_FIELD_TARGET, target);
    }
    let realm = map.realm.clone();
    if let (Some(me), Some(them)) = (online(map, g), realm.registry.online(target))
        && same_side(&me, &them)
    {
        them.outbound.send(&SMSG_INSPECT { guid: g });
    }
}

/// Whether `npc` is a guild master `g` may deal with now.
fn petitioner(map: &Map, g: Guid, npc: Guid) -> bool {
    crate::game::map::npc::npc_entry(map, g, npc, crate::game::map::npc::NPC_FLAG_PETITIONER)
        .is_some()
}

/// The opcodes that go to the social services. Returns the message if it
/// is none of theirs.
pub fn handle(map: &mut Map, g: Guid, msg: C) -> Option<C> {
    let realm = map.realm.clone();
    let me = online(map, g)?;
    match msg {
        C::CMSG_JOIN_CHANNEL(m) => {
            channels::join(&realm, &me, &m.channel_name, &m.channel_password)
        }
        C::CMSG_LEAVE_CHANNEL(m) => channels::leave(&realm, &me, &m.channel_name, true),
        C::CMSG_CHANNEL_LIST(m) => channels::list(&realm, &me, &m.channel_name),
        C::CMSG_CHANNEL_PASSWORD(m) => {
            channels::password(&realm, &me, &m.channel_name, &m.channel_password)
        }
        C::CMSG_CHANNEL_SET_OWNER(m) => {
            channels::set_owner(&realm, &me, &m.channel_name, &m.new_owner)
        }
        C::CMSG_CHANNEL_OWNER(m) => channels::owner(&realm, &me, &m.channel_name),
        C::CMSG_CHANNEL_MODERATOR(m) => channels::set_mode(
            &realm,
            &me,
            &m.channel_name,
            &m.player_name,
            channels::MEMBER_MODERATOR,
            true,
        ),
        C::CMSG_CHANNEL_UNMODERATOR(m) => channels::set_mode(
            &realm,
            &me,
            &m.channel_name,
            &m.player_name,
            channels::MEMBER_MODERATOR,
            false,
        ),
        C::CMSG_CHANNEL_MUTE(m) => channels::set_mode(
            &realm,
            &me,
            &m.channel_name,
            &m.player_name,
            channels::MEMBER_MUTED,
            true,
        ),
        C::CMSG_CHANNEL_UNMUTE(m) => channels::set_mode(
            &realm,
            &me,
            &m.channel_name,
            &m.player_name,
            channels::MEMBER_MUTED,
            false,
        ),
        C::CMSG_CHANNEL_INVITE(m) => channels::invite(&realm, &me, &m.channel_name, &m.player_name),
        C::CMSG_CHANNEL_KICK(m) => {
            channels::kick(&realm, &me, &m.channel_name, &m.player_name, false)
        }
        C::CMSG_CHANNEL_BAN(m) => {
            channels::kick(&realm, &me, &m.channel_name, &m.player_name, true)
        }
        C::CMSG_CHANNEL_UNBAN(m) => channels::unban(&realm, &me, &m.channel_name, &m.player_name),
        C::CMSG_CHANNEL_ANNOUNCEMENTS(m) => channels::toggle(&realm, &me, &m.channel_name, false),
        C::CMSG_CHANNEL_MODERATE(m) => channels::toggle(&realm, &me, &m.channel_name, true),
        C::CMSG_ADD_FRIEND(m) => friends::add_friend(map, g, &m.name),
        C::CMSG_DEL_FRIEND(m) => friends::del_friend(map, g, m.guid),
        C::CMSG_ADD_IGNORE(m) => friends::add_ignore(map, g, &m.name),
        C::CMSG_DEL_IGNORE(m) => friends::del_ignore(map, g, m.guid),
        C::CMSG_WHO(m) => who(map, g, &m),
        C::CMSG_INSPECT(m) => inspect(map, g, m.guid),
        C::CMSG_GROUP_INVITE(m) => groups::invite(&realm, &me, &m.name),
        C::CMSG_GROUP_ACCEPT => groups::accept(&realm, &me),
        C::CMSG_GROUP_DECLINE => groups::decline(&realm, &me),
        C::CMSG_GROUP_UNINVITE(m) => groups::uninvite(&realm, &me, None, &m.name),
        C::CMSG_GROUP_UNINVITE_GUID(m) => groups::uninvite(&realm, &me, Some(m.guid), ""),
        C::CMSG_GROUP_SET_LEADER(m) => groups::set_leader(&realm, &me, m.guid),
        C::CMSG_GROUP_DISBAND => groups::leave(&realm, &me),
        C::CMSG_LOOT_METHOD(m) => groups::loot_method(&realm, &me, &m),
        C::CMSG_GROUP_RAID_CONVERT => groups::convert_to_raid(&realm, &me),
        C::CMSG_GROUP_CHANGE_SUB_GROUP(m) => {
            groups::change_subgroup(&realm, &me, &m.name, m.group_number)
        }
        C::CMSG_GROUP_ASSISTANT_LEADER(m) => {
            groups::set_assistant(&realm, &me, m.guid, m.set_assistant)
        }
        C::MSG_RAID_READY_CHECK(m) => groups::ready_check(&realm, &me, m.answer.map(|a| a.state)),
        C::MSG_RAID_TARGET_UPDATE(m) => {
            use MSG_RAID_TARGET_UPDATE_Client_RaidTargetIndex as I;
            let update = match m.target_index {
                I::RequestIcons => None,
                I::Unknown0 { target } => Some((0, target)),
                I::Unknown1 { target } => Some((1, target)),
                I::Unknown2 { target } => Some((2, target)),
                I::Unknown3 { target } => Some((3, target)),
                I::Unknown4 { target } => Some((4, target)),
                I::Unknown5 { target } => Some((5, target)),
                I::Unknown6 { target } => Some((6, target)),
                I::Unknown7 { target } => Some((7, target)),
                I::Unknown8 { target } => Some((8, target)),
            };
            groups::target_icon(&realm, &me, update)
        }
        C::MSG_RANDOM_ROLL(m) => groups::roll(&realm, &me, m.minimum, m.maximum),
        C::MSG_MINIMAP_PING(m) => groups::ping(&realm, &me, m.position_x, m.position_y),
        C::CMSG_REQUEST_PARTY_MEMBER_STATS(m) => groups::request_stats(&realm, &me, m.guid),
        C::CMSG_GUILD_CREATE(m) => guilds::create(&realm, &me, &m.guild_name),
        C::CMSG_GUILD_INVITE(m) => guilds::invite(&realm, &me, &m.invited_player),
        C::CMSG_GUILD_ACCEPT => guilds::accept(&realm, &me),
        C::CMSG_GUILD_DECLINE => guilds::decline(&realm, &me),
        C::CMSG_GUILD_INFO => guilds::info(&realm, &me),
        C::CMSG_GUILD_ROSTER => guilds::roster(&realm, &me),
        C::CMSG_GUILD_QUERY(m) => guilds::query(&realm, &me, m.guild_id),
        C::CMSG_GUILD_PROMOTE(m) => guilds::promote(&realm, &me, &m.player_name, true),
        C::CMSG_GUILD_DEMOTE(m) => guilds::promote(&realm, &me, &m.player_name, false),
        C::CMSG_GUILD_LEAVE => guilds::leave(&realm, &me),
        C::CMSG_GUILD_REMOVE(m) => guilds::remove(&realm, &me, &m.player_name),
        C::CMSG_GUILD_DISBAND => guilds::disband(&realm, &me),
        C::CMSG_GUILD_LEADER(m) => guilds::set_leader(&realm, &me, &m.new_guild_leader_name),
        C::CMSG_GUILD_MOTD(m) => guilds::set_motd(&realm, &me, &m.message_of_the_day),
        C::CMSG_GUILD_INFO_TEXT(m) => guilds::set_info(&realm, &me, &m.guild_info),
        C::CMSG_GUILD_SET_PUBLIC_NOTE(m) => {
            guilds::set_note(&realm, &me, &m.player_name, &m.note, false)
        }
        C::CMSG_GUILD_SET_OFFICER_NOTE(m) => {
            guilds::set_note(&realm, &me, &m.player_name, &m.note, true)
        }
        C::CMSG_GUILD_RANK(m) => guilds::edit_rank(
            &realm,
            &me,
            guilds::RankEdit::Set {
                rank: m.rank_id,
                rights: m.rights,
                name: m.rank_name,
            },
        ),
        C::CMSG_GUILD_ADD_RANK(m) => {
            guilds::edit_rank(&realm, &me, guilds::RankEdit::Add(m.rank_name))
        }
        C::CMSG_GUILD_DEL_RANK => guilds::edit_rank(&realm, &me, guilds::RankEdit::DeleteLowest),
        C::MSG_SAVE_GUILD_EMBLEM(m) => guilds::save_emblem(map, g, &m),
        // At a tabard designer within reach alone (mangos's
        // `HandleTabardVendorActivateOpcode`).
        C::MSG_TABARDVENDOR_ACTIVATE(m) => {
            let npc = crate::game::map::npc::NPC_FLAG_TABARDDESIGNER;
            if crate::game::map::npc::npc_entry(map, g, m.guid, npc).is_some() {
                me.outbound
                    .send(&MSG_TABARDVENDOR_ACTIVATE { guid: m.guid });
            }
        }
        // A charter's list and sale, from a guild master within reach
        // (mangos's `GetNPCIfCanInteractWith`, Thistle Tea's
        // `petitioner?`).
        C::CMSG_PETITION_SHOWLIST(m) => {
            if petitioner(map, g, m.guid) {
                guilds::petition_showlist(map, g, m.guid);
            }
        }
        C::CMSG_PETITION_BUY(m) => {
            if petitioner(map, g, m.npc) {
                guilds::petition_buy(map, g, m.npc, &m.name);
            }
        }
        C::CMSG_PETITION_SHOW_SIGNATURES(m) => guilds::petition_signatures(&realm, &me, m.item),
        C::CMSG_PETITION_QUERY(m) => guilds::petition_query(&realm, &me, m.petition),
        C::CMSG_OFFER_PETITION(m) => guilds::petition_offer(&realm, &me, m.petition, m.target),
        C::CMSG_PETITION_SIGN(m) => guilds::petition_sign(&realm, &me, m.petition),
        C::MSG_PETITION_DECLINE(m) => guilds::petition_decline(&realm, &me, m.petition),
        C::MSG_PETITION_RENAME(m) => guilds::petition_rename(&realm, &me, m.petition, &m.new_name),
        C::CMSG_TURN_IN_PETITION(m) => guilds::petition_turn_in(map, g, m.petition),
        C::CMSG_GET_MAIL_LIST(_) => mail::list(map, g),
        C::CMSG_SEND_MAIL(m) => mail::send(map, g, m),
        C::CMSG_MAIL_TAKE_MONEY(m) => mail::take_money(map, g, m.mail_id),
        C::CMSG_MAIL_TAKE_ITEM(m) => mail::take_item(map, g, m.mail_id),
        C::CMSG_MAIL_MARK_AS_READ(m) => mail::mark_read(map, g, m.mail_id),
        C::CMSG_MAIL_RETURN_TO_SENDER(m) => mail::return_to_sender(map, g, m.mail_id),
        C::CMSG_MAIL_DELETE(m) => mail::delete(map, g, m.mail_id),
        C::CMSG_MAIL_CREATE_TEXT_ITEM(m) => mail::make_letter(map, g, m.mail_id),
        C::CMSG_ITEM_TEXT_QUERY(m) => mail::text_query(map, g, m.item_text_id),
        C::MSG_QUERY_NEXT_MAIL_TIME => mail::next_mail_time(map, g),
        C::CMSG_AUCTION_SELL_ITEM(m) => auction::sell(map, g, &m),
        C::CMSG_INITIATE_TRADE(m) => trade::initiate(map, g, m.guid),
        C::CMSG_BEGIN_TRADE => trade::begin(map, g),
        C::CMSG_BUSY_TRADE => trade::cancel(map, g, SMSG_TRADE_STATUS_TradeStatus::Busy),
        C::CMSG_IGNORE_TRADE => trade::cancel(map, g, SMSG_TRADE_STATUS_TradeStatus::IgnoreYou),
        C::CMSG_CANCEL_TRADE => trade::cancel(map, g, SMSG_TRADE_STATUS_TradeStatus::TradeCanceled),
        C::CMSG_ACCEPT_TRADE(_) => trade::accept(map, g),
        C::CMSG_UNACCEPT_TRADE => trade::unaccept(map, g),
        C::CMSG_SET_TRADE_ITEM(m) => trade::set_item(map, g, m.trade_slot, m.bag, m.slot),
        C::CMSG_CLEAR_TRADE_ITEM(m) => trade::clear_item(map, g, m.trade_slot),
        C::CMSG_SET_TRADE_GOLD(m) => trade::set_money(map, g, m.gold.as_int()),
        C::CMSG_DUEL_ACCEPTED(_) => duel::accept(map, g),
        C::CMSG_DUEL_CANCELLED(_) => duel::cancel(map, g),
        C::CMSG_AUCTION_PLACE_BID(m) => auction::bid(map, g, &m),
        C::CMSG_AUCTION_REMOVE_ITEM(m) => auction::cancel(map, g, &m),
        C::CMSG_AUCTION_LIST_ITEMS(m) => auction::list(map, g, &m),
        C::CMSG_AUCTION_LIST_OWNER_ITEMS(m) => auction::list_mine(map, g, m.auctioneer, false, &[]),
        C::CMSG_AUCTION_LIST_BIDDER_ITEMS(m) => {
            auction::list_mine(map, g, m.auctioneer, true, &m.outbid_item_ids)
        }
        other => return Some(other),
    }
    None
}

/// A player in the world: their lists, their group, and those who have
/// them as a friend.
pub fn logged_in(map: &mut Map, g: Guid) {
    crate::game::map::pvp::logged_in(map, g);
    friends::logged_in(map, g);
    guilds::logged_in(map, g);
    if let Some(p) = map.players.get(&g) {
        crate::game::weather::tell(&map.realm, &p.online, p.zone);
    }
    groups::logged_in(&map.realm, g);
}

/// A player has gone from the world, however they went (a logout, their
/// client gone, let go with a map, reaped, or dropped by a panic): they
/// leave the social world ([`logged_out`]), then the online list. Every
/// way out comes here. Before, those let go or lost with a map left the
/// online list alone, and stayed in their channels and battleground
/// queues, online to their friends and group.
pub fn went_offline(realm: &crate::game::Realm, g: Guid) {
    logged_out(realm, g);
    realm.registry.go_offline(g);
}

/// A player going offline leaves their channels and battleground queues;
/// their friends, guild and group hear of it.
fn logged_out(realm: &crate::game::Realm, g: Guid) {
    if let Some(me) = realm.registry.online(g) {
        channels::leave_all(realm, &me);
    }
    friends::logged_out(realm, g);
    crate::game::battlegrounds::logged_out(realm, g);
    // Out of a meeting stone's queue, as mangos's `LogoutPlayer` takes them.
    crate::game::map::meeting_stones::logged_out(realm, g);
    guilds::logged_out(realm, g);
    groups::logged_out(realm, g);
}

/// The stats of `g`'s pet out, or of the unit they charm (mangos's
/// `GetCharm`).
fn pet_stats_of(map: &Map, g: Guid) -> Option<groups::PetStats> {
    let pet = crate::game::map::pets::pet_of(map, g)?;
    let c = map.creature(pet)?;
    let f = &c.fields;
    let name = c
        .pet
        .as_ref()
        .map(|s| s.name.as_str())
        .filter(|n| !n.is_empty())
        .or_else(|| {
            map.realm
                .data
                .creature_templates
                .get(&c.entry)
                .map(|t| t.name.as_str())
        })
        .unwrap_or_default();
    let power_type = f.get_byte(UNIT_FIELD_BYTES_0, 3);
    let mut auras = [0u32; 48];
    for (i, a) in auras.iter_mut().enumerate() {
        *a = f.get_u32(UNIT_FIELD_AURA + i as u16);
    }
    Some(groups::PetStats {
        guid: pet,
        name: Arc::from(name),
        model: f.get_u32(UNIT_FIELD_DISPLAYID),
        health: f.get_u32(UNIT_FIELD_HEALTH),
        max_health: f.get_u32(UNIT_FIELD_MAXHEALTH),
        power_type,
        power: f.get_u32(UNIT_FIELD_POWER1 + u16::from(power_type)),
        max_power: f.get_u32(UNIT_FIELD_MAXPOWER1 + u16::from(power_type)),
        auras,
    })
}

fn stats_of(map: &Map, p: &crate::game::map::player::Player) -> MemberStats {
    use crate::game::map::death::Death;
    let f = &p.fields;
    let power_type = p.power_type();
    let mut auras = [0u32; 48];
    for (i, a) in auras.iter_mut().enumerate() {
        *a = f.get_u32(UNIT_FIELD_AURA + i as u16);
    }
    // Away or busy shows on the party's frames too (Thistle Tea's
    // `MemberStats.from_character`).
    let away = if f.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_AFK) {
        groups::STATUS_AFK
    } else if f.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_DND) {
        groups::STATUS_DND
    } else {
        0
    };
    // Flagged for PvP, or free-for-all (mangos's `GetGroupMemberStatus`).
    let pvp = if f.has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP) {
        groups::STATUS_PVP
    } else {
        0
    } | if f.has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_FFA_PVP) {
        groups::STATUS_PVP_FFA
    } else {
        0
    };
    MemberStats {
        status: groups::STATUS_ONLINE
            | away
            | pvp
            | match p.death {
                Death::Alive => 0,
                Death::Dead { .. } => groups::STATUS_DEAD,
                _ => groups::STATUS_GHOST,
            },
        health: f.get_u32(UNIT_FIELD_HEALTH),
        max_health: f.get_u32(UNIT_FIELD_MAXHEALTH),
        power_type,
        power: f.get_u32(UNIT_FIELD_POWER1 + u16::from(power_type)),
        max_power: f.get_u32(UNIT_FIELD_MAXPOWER1 + u16::from(power_type)),
        level: f.get_u32(UNIT_FIELD_LEVEL),
        zone: p.zone,
        x: p.position().x,
        y: p.position().y,
        auras,
        pet: pet_stats_of(map, p.guid),
    }
}

/// Once a second: each grouped player's changed stats, to the members
/// who can't see them.
pub fn push_member_stats(map: &mut Map) {
    if map.now < map.member_stats_at {
        return;
    }
    map.member_stats_at = map.now + STATS_EVERY;
    let realm = map.realm.clone();
    // The realm's groups lock is taken twice, briefly: for who is grouped,
    // and to keep the stats and who saw them. Their stats are read out of
    // the map, and the frames made and sent, without it, where it was held
    // across all of it and every map's taps, group spells, party chat and
    // copies waited (review 5's social 13).
    let grouped: Vec<(Guid, Vec<Guid>)> = {
        let groups = realm.social.groups.lock();
        map.players
            .keys()
            .filter_map(|&g| {
                let others = groups.others(g);
                (!others.is_empty()).then_some((g, others))
            })
            .collect()
    };
    if grouped.is_empty() {
        return;
    }
    let read: Vec<(Guid, Vec<Guid>, MemberStats, Vec<Guid>)> = grouped
        .into_iter()
        .filter_map(|(g, others)| {
            let p = map.players.get(&g)?;
            let seen = others
                .iter()
                .copied()
                .filter(|o| p.observers.contains(o))
                .collect();
            Some((g, others, stats_of(map, p), seen))
        })
        .collect();
    type Kept = (Option<MemberStats>, Vec<Guid>);
    let kept: Vec<Kept> = {
        let mut groups = realm.social.groups.lock();
        read.iter()
            .map(|(g, _, now, seen)| {
                let before = groups.stats.insert(*g, now.clone());
                let was_seen = groups.seen_by.insert(*g, seen.clone()).unwrap_or_default();
                (before, was_seen)
            })
            .collect()
    };
    for ((g, others, now, seen), (before, was_seen)) in read.iter().zip(kept) {
        let mask = before.as_ref().map_or(now.all(), |b| now.changed(b));
        let frame = (mask != 0).then(|| now.packet(*g, mask, before.is_none()));
        let mut all = None;
        for o in others.iter().filter(|o| !seen.contains(o)) {
            if was_seen.contains(o) {
                // Just out of sight: everything.
                let all = all.get_or_insert_with(|| now.packet(*g, now.all(), false));
                crate::game::social::send_frame(&realm, *o, all);
            } else if let Some(frame) = &frame {
                crate::game::social::send_frame(&realm, *o, frame);
            }
        }
    }
}
