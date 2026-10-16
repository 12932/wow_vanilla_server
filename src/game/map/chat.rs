//! Chat heard nearby (say, yell, emote) and emotes. A line starting with
//! `.` is a GM command.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    CMSG_EMOTE, CMSG_MESSAGECHAT, CMSG_MESSAGECHAT_ChatType, CMSG_TEXT_EMOTE, Language,
    PlayerChatTag, SMSG_EMOTE, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType, SMSG_TEXT_EMOTE,
};

use crate::game::conn::{Outbound, encode};
use crate::game::map::{Map, gm};

/// How far a say, an emote and a yell carry.
pub const SAY_YARDS: f32 = 25.0;
pub const YELL_YARDS: f32 = 300.0;
pub const TEXT_EMOTE_YARDS: f32 = 25.0;
/// The longest line a client may send.
pub const MAX_MESSAGE: usize = 255;

pub fn send_system(out: &Outbound, text: &str) {
    for line in text.lines() {
        out.send(&SMSG_MESSAGECHAT {
            chat_type: SMSG_MESSAGECHAT_ChatType::System {
                sender2: Guid::zero(),
            },
            language: Language::Universal,
            message: line.to_owned(),
            tag: PlayerChatTag::None,
        });
    }
}

/// The tag a player's lines carry: GM, AFK or DND.
pub fn tag_of(map: &Map, guid: Guid) -> PlayerChatTag {
    let Some(p) = map.players.get(&guid) else {
        return PlayerChatTag::None;
    };
    use crate::game::fields::PLAYER_FLAGS;
    use crate::game::map::player::{PLAYER_FLAGS_AFK, PLAYER_FLAGS_DND, PLAYER_FLAGS_GM};
    if p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_GM) {
        PlayerChatTag::Gm
    } else if p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_AFK) {
        PlayerChatTag::Afk
    } else if p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_DND) {
        PlayerChatTag::Dnd
    } else {
        PlayerChatTag::None
    }
}

/// `CMSG_MESSAGECHAT`. Returns the message if it is a kind this module
/// doesn't serve (party, whisper, channel, ...).
pub fn message(map: &mut Map, guid: Guid, m: CMSG_MESSAGECHAT) -> Option<CMSG_MESSAGECHAT> {
    if m.message.len() > MAX_MESSAGE {
        return None;
    }
    // A player's "." line is chat; a GM's, a command.
    let gm = map.players.get(&guid).is_some_and(|p| p.gm_level > 0);
    if gm && m.message.starts_with('.') && m.message.len() > 1 {
        gm::command(map, guid, &m.message[1..]);
        return None;
    }
    let (radius, chat_type) = match &m.chat_type {
        CMSG_MESSAGECHAT_ChatType::Say => (
            SAY_YARDS,
            SMSG_MESSAGECHAT_ChatType::Say {
                chat_credit: guid,
                speech_bubble_credit: guid,
            },
        ),
        CMSG_MESSAGECHAT_ChatType::Yell => (
            YELL_YARDS,
            SMSG_MESSAGECHAT_ChatType::Yell {
                chat_credit: guid,
                speech_bubble_credit: guid,
            },
        ),
        CMSG_MESSAGECHAT_ChatType::Emote => (
            SAY_YARDS,
            SMSG_MESSAGECHAT_ChatType::Emote { sender2: guid },
        ),
        _ => return Some(m),
    };
    if m.message.trim().is_empty() {
        return None;
    }
    let tag = tag_of(map, guid);
    let pos = map.players.get(&guid).map(|p| p.position())?;
    let frame = encode(&SMSG_MESSAGECHAT {
        chat_type,
        language: m.language,
        message: m.message,
        tag,
    })?;
    for g in map.players_near(pos.x, pos.y, radius) {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send_frame(frame.clone());
        }
    }
    None
}

/// `CMSG_TEXT_EMOTE`: everyone near hears it, with the target's name.
pub fn text_emote(map: &mut Map, guid: Guid, m: CMSG_TEXT_EMOTE) {
    let Some(pos) = map.players.get(&guid).map(|p| p.position()) else {
        return;
    };
    let name = target_name(map, m.target);
    let Some(frame) = encode(&SMSG_TEXT_EMOTE {
        guid,
        text_emote: m.text_emote,
        emote: m.emote,
        name,
    }) else {
        return;
    };
    for g in map.players_near(pos.x, pos.y, TEXT_EMOTE_YARDS) {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send_frame(frame.clone());
        }
    }
}

/// `CMSG_EMOTE`: an animation, seen by everyone who sees the player.
pub fn emote(map: &mut Map, guid: Guid, m: CMSG_EMOTE) {
    if let Some(frame) = encode(&SMSG_EMOTE {
        emote: m.emote,
        guid,
    }) {
        map.send_to_observers(guid, &frame, true);
    }
}

/// The name of a player or creature, for a text emote.
pub fn target_name(map: &Map, target: Guid) -> String {
    if target.is_zero() {
        return String::new();
    }
    if let Some(p) = map.players.get(&target) {
        return p.name.to_string();
    }
    if let Some(c) = map.creature(target)
        && let Some(t) = map.realm.data.creature_template(c.entry)
    {
        return t.name.clone();
    }
    map.realm
        .registry
        .name(target)
        .map(|n| n.name.to_string())
        .unwrap_or_default()
}

/// mangos's chat flood control: ten lines each within a second of the last
/// mute the speaker for ten seconds.
pub const FLOOD_LINES: u32 = 10;
pub const FLOOD_GAP: std::time::Duration = std::time::Duration::from_secs(1);
pub const FLOOD_MUTE: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a player's line is to be dropped: muted, or flooding now.
pub fn flooded(map: &mut Map, guid: Guid) -> bool {
    let now = map.now;
    let Some(p) = map.players.get_mut(&guid) else {
        return true;
    };
    if p.gm {
        return false;
    }
    if p.muted_until.is_some_and(|t| t > now) {
        send_system(&p.outbound, "You are muted for flooding the chat.");
        return true;
    }
    let close = p
        .chat_last
        .is_some_and(|t| now.duration_since(t) < FLOOD_GAP);
    p.chat_burst = if close { p.chat_burst + 1 } else { 0 };
    p.chat_last = Some(now);
    if p.chat_burst >= FLOOD_LINES {
        p.chat_burst = 0;
        p.muted_until = Some(now + FLOOD_MUTE);
        send_system(&p.outbound, "You are muted for flooding the chat.");
        return true;
    }
    false
}
