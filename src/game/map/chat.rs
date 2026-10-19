//! Chat heard nearby (say, yell, emote) and emotes. A line starting with
//! `.` is a GM command. A player speaks only the languages they know
//! (Thistle Tea's `Chat.Language`); the client garbles what its own player
//! doesn't. Whispers and emotes go out in the universal language, and a
//! curse of tongues speaks for its victim.

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
/// The spells that teach each language, should their templates be
/// missing.
const LANGUAGE_SPELLS: &[(u32, u32)] = &[
    (668, 7),
    (669, 1),
    (670, 3),
    (671, 2),
    (672, 6),
    (813, 10),
    (814, 11),
    (815, 8),
    (816, 9),
    (817, 12),
    (7340, 13),
    (7341, 14),
    (17737, 33),
];

const EFFECT_LANGUAGE: u32 = 39;
const ADDON: u32 = 0xFFFF_FFFF;

/// Whether `g` speaks `language`: a spell that teaches it.
pub fn knows_language(map: &Map, g: Guid, language: u32) -> bool {
    let Some(p) = map.players.get(&g) else {
        return false;
    };
    if p.gm_level > 0 {
        return true;
    }
    let data = &map.realm.data;
    p.spells.iter().any(|s| match data.spells.get(*s) {
        Some(t) => t
            .effects
            .iter()
            .any(|e| e.effect == EFFECT_LANGUAGE && e.misc_value as u32 == language),
        None => LANGUAGE_SPELLS.contains(&(*s, language)),
    })
}

/// The language a line goes out in, or `None` if the speaker can't use
/// the one they chose.
pub fn resolve_language(
    map: &Map,
    g: Guid,
    kind: &CMSG_MESSAGECHAT_ChatType,
    language: Language,
) -> Option<Language> {
    use CMSG_MESSAGECHAT_ChatType as T;
    let id = language.as_int();
    // Addon messages keep their marker on the chats addons use.
    if id == ADDON {
        return matches!(
            kind,
            T::Party
                | T::Raid
                | T::Guild
                | T::Officer
                | T::Channel { .. }
                | T::RaidLeader
                | T::RaidWarning
        )
        .then_some(language);
    }
    if matches!(kind, T::Afk | T::Dnd) {
        return Some(Language::Universal);
    }
    // Without the spells' data (a bare test world) every language is known.
    let checked = !map.realm.data.spells.spells.is_empty();
    let speakable = if id == 0 {
        // Only GMs speak the universal language; an emote is in it anyway.
        matches!(kind, T::Emote) || map.players.get(&g).is_some_and(|p| p.gm_level > 0)
    } else {
        knows_language(map, g, id)
    };
    if checked && !speakable {
        return None;
    }
    if matches!(kind, T::Whisper { .. } | T::Emote) {
        return Some(Language::Universal);
    }
    // A curse of tongues: the victim speaks the curse's language.
    let cursed = crate::game::map::auras::auras(map, g).and_then(|a| {
        a.effects()
            .find(|(_, x)| x.kind == crate::game::map::auras::AURA_MOD_LANGUAGE)
            .map(|(_, x)| x.misc as u32)
    });
    Some(
        cursed
            .and_then(|c| Language::try_from(c).ok())
            .unwrap_or(language),
    )
}

pub fn message(map: &mut Map, guid: Guid, mut m: CMSG_MESSAGECHAT) -> Option<CMSG_MESSAGECHAT> {
    if m.message.len() > MAX_MESSAGE {
        return None;
    }
    // An addon's line is its payload, never a command (Thistle Tea's
    // `Chat.handle`): an addon's prefix may start with a dot.
    let gm = m.language.as_int() != ADDON && map.players.get(&guid).is_some_and(|p| p.gm_level > 0);
    let gm_line = gm && m.message.starts_with('.');
    if !gm_line {
        let Some(language) = resolve_language(map, guid, &m.chat_type, m.language) else {
            // A language not learned (Thistle Tea's `Language.resolve`).
            if m.language.as_int() != ADDON
                && let Some(p) = map.players.get(&guid)
            {
                p.outbound
                    .send(&wow_world_messages::vanilla::SMSG_NOTIFICATION {
                        notification: "You have not learned that language.".into(),
                    });
            }
            return None;
        };
        m.language = language;
    }
    // A player's "." line is chat; a GM's, a command.
    if gm_line && m.message.len() > 1 {
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
    for g in map.players_near(pos, radius) {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send_frame(frame.clone());
        }
    }
    None
}

/// `UNIT_FIELD_FLAGS`: animations are held (a stun's, a cinematic's).
const UNIT_FLAG_PREVENT_ANIM: u32 = 0x2000_0000;
/// Text emotes whose animation is a posture the client asks for itself
/// (Thistle Tea's `@text_only_animations`): none, sleep, sit, kneel.
const TEXT_ONLY_ANIMATIONS: [u32; 4] = [0, 12, 13, 68];
/// The emotes a client may play itself (vmangos's `HandleEmoteOpcode`):
/// none, and a wave.
const CLIENT_EMOTES: [u32; 2] = [0, 3];

/// Whether `g` may emote: alive, and not held from moving its body.
fn may_emote(map: &Map, g: Guid) -> bool {
    // Feign Death's flag is the same bit, and an emote ends it (mangos's
    // feign death sets no PREVENT_ANIM, Thistle Tea's emote interrupts it).
    map.players.get(&g).is_some_and(|p| {
        p.death == crate::game::map::death::Death::Alive
            && (!p.fields.has_flag(
                crate::game::fields::UNIT_FIELD_FLAGS,
                UNIT_FLAG_PREVENT_ANIM,
            ) || p.auras.has(crate::game::map::auras::AURA_FEIGN_DEATH))
    })
}

/// What an animation ends (Thistle Tea's `Emote.interrupt`): a channel
/// its interrupt flags say an animation ends, and such auras.
fn animation_interrupts(map: &mut Map, g: Guid) {
    const ANIMATION: u32 = crate::game::map::auras::INTERRUPT_ENTER_COMBAT;
    let channel = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .filter(|c| c.channel_until.is_some())
        .map(|c| c.spell);
    let breaks = channel
        .and_then(|s| map.realm.data.spells.get(s))
        .is_some_and(|s| s.channel_interrupt_flags & ANIMATION != 0);
    if breaks {
        crate::game::map::spells::interrupt(map, g);
    }
    crate::game::map::auras::interrupt(map, g, ANIMATION);
}

/// An emote's animation: a state the player stays in, or played once to
/// everyone who sees them; and the auras an animation ends.
pub fn play_animation(map: &mut Map, g: Guid, emote: u32) {
    animation_interrupts(map, g);
    if emote == 0 || map.realm.data.emote_states.contains(&emote) {
        if let Some(p) = map.players.get_mut(&g) {
            p.fields
                .set_u32(crate::game::fields::UNIT_NPC_EMOTESTATE, emote);
        }
        if emote != 0 {
            return;
        }
    }
    let Ok(id) = wow_world_messages::vanilla::Emote::try_from(emote) else {
        return;
    };
    if let Some(frame) = encode(&SMSG_EMOTE { emote: id, guid: g }) {
        map.send_to_observers(g, &frame, true);
    }
}

/// Whether a posture is sitting, on the ground or a chair (mangos's
/// `IsSitState`).
pub fn sitting(posture: u8) -> bool {
    matches!(posture, 1 | 2 | 4..=6)
}

/// An aura that standing ends (food, drink) seats the living player it
/// goes on, unless they sit already (on a chair too), and tells their
/// client: mangos's `_AddSpellAuraHolder`, `SetStandState(SIT)`.
pub fn sit_for_aura(map: &mut Map, g: Guid) {
    use crate::game::map::player::STAND_STATE_SIT;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.death != crate::game::map::death::Death::Alive || sitting(p.stand_state()) {
        return;
    }
    p.set_stand_state(STAND_STATE_SIT);
    p.outbound
        .send(&wow_world_messages::vanilla::SMSG_STANDSTATE_UPDATE {
            state: wow_world_messages::vanilla::UnitStandState::Sit,
        });
}

/// `CMSG_STANDSTATECHANGE`: the postures a client may take (stand, sit,
/// sleep, kneel), as Thistle Tea's `Emote.stand`; standing or lying down
/// to sleep ends what needs one seated (food, drink), as mangos's
/// `SetStandState` has it (`IsSeatedState`: neither standing nor
/// sleeping).
pub fn stand_state(map: &mut Map, g: Guid, state: wow_world_messages::vanilla::UnitStandState) {
    let posture = state.as_int();
    if !matches!(posture, 0 | 1 | 3 | 8) || !may_emote(map, g) {
        return;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.logout_at.is_some() {
        return;
    }
    p.set_stand_state(posture);
    p.outbound
        .send(&wow_world_messages::vanilla::SMSG_STANDSTATE_UPDATE { state });
    animation_interrupts(map, g);
    if matches!(posture, 0 | 3) {
        crate::game::map::auras::interrupt(map, g, crate::game::map::auras::INTERRUPT_NOT_SEATED);
    }
}

/// `CMSG_TEXT_EMOTE`: everyone near hears it, with the target's name, and
/// the player plays its animation.
pub fn text_emote(map: &mut Map, guid: Guid, m: CMSG_TEXT_EMOTE) {
    if !may_emote(map, guid) {
        return;
    }
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
    for g in map.players_near(pos, TEXT_EMOTE_YARDS) {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send_frame(frame.clone());
        }
    }
    let animation = map
        .realm
        .data
        .text_emote_anims
        .get(&m.text_emote.as_int())
        .copied()
        .filter(|a| !TEXT_ONLY_ANIMATIONS.contains(a));
    if let Some(a) = animation {
        play_animation(map, guid, a);
    }
    if crate::game::guid::is_unit(m.target) {
        crate::game::map::event_ai::receive_emote(map, m.target, guid, m.text_emote.as_int());
    }
}

/// `CMSG_EMOTE`: the two animations a client plays itself, seen by
/// everyone who sees the player; none stops a state.
pub fn emote(map: &mut Map, guid: Guid, m: CMSG_EMOTE) {
    let id = m.emote.as_int();
    if !CLIENT_EMOTES.contains(&id) || !may_emote(map, guid) {
        return;
    }
    play_animation(map, guid, id);
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
