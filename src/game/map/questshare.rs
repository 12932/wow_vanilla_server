//! Sharing quests with a group: a quest pushed to the members near, who
//! see its details from the sharer and may take it; quests every member
//! must accept together (escorts); and swapping two quests in the log.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::map::Map;
use crate::game::map::quests;

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

/// Whether `a` and `b` may share a quest every member must accept: in
/// one party, or in one raid for a raid quest (mangos's
/// `HandleQuestConfirmAccept`: `IsAllowedInRaid`, else `IsInParty`, the
/// same subgroup; Thistle Tea's "enforces ordinary subgroups and allows
/// raid quest confirmations").
fn in_party_for(map: &Map, a: Guid, b: Guid, quest: u32) -> bool {
    let raid_quest = map
        .realm
        .data
        .npc
        .quests
        .get(&quest)
        .is_some_and(|q| q.kind == 62);
    let groups = map.realm.social.groups.lock();
    groups.group_of(a).is_some_and(|gr| {
        let sub = |g: Guid| gr.members.iter().find(|m| m.guid == g).map(|m| m.subgroup);
        sub(b).is_some() && (!gr.raid || raid_quest || sub(a) == sub(b))
    })
}

/// `CMSG_PUSHQUESTTOPARTY`: a shareable quest under way in the sharer's
/// log (mangos's `CanShareQuest`, Thistle Tea's `shareable?`) offered to
/// each member, the sharer told of each first (mangos's
/// `HandlePushQuestToParty`) and why one isn't offered it: too far, on it
/// or done with it, finished, unable to take it, with a full log, or with
/// an offer already.
pub fn push(map: &mut Map, g: Guid, quest: u32) {
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return;
    };
    let Some(me) = map.players.get(&g) else {
        return;
    };
    let current = me
        .journal
        .slot_of(quest)
        .is_some_and(|s| !me.journal.log[s].failed);
    if q.flags & QUEST_FLAGS_SHARABLE == 0 || !current {
        return;
    }
    let at = me.position();
    for m in others(map, g) {
        result(map, g, m, QuestPartyMessage::SharingQuest);
        let Some(p) = map.players.get(&m) else {
            // On another map: too far.
            result(map, g, m, QuestPartyMessage::TooFar);
            continue;
        };
        let done = p
            .journal
            .slot_of(quest)
            .is_some_and(|s| quests::complete(p, q, &p.journal.log[s]));
        let why = if crate::game::map::combat::dist(p.position(), at) > SHARE_YARDS {
            Some(QuestPartyMessage::TooFar)
        } else if done {
            Some(QuestPartyMessage::FinishQuest)
        } else if p.journal.has(quest)
            || (p.journal.rewarded.contains(&quest)
                && q.special_flags & crate::game::data::npc::QUEST_SPECIAL_REPEATABLE == 0)
        {
            Some(QuestPartyMessage::HaveQuest)
        } else if !quests::can_take(p, q, &data) || !quests::condition_ok(map, m, g, quest) {
            Some(QuestPartyMessage::CantTakeQuest)
        } else if p.journal.full() {
            Some(QuestPartyMessage::LogFull)
        } else if p.misc.quest_shared.is_some() {
            Some(QuestPartyMessage::Busy)
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
    }
}

/// A shared quest taken: still near the sharer and able to take it, or
/// the sharer hears why not and the offer goes (Thistle Tea's
/// `QuestSharing.accept`). The sharer's time left, for a timed quest
/// (mangos's `AddQuest`); none, refused. `asked`: a member asked to join
/// a party-accept quest (`SMSG_QUEST_CONFIRM_ACCEPT`), who may be anywhere
/// they were asked from (mangos's `HandleQuestConfirmAccept` checks no
/// distance); a shared quest must be taken near its sharer.
pub fn admit(map: &mut Map, g: Guid, sharer: Guid, quest: u32, asked: bool) -> Option<i64> {
    let data = map.realm.data.clone();
    let q = data.npc.quests.get(&quest)?;
    let near = match (map.players.get(&g), map.players.get(&sharer)) {
        (Some(p), Some(s)) => {
            asked || crate::game::map::combat::dist(p.position(), s.position()) <= SHARE_YARDS
        }
        _ => false,
    };
    // The offer holds while the sharer has the quest under way and the two
    // are still in one group (Thistle Tea's `QuestSharing.refresh` and
    // `cancel`); gone, it is gone, with nothing to say.
    let still = map.players.get(&sharer).is_some_and(|s| {
        s.journal
            .slot_of(quest)
            .is_some_and(|i| !s.journal.log[i].failed)
    }) && others(map, sharer).contains(&g);
    if !still {
        if let Some(p) = map.players.get_mut(&g) {
            p.misc.quest_shared = None;
        }
        return None;
    }
    let why = if !near {
        Some(QuestPartyMessage::TooFar)
    } else if !crate::game::events::quest_offered(&map.realm, quest) {
        // Its event over since it was offered.
        Some(QuestPartyMessage::CantTakeQuest)
    } else if !map
        .players
        .get(&g)
        .is_some_and(|p| quests::can_take(p, q, &data))
        || !quests::condition_ok(map, g, sharer, quest)
    {
        Some(QuestPartyMessage::CantTakeQuest)
    } else {
        None
    };
    if let Some(why) = why {
        if let Some(p) = map.players.get_mut(&g) {
            p.misc.quest_shared = None;
        }
        result(map, sharer, g, why);
        return None;
    }
    let expires = map
        .players
        .get(&sharer)
        .and_then(|s| s.journal.slot_of(quest).map(|i| s.journal.log[i].expires))
        .unwrap_or(0);
    Some(expires)
}

/// A shared quest admitted but not taken after all (no room for its
/// items): the offer goes, and the sharer hears it can't be taken.
pub fn refused(map: &mut Map, g: Guid, sharer: Guid) {
    if let Some(p) = map.players.get_mut(&g) {
        p.misc.quest_shared = None;
    }
    result(map, sharer, g, QuestPartyMessage::CantTakeQuest);
}

/// `MSG_QUEST_PUSH_RESULT` from a member: told to the one sharing with
/// them, and only what a member may answer; that they took it the server
/// says (mangos's `HandleQuestPushResult`, Thistle Tea's
/// `QuestSharing.result`).
fn answer(map: &mut Map, g: Guid, sharer: Guid, message: QuestPartyMessage) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if !p.misc.quest_shared.is_some_and(|(by, _)| by == sharer)
        || matches!(
            message,
            QuestPartyMessage::SharingQuest | QuestPartyMessage::AcceptQuest
        )
    {
        return;
    }
    p.misc.quest_shared = None;
    result(map, sharer, g, message);
}

/// A member took a shared quest: the sharer hears; one every member must
/// accept asks the rest.
pub fn accepted(map: &mut Map, g: Guid, giver: Guid, quest: u32) {
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&g)
        && p.misc
            .quest_shared
            .is_some_and(|(by, q)| by == giver && q == quest)
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
        if !in_party_for(map, g, m, quest) {
            continue;
        }
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
    if !in_party_for(map, g, by, quest) {
        return;
    }
    quests::accept_asked(map, g, by, quest);
}

/// `CMSG_QUESTLOG_SWAP_QUEST`: two of the log's slots trade places.
fn swap(map: &mut Map, g: Guid, a: u8, b: u8) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    p.journal.swap(usize::from(a), usize::from(b));
    quests::refresh_log(p, &data);
}
