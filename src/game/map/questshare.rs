//! Sharing quests with a group: a quest pushed to the members near, who
//! see its details from the sharer and may take it; quests every member
//! must accept together (escorts); and swapping two quests in the log.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::map::Map;
use crate::game::map::quests::{self, QUEST_LOG_SIZE};

/// How near the sharer a member must be.
pub const SHARE_YARDS: f32 = 10.0;
/// `QuestFlags`: every member of the group must accept (escorts).
pub const QUEST_FLAGS_PARTY_ACCEPT: u32 = 0x2;
/// `QuestFlags`: may be shared.
pub const QUEST_FLAGS_SHARABLE: u32 = 0x8;

pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_PUSHQUESTTOPARTY(m) => push(map, g, m.quest_id),
        C::MSG_QUEST_PUSH_RESULT(m) => answer(map, g, m.guid, m.message),
        C::CMSG_QUEST_CONFIRM_ACCEPT(m) => confirm(map, g, m.quest_id),
        C::CMSG_QUESTLOG_SWAP_QUEST(m) => swap(map, g, m.slot1, m.slot2),
        other => return Some(other),
    }
    None
}

fn result(map: &Map, to: Guid, about: Guid, message: QuestPartyMessage) {
    if let Some(p) = map.players.get(&to) {
        p.outbound.send(&MSG_QUEST_PUSH_RESULT {
            guid: about,
            message,
        });
    }
}

/// The members of `g`'s group other than `g`.
fn others(map: &Map, g: Guid) -> Vec<Guid> {
    let groups = map.realm.social.groups.lock();
    groups
        .group_of(g)
        .map(|gr| {
            gr.members
                .iter()
                .map(|m| m.guid)
                .filter(|m| *m != g)
                .collect()
        })
        .unwrap_or_default()
}

/// `CMSG_PUSHQUESTTOPARTY`: a quest in the sharer's log offered to each
/// member near who could take it.
pub fn push(map: &mut Map, g: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let Some(me) = map.players.get(&g) else {
        return;
    };
    if !me.journal.has(quest) {
        return;
    }
    let at = me.position();
    for m in others(map, g) {
        let Some(p) = map.players.get(&m) else {
            // On another map: too far.
            result(map, g, m, QuestPartyMessage::TooFar);
            continue;
        };
        let why = if crate::game::map::combat::dist(p.position(), at) > SHARE_YARDS {
            Some(QuestPartyMessage::TooFar)
        } else if p.journal.has(quest) {
            Some(QuestPartyMessage::HaveQuest)
        } else if p.journal.rewarded.contains(&quest)
            && q.special_flags & crate::game::data::npc::QUEST_SPECIAL_REPEATABLE == 0
        {
            Some(QuestPartyMessage::FinishQuest)
        } else if p.journal.log.len() >= QUEST_LOG_SIZE {
            Some(QuestPartyMessage::LogFull)
        } else if p.misc.quest_shared.is_some() {
            Some(QuestPartyMessage::Busy)
        } else if !quests::can_take(p, q, &data) {
            Some(QuestPartyMessage::CantTakeQuest)
        } else {
            None
        };
        if let Some(why) = why {
            result(map, g, m, why);
            continue;
        }
        if let Some(p) = map.players.get_mut(&m) {
            p.misc.quest_shared = Some((g, quest));
        }
        quests::details(map, m, g, quest);
        result(map, g, m, QuestPartyMessage::SharingQuest);
    }
}

/// `MSG_QUEST_PUSH_RESULT` from a member: told to the sharer.
fn answer(map: &mut Map, g: Guid, sharer: Guid, message: QuestPartyMessage) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.misc.quest_shared.is_some_and(|(by, _)| by == sharer) {
        p.misc.quest_shared = None;
    }
    result(map, sharer, g, message);
}

/// A member took a shared quest: the sharer hears; one every member must
/// accept asks the rest.
pub fn accepted(map: &mut Map, g: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&g)
        && p.misc.quest_shared.is_some_and(|(by, q)| by == giver && q == quest)
    {
        p.misc.quest_shared = None;
        result(map, giver, g, QuestPartyMessage::AcceptQuest);
    }
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    if q.flags & QUEST_FLAGS_PARTY_ACCEPT == 0 || crate::game::guid::is_player(giver) {
        return;
    }
    let Some(at) = map.players.get(&g).map(|p| p.position()) else {
        return;
    };
    for m in others(map, g) {
        let Some(p) = map.players.get_mut(&m) else {
            continue;
        };
        if crate::game::map::combat::dist(p.position(), at) > SHARE_YARDS * 3.0
            || p.journal.has(quest)
            || !quests::can_take(p, q, &data)
        {
            continue;
        }
        p.misc.quest_shared = Some((g, quest));
        p.outbound.send(&SMSG_QUEST_CONFIRM_ACCEPT {
            quest_id: quest,
            quest_title: q.title.clone(),
            guid: g,
        });
    }
}

/// Whether `g` may take `quest` from the player `giver`: it was shared.
pub fn shared_by(map: &Map, g: Guid, giver: Guid, quest: u32) -> bool {
    map.players
        .get(&g)
        .is_some_and(|p| p.misc.quest_shared == Some((giver, quest)))
}

/// `CMSG_QUEST_CONFIRM_ACCEPT`: a member joins the escort.
fn confirm(map: &mut Map, g: Guid, quest: u32) {
    let Some((by, _)) = map
        .players
        .get(&g)
        .and_then(|p| p.misc.quest_shared)
        .filter(|(_, q)| *q == quest)
    else {
        return;
    };
    quests::accept(map, g, by, quest);
}

/// `CMSG_QUESTLOG_SWAP_QUEST`: two of the log's slots trade places.
fn swap(map: &mut Map, g: Guid, a: u8, b: u8) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let (a, b) = (usize::from(a), usize::from(b));
    if a >= p.journal.log.len() || b >= p.journal.log.len() || a == b {
        return;
    }
    p.journal.log.swap(a, b);
    quests::refresh_log(p, &data);
}
